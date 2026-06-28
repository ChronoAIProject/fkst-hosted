#!/usr/bin/env python3
"""Dry-run allowlist migration slicer for code-owned ratchet parents."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from time import time
from typing import Any, Callable

import check_repo_dedup as code_dedup
import check_repo_gh_git_adapter as gh_git_adapter


TARGET_COUNT = 0
DEFAULT_SLICE_SIZE = 3
MAX_SLICE_SIZE = 10
SCHEMA = "fkst.ratchet-slice.v1"
VALID_RATCHETS = ("gh-git-adapter", "saga-handler", "code-dedup")
RATCHET_ALIASES = {
    "891": "gh-git-adapter",
    "892": "saga-handler",
}
DEFAULT_RECONCILE_RATCHETS = ("saga-handler", "code-dedup")
DEFAULT_LABELS = ("fkst-dev:enabled",)
FREE_FORM_PIPELINE_RE = re.compile(r"(?m)^\s*(?:function\s+pipeline\s*\(|pipeline\s*=\s*function\b)")
SAFE_MARKER_VALUE_RE = re.compile(r"^[A-Za-z0-9._/,-]+$")
SAFE_REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
UNRESOLVED_LEDGER_ISSUE = object()
UNKNOWN_LEDGER_HOLD_SECONDS = 15 * 60


@dataclass(frozen=True)
class MigrationSpec:
    parent: str
    ratchet: str
    migration_kind: str
    allowlist_path: str
    title: str
    reference_shape: str
    allowlist_contract: tuple[str, ...]
    inventory_loader: Callable[[Path, "MigrationSpec"], list["InventorySite"]]


@dataclass(frozen=True)
class InventorySite:
    path: str
    line: int
    detail: str
    allowlist_entry: str | None = None

    def site_ref(self) -> str:
        return f"{self.path}:{self.line}"

    def entry_id(self) -> str:
        return self.allowlist_entry or f"{self.path}|{self.detail}"


@dataclass(frozen=True)
class ReconcileResult:
    ratchet: str
    action: str
    dedup_key: str | None
    issue_number: int | None = None
    parent_issue: int | None = None
    reason: str | None = None

    def to_dict(self) -> dict[str, object]:
        result: dict[str, object] = {
            "ratchet": self.ratchet,
            "action": self.action,
        }
        if self.dedup_key is not None:
            result["dedup_key"] = self.dedup_key
        if self.issue_number is not None:
            result["issue_number"] = self.issue_number
        if self.parent_issue is not None:
            result["parent_issue"] = self.parent_issue
        if self.reason is not None:
            result["reason"] = self.reason
        return result


class GithubClient:
    def run(self, argv: list[str]) -> str:
        result = subprocess.run(
            argv,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{' '.join(argv[:3])} failed: {result.stderr.strip()}")
        return result.stdout

    def issue_search(self, repo: str, state: str, query: str) -> list[dict[str, Any]]:
        stdout = self.run([
            "gh",
            "issue",
            "list",
            "--repo",
            repo,
            "--state",
            state,
            "--limit",
            "100",
            "--search",
            query,
            "--json",
            "number,title,state,author,body,url",
        ])
        return parse_json_list(stdout)

    def issue_view(self, repo: str, number: int, fields: str) -> dict[str, Any]:
        stdout = self.run([
            "gh",
            "issue",
            "view",
            str(number),
            "--repo",
            repo,
            "--json",
            fields,
        ])
        decoded = json.loads(stdout or "{}")
        if not isinstance(decoded, dict):
            raise ValueError("gh issue view did not return a JSON object")
        return decoded

    def issue_comment(self, repo: str, number: int, body: str) -> None:
        with NamedTemporaryFile("w", encoding="utf-8", suffix=".md", prefix="fkst-ratchet-", delete=True) as handle:
            handle.write(body)
            handle.flush()
            self.run(["gh", "issue", "comment", str(number), "--repo", repo, "--body-file", handle.name])

    def issue_create(self, repo: str, title: str, body: str, labels: list[str]) -> int | None:
        with NamedTemporaryFile("w", encoding="utf-8", suffix=".md", prefix="fkst-ratchet-", delete=True) as handle:
            handle.write(body)
            handle.flush()
            argv = [
                "gh",
                "issue",
                "create",
                "--repo",
                repo,
                "--title",
                title,
                "--body-file",
                handle.name,
            ]
            for label in labels:
                argv.extend(["--label", label])
            stdout = self.run(argv)
        return parse_created_issue_number(stdout)

    def issue_close(self, repo: str, number: int) -> None:
        self.run(["gh", "issue", "close", str(number), "--repo", repo])


def repo_rel(root: Path, path: Path) -> str:
    packages = root / "packages"
    try:
        return "packages/" + path.relative_to(packages).as_posix()
    except ValueError:
        return path.relative_to(root).as_posix()


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def validated_repo_path(root: Path, relpath: str) -> Path:
    if relpath.startswith("/") or relpath == "" or "\x00" in relpath:
        raise ValueError(f"invalid repository path: {relpath}")
    parts = Path(relpath).parts
    if ".." in parts:
        raise ValueError(f"invalid repository path: {relpath}")
    path = root / relpath
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError as exc:
        raise ValueError(f"repository path escapes root: {relpath}") from exc
    if not path.is_file():
        raise ValueError(f"repository path does not exist: {relpath}")
    return path


def line_for_offset(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def gh_git_head_locations(source: str) -> dict[str, int]:
    mask = gh_git_adapter.lua_code_mask(source)
    contexts = gh_git_adapter.lua_call_contexts(mask)
    literals = gh_git_adapter.lua_string_literals(source)
    literals_by_start = {literal.start: literal for literal in literals}
    locations: dict[str, int] = {}
    previous_literals: list[gh_git_adapter.LuaStringLiteral] = []
    for literal in literals:
        if gh_git_adapter.prior_literal_in_concat(source, literal, previous_literals):
            previous_literals.append(literal)
            continue
        head = gh_git_adapter.exec_argv_head_for_literal(source, mask, contexts, literal, literals_by_start)
        if head is None:
            command = gh_git_adapter.command_prefix_for_literal(source, mask, literal, literals_by_start)
            head = gh_git_adapter.normalized_command_head(command)
        if head is not None and not gh_git_adapter.is_excluded_literal(contexts, literal.start):
            locations.setdefault(head, line_for_offset(source, literal.start))
        previous_literals.append(literal)
    return locations


def load_gh_git_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist = gh_git_adapter.load_allowlist(validated_repo_path(root, spec.allowlist_path))
    sources = gh_git_adapter.sources(root, root / "packages", read_text, repo_rel)
    sites: list[InventorySite] = []
    for relpath, heads in sorted(allowlist.items()):
        source_path = validated_repo_path(root, relpath)
        source = sources.get(relpath) or read_text(source_path)
        locations = gh_git_head_locations(source)
        for head in sorted(heads):
            line = locations.get(head)
            if line is None:
                raise ValueError(f"allowlist entry is not present in source: {relpath} -> {head}")
            sites.append(InventorySite(relpath, line, f"command_head: {head}", f"{relpath}|{head}"))
    return sites


def load_saga_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist_path = validated_repo_path(root, spec.allowlist_path)
    sites: list[InventorySite] = []
    for raw in read_text(allowlist_path).splitlines():
        relpath = raw.strip()
        if not relpath or relpath.startswith("#"):
            continue
        source_path = validated_repo_path(root, relpath)
        source = read_text(source_path)
        masked = strip_lua_comments_and_strings(source)
        match = FREE_FORM_PIPELINE_RE.search(masked)
        if match is None:
            sites.append(InventorySite(relpath, 1, "stale_allowlist_entry", relpath))
        else:
            sites.append(InventorySite(relpath, line_for_offset(masked, match.start()), "free_form_pipeline", relpath))
    return sorted(sites, key=lambda site: (site.path, site.line, site.detail))


def load_code_dedup_inventory(root: Path, spec: MigrationSpec) -> list[InventorySite]:
    allowlist_path = validated_repo_path(root, spec.allowlist_path)
    sites: list[InventorySite] = []
    for entry in sorted(code_dedup.load_allowlist(allowlist_path)):
        for relpath in entry.files:
            source_path = validated_repo_path(root, relpath)
            source = read_text(source_path)
            line = line_for_function_basename(source, entry.name)
            detail = f"duplicate_function: {entry.name} {entry.body_hash}"
            sites.append(InventorySite(relpath, line or 1, detail, entry.allowlist_line()))
    return sorted(sites, key=lambda site: (site.path, site.line, site.detail))


def line_for_function_basename(source: str, basename: str) -> int | None:
    code = code_dedup.code_without_comments_and_strings(source)
    expected = code_dedup.function_basename(basename)
    for offset, line in enumerate(code.splitlines(), start=1):
        match = code_dedup.FUNCTION_RE.match(line)
        if match is not None and code_dedup.function_basename(match.group("name")) == expected:
            return offset
    return None


def strip_lua_comments_and_strings(text: str) -> str:
    return gh_git_adapter.lua_code_mask(text)


def specs() -> dict[str, MigrationSpec]:
    return {
        "gh-git-adapter": MigrationSpec(
            parent="891",
            ratchet="gh-git-adapter",
            migration_kind="allowlist",
            allowlist_path="migration/gh-git-adapter.allowlist",
            title="gh/git ports adapter allowlist migration slice",
            reference_shape="Migrate raw gh/git command construction behind forge.github/forge.git adapter operations.",
            allowlist_contract=(),
            inventory_loader=load_gh_git_inventory,
        ),
        "saga-handler": MigrationSpec(
            parent="979",
            ratchet="saga-handler",
            migration_kind="allowlist",
            allowlist_path="migration/saga-handler.allowlist",
            title="saga handler allowlist migration slice",
            reference_shape="Use the existing workflow.saga.department(spec, handlers) shape from migrated departments.",
            allowlist_contract=(
                "A `saga-handler` slice is single-flight by stable `dedup_key`: at most one live issue or PR surface may own the same `dedup_key`.",
                "Before opening or implementing a duplicate slice, prove the prior surface is stale, cancelled, invalid, or explicitly waived as a duplicate run.",
                "If the same `dedup_key` is already live without that proof, treat the slice as in-flight and make no source changes.",
            ),
            inventory_loader=load_saga_inventory,
        ),
        "code-dedup": MigrationSpec(
            parent="1018",
            ratchet="code-dedup",
            migration_kind="allowlist",
            allowlist_path="migration/code-dedup.allowlist",
            title="code dedup allowlist migration slice",
            reference_shape="Hoist the byte-identical production Lua function body to an existing shared module such as std.*, then call the shared helper from each site.",
            allowlist_contract=(
                "`migration/code-dedup.allowlist` is a shrink-only debt ledger, not an alternate duplicate inventory.",
                "The authoritative current inventory is derived from `check_repo_dedup.duplicate_groups`; an allowlist line is retained only while that exact duplicate group still exists.",
                "A `code-dedup` allowlist line is owned as one group by its function name, body hash, and listed file set.",
                "After a selected group is migrated so the exact duplicate group no longer exists, remove the whole matching allowlist line; do not preserve a reduced singleton entry such as `safe_segment` as a live allowlist exception.",
                "This spec-only slice explicitly waives a migration-slicer recurrence-class fix; any broader slicer deduplication change must be tracked separately.",
            ),
            inventory_loader=load_code_dedup_inventory,
        ),
    }


def markdown_code(value: str) -> str:
    text = str(value).replace("`", "\\`")
    return f"`{text}`"


def selected_sites(inventory: list[InventorySite], slice_size: int) -> list[InventorySite]:
    return inventory[:slice_size]


def site_records(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> list[dict[str, object]]:
    return [
        {
            "path": site.path,
            "line": site.line,
            "detail": site.detail,
            "site_ref": site.site_ref(),
            "allowlist_entry": site.entry_id(),
            "entry_key": entry_key(spec.allowlist_path, site),
        }
        for site in selected_sites(inventory, slice_size)
    ]


def entry_key(allowlist_path: str, site: InventorySite) -> str:
    encoded = (
        "fkst-migration-slice-v1"
        + "\0"
        + str(allowlist_path)
        + "\0"
        + site.entry_id()
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def sites_fingerprint(sites: list[dict[str, object]]) -> str:
    encoded = json.dumps(sites, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()[:16]


def slice_document(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> dict[str, object]:
    sites = site_records(spec, inventory, slice_size)
    fingerprint = sites_fingerprint(sites)
    return {
        "schema": SCHEMA,
        "ratchet": spec.ratchet,
        "parent_issue": int(spec.parent),
        "migration_kind": spec.migration_kind,
        "allowlist_path": spec.allowlist_path,
        "title": spec.title,
        "reference_shape": spec.reference_shape,
        "current_count": len(inventory),
        "target_count": TARGET_COUNT,
        "slice_size": slice_size,
        "selected_count": len(sites),
        "sites_fingerprint": fingerprint,
        "dedup_key": f"{spec.ratchet}/slice/{fingerprint}",
        "sites": sites,
    }


def controller_plan(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> dict[str, object]:
    if not inventory:
        return {
            "schema_version": SCHEMA,
            "ratchet": spec.ratchet,
            "allowlist_path": spec.allowlist_path,
            "remaining_count": 0,
            "slice_size": slice_size,
            "status": "inventory_empty",
            "next_slice": None,
        }
    doc = slice_document(spec, inventory, 1)
    return {
        "schema_version": SCHEMA,
        "ratchet": spec.ratchet,
        "allowlist_path": spec.allowlist_path,
        "remaining_count": len(inventory),
        "slice_size": 1,
        "status": "slice_available",
        "next_slice": {
            "dedup_key": doc["dedup_key"],
            "sites": doc["sites"],
            "title": slice_issue_title(doc),
            "body": render_reconciled_issue_body(spec, inventory, 1),
            "labels": list(DEFAULT_LABELS),
        },
    }


def ensure_marker_value(value: str) -> str:
    text = str(value)
    if SAFE_MARKER_VALUE_RE.fullmatch(text) is None:
        raise ValueError(f"unsafe marker value: {text}")
    return text


def issue_create_marker(dedup_key: str) -> str:
    return f"<!-- fkst:github-proxy:issue-create:{ensure_marker_value(dedup_key)} -->"


def issue_create_intent_marker(dedup_key: str) -> str:
    return f'<!-- fkst:github-proxy:issue-create-intent:v1 dedup="{ensure_marker_value(dedup_key)}" -->'


def issue_created_marker(dedup_key: str, issue_number: int | None) -> str:
    issue = "unknown" if issue_number is None else str(int(issue_number))
    return f'<!-- fkst:github-proxy:issue-created:v1 dedup="{ensure_marker_value(dedup_key)}" issue="{issue}" -->'


def ratchet_slice_marker(doc: dict[str, object]) -> str:
    sites = doc["sites"]
    entry_keys = ",".join(str(site.get("entry_key")) for site in sites if site.get("entry_key") is not None)
    entry_key = ""
    if len(sites) == 1 and sites[0].get("entry_key") is not None:
        entry_key = str(sites[0]["entry_key"])
    return (
        '<!-- fkst:ratchet-slice:v1'
        f' schema="{ensure_marker_value(str(doc["schema"]))}"'
        f' ratchet="{ensure_marker_value(str(doc["ratchet"]))}"'
        f' parent="{int(doc["parent_issue"])}"'
        f' dedup="{ensure_marker_value(str(doc["dedup_key"]))}"'
        f' fingerprint="{ensure_marker_value(str(doc["sites_fingerprint"]))}"'
        f' allowlist_path="{ensure_marker_value(str(doc["allowlist_path"]))}"'
        f' entry_key="{ensure_marker_value(entry_key)}"'
        f' generation="1"'
        f' coord_ref="{ensure_marker_value("refs/fkst/migration-slices/" + entry_key) if entry_key else ""}"'
        f' entries="{ensure_marker_value(entry_keys)}"'
        " -->"
    )


def ratchet_slice_search_query(ratchet: str) -> str:
    return f'fkst:ratchet-slice:v1 ratchet="{ensure_marker_value(ratchet)}"'


def selected_entry_keys(doc: dict[str, object]) -> set[str]:
    keys: set[str] = set()
    sites = doc.get("sites")
    if not isinstance(sites, list):
        return keys
    for site in sites:
        if isinstance(site, dict) and site.get("entry_key") is not None:
            keys.add(str(site["entry_key"]))
    return keys


def ratchet_slice_markers(text: str) -> list[str]:
    return re.findall(r"<!-- fkst:ratchet-slice:v1 .*?-->", text)


def marker_attribute(marker: str, name: str) -> str | None:
    match = re.search(r'\b' + re.escape(name) + r'="([^"]*)"', marker)
    if match is None:
        return None
    return match.group(1)


def marker_entry_keys(marker: str) -> set[str] | None:
    entries = marker_attribute(marker, "entries")
    if entries is None:
        return None
    if entries == "":
        return set()
    return {entry for entry in entries.split(",") if entry}


def slice_overlaps_entry(open_issue: dict[str, Any], ratchet: str, entry_keys: set[str]) -> bool:
    for marker in ratchet_slice_markers(record_body(open_issue)):
        if marker_attribute(marker, "ratchet") != ratchet:
            continue
        open_keys = marker_entry_keys(marker)
        if open_keys is None:
            return True
        if entry_keys & open_keys:
            return True
    return False


def issue_author_login(issue: dict[str, Any]) -> str | None:
    author = issue.get("author")
    if isinstance(author, dict) and author.get("login") is not None:
        return str(author["login"]).removesuffix("[bot]")
    if issue.get("author_login") is not None:
        return str(issue["author_login"]).removesuffix("[bot]")
    return None


def is_trusted_record(issue: dict[str, Any], bot_login: str | None) -> bool:
    if bot_login is None or bot_login == "":
        return True
    return issue_author_login(issue) == str(bot_login).removesuffix("[bot]")


def record_body(issue: dict[str, Any]) -> str:
    return str(issue.get("body") or "")


def matching_issues(issues: list[dict[str, Any]], marker: str, bot_login: str | None) -> list[dict[str, Any]]:
    return [
        issue
        for issue in issues
        if is_trusted_record(issue, bot_login) and marker in record_body(issue)
    ]


def comments_from_parent(parent: dict[str, Any]) -> list[dict[str, Any]]:
    comments = parent.get("comments")
    if isinstance(comments, list):
        return [comment for comment in comments if isinstance(comment, dict)]
    return []


def parent_has_marker(parent: dict[str, Any], marker: str, bot_login: str | None) -> bool:
    return bool(matching_issues(comments_from_parent(parent), marker, bot_login))


def parse_github_timestamp_seconds(value: object) -> int | None:
    if value is None:
        return None
    text = str(value).strip()
    if text == "":
        return None
    try:
        parsed = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return int(parsed.timestamp())


def comment_created_seconds(comment: dict[str, Any]) -> int | None:
    for key in ("createdAt", "created_at"):
        value = parse_github_timestamp_seconds(comment.get(key))
        if value is not None:
            return value
    return None


def recent_unknown_ledger_comment(comment: dict[str, Any], now_seconds: int) -> bool:
    created = comment_created_seconds(comment)
    if created is None:
        return False
    return 0 <= now_seconds - created <= UNKNOWN_LEDGER_HOLD_SECONDS


def parent_issue_created_marker_issues(
    parent: dict[str, Any],
    dedup_key: str,
    bot_login: str | None,
    now_seconds: int,
) -> list[int | object]:
    pattern = re.compile(r"<!-- fkst:github-proxy:issue-created:v1 .*?-->")
    expected = f'dedup="{ensure_marker_value(dedup_key)}"'
    issues: list[int | object] = []
    for comment in comments_from_parent(parent):
        if not is_trusted_record(comment, bot_login):
            continue
        for marker in pattern.findall(record_body(comment)):
            if expected in marker:
                issue = marker_attribute(marker, "issue")
                if issue is None or issue == "unknown":
                    if recent_unknown_ledger_comment(comment, now_seconds):
                        issues.append(UNRESOLVED_LEDGER_ISSUE)
                    continue
                try:
                    issues.append(int(issue))
                except ValueError:
                    if recent_unknown_ledger_comment(comment, now_seconds):
                        issues.append(UNRESOLVED_LEDGER_ISSUE)
                    continue
    return issues


def parse_json_list(stdout: str) -> list[dict[str, Any]]:
    decoded = json.loads(stdout or "[]")
    if not isinstance(decoded, list):
        raise ValueError("GitHub command did not return a JSON list")
    return [item for item in decoded if isinstance(item, dict)]


def parse_created_issue_number(stdout: str) -> int | None:
    text = str(stdout or "")
    match = re.search(r"/issues/(\d+)", text) or re.search(r"#(\d+)", text)
    if match is None:
        return None
    return int(match.group(1))


def issue_number(issue: dict[str, Any]) -> int | None:
    try:
        return int(issue.get("number"))
    except (TypeError, ValueError):
        return None


def render_reconciled_issue_body(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> str:
    doc = slice_document(spec, inventory, slice_size)
    return (
        render_child_issue(spec, inventory, slice_size).replace(
            "Dry-run child issue draft. No GitHub state was modified.",
            "Machine-filed ratchet slice issue.",
        )
        + "\n"
        + issue_create_marker(str(doc["dedup_key"]))
        + "\n"
        + ratchet_slice_marker(doc)
        + "\n"
    )


def slice_issue_title(doc: dict[str, object]) -> str:
    return f"{doc['title']}: {doc['sites_fingerprint']}"


def validate_repo(repo: str) -> str:
    if SAFE_REPO_RE.fullmatch(str(repo)) is None:
        raise ValueError(f"invalid GitHub repository: {repo}")
    return str(repo)


def trusted_bot_login(write_enabled: bool, env: dict[str, str]) -> str | None:
    bot_login = env.get("FKST_GITHUB_BOT_LOGIN")
    if write_enabled and not bot_login:
        raise ValueError("FKST_GITHUB_BOT_LOGIN is required when FKST_GITHUB_WRITE=1")
    return bot_login


def reconcile_ratchet(
    spec: MigrationSpec,
    inventory: list[InventorySite],
    slice_size: int,
    repo: str,
    client: GithubClient,
    env: dict[str, str] | None = None,
    labels: list[str] | None = None,
) -> ReconcileResult:
    env = dict(os.environ) if env is None else env
    labels = list(labels or DEFAULT_LABELS)
    repo = validate_repo(repo)
    write_enabled = env.get("FKST_GITHUB_WRITE") == "1"
    bot_login = trusted_bot_login(write_enabled, env)
    parent_issue = int(spec.parent)

    parent = client.issue_view(repo, parent_issue, "number,state,comments")
    if not inventory:
        state = str(parent.get("state") or "").upper()
        if state != "OPEN":
            return ReconcileResult(spec.ratchet, "parent-already-closed", None, parent_issue=parent_issue)
        if write_enabled:
            client.issue_close(repo, parent_issue)
            return ReconcileResult(spec.ratchet, "closed-parent", None, parent_issue=parent_issue)
        return ReconcileResult(spec.ratchet, "would-close-parent", None, parent_issue=parent_issue, reason="FKST_GITHUB_WRITE!=1")

    doc = slice_document(spec, inventory, slice_size)
    dedup_key = str(doc["dedup_key"])
    ledger_issues = parent_issue_created_marker_issues(parent, dedup_key, bot_login, int(time()))
    for ledger_issue in ledger_issues:
        if ledger_issue is UNRESOLVED_LEDGER_ISSUE:
            return ReconcileResult(spec.ratchet, "deduped-parent-ledger", dedup_key, parent_issue=parent_issue)
        prior = client.issue_view(repo, int(ledger_issue), "number,state,author,body")
        if not is_trusted_record(prior, bot_login) or str(prior.get("state") or "").upper() != "CLOSED":
            return ReconcileResult(
                spec.ratchet,
                "deduped-parent-ledger",
                dedup_key,
                issue_number=int(ledger_issue),
                parent_issue=parent_issue,
            )

    open_candidates = client.issue_search(repo, "open", ratchet_slice_search_query(spec.ratchet))
    entry_keys = selected_entry_keys(doc)
    open_slices = [
        issue
        for issue in open_candidates
        if is_trusted_record(issue, bot_login)
        and "fkst:ratchet-slice:v1" in record_body(issue)
        and f'ratchet="{spec.ratchet}"' in record_body(issue)
        and slice_overlaps_entry(issue, spec.ratchet, entry_keys)
    ]
    if open_slices:
        return ReconcileResult(
            spec.ratchet,
            "deduped-in-flight",
            dedup_key,
            issue_number=issue_number(open_slices[0]),
            parent_issue=parent_issue,
        )

    exact_marker = issue_create_marker(dedup_key)
    existing = matching_issues(client.issue_search(repo, "open", exact_marker), exact_marker, bot_login)
    if existing:
        return ReconcileResult(
            spec.ratchet,
            "deduped-existing-slice",
            dedup_key,
            issue_number=issue_number(existing[0]),
            parent_issue=parent_issue,
        )

    if not write_enabled:
        return ReconcileResult(spec.ratchet, "would-create-slice", dedup_key, parent_issue=parent_issue, reason="FKST_GITHUB_WRITE!=1")

    intent = issue_create_intent_marker(dedup_key)
    if not parent_has_marker(parent, intent, bot_login):
        client.issue_comment(repo, parent_issue, intent + "\n")

    issue = client.issue_create(repo, slice_issue_title(doc), render_reconciled_issue_body(spec, inventory, slice_size), labels)
    client.issue_comment(repo, parent_issue, issue_created_marker(dedup_key, issue) + "\n")
    return ReconcileResult(spec.ratchet, "created-slice", dedup_key, issue_number=issue, parent_issue=parent_issue)


def render_child_issue(spec: MigrationSpec, inventory: list[InventorySite], slice_size: int) -> str:
    doc = slice_document(spec, inventory, slice_size)
    selected = selected_sites(inventory, slice_size)
    lines = [
        f"# {spec.title}",
        "",
        "Dry-run child issue draft. No GitHub state was modified.",
        "",
        "## Ratchet",
        f"- parent_issue: #{spec.parent}",
        f"- ratchet: {markdown_code(spec.ratchet)}",
        f"- migration_kind: {markdown_code(spec.migration_kind)}",
        f"- allowlist_path: {markdown_code(spec.allowlist_path)}",
        f"- current_count: {len(inventory)}",
        f"- target_count: {TARGET_COUNT}",
        f"- slice_size: {slice_size}",
        f"- selected_count: {len(selected)}",
        f"- sites_fingerprint: {markdown_code(str(doc['sites_fingerprint']))}",
        f"- dedup_key: {markdown_code(str(doc['dedup_key']))}",
        "",
        "## Reference Shape",
        spec.reference_shape,
        "",
    ]
    if spec.allowlist_contract:
        lines.extend(["## Allowlist Contract"])
        lines.extend(f"- {line}" for line in spec.allowlist_contract)
        lines.append("")
    lines.append("## Exact Sites")
    if selected:
        for site in selected:
            lines.append(f"- {markdown_code(site.site_ref())} ({markdown_code(site.detail)})")
    else:
        lines.append("- none")
    lines.extend(
        [
            "",
            "## Acceptance Criteria",
            "- Migrate only the exact sites listed above.",
            f"- Remove only those migrated entries from `{spec.allowlist_path}`.",
            "- If every listed site is already migrated and every corresponding allowlist entry is already absent, treat the slice as already converged and make no source changes.",
            f"- The allowlist count decreases only for listed entries still present in `{spec.allowlist_path}` and may be unchanged only when the slice is already converged.",
            "- Behavior is preserved.",
            "- `scripts/run.sh test` exits 0.",
            "- No broad cleanup, opportunistic refactors, or unrelated migration work.",
        ]
    )
    if not selected:
        lines.append("- No child issue is needed because the target count is already reached.")
    return "\n".join(lines) + "\n"


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Slice and optionally reconcile code-owned allowlist ratchets.",
    )
    parser.add_argument("ratchet", choices=(*VALID_RATCHETS, *RATCHET_ALIASES.keys(), "all"), help="Code-owned ratchet selector.")
    parser.add_argument("--repo-root", default=Path.cwd(), type=Path)
    parser.add_argument("--slice-size", type=int, default=DEFAULT_SLICE_SIZE)
    parser.add_argument("--json", action="store_true", help="Emit the stable machine-readable slice schema.")
    parser.add_argument("--reconcile", action="store_true", help="Reconcile slice issue creation or parent closure through GitHub.")
    parser.add_argument("--repo", help="GitHub owner/repo used with --reconcile.")
    parser.add_argument("--label", action="append", dest="labels", help="Label to add to created slice issues; repeatable.")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(list(sys.argv[1:] if argv is None else argv))
    if args.slice_size < 1 or args.slice_size > MAX_SLICE_SIZE:
        print(f"error: --slice-size must be between 1 and {MAX_SLICE_SIZE}", file=sys.stderr)
        return 2
    root = args.repo_root.resolve()
    ratchets = DEFAULT_RECONCILE_RATCHETS if args.ratchet == "all" else (RATCHET_ALIASES.get(args.ratchet, args.ratchet),)
    all_specs = specs()
    try:
        inventories = [(all_specs[ratchet], all_specs[ratchet].inventory_loader(root, all_specs[ratchet])) for ratchet in ratchets]
    except Exception as exc:
        print(f"error: ratchet inventory failed: {exc}", file=sys.stderr)
        return 1
    if args.reconcile:
        if not args.repo:
            print("error: --repo is required with --reconcile", file=sys.stderr)
            return 2
        try:
            results = [
                reconcile_ratchet(spec, inventory, args.slice_size, args.repo, GithubClient(), labels=args.labels)
                for spec, inventory in inventories
            ]
        except Exception as exc:
            print(f"error: ratchet reconcile failed: {exc}", file=sys.stderr)
            return 1
        if args.json:
            print(json.dumps([result.to_dict() for result in results], sort_keys=True, ensure_ascii=False))
        else:
            for result in results:
                print(json.dumps(result.to_dict(), sort_keys=True, ensure_ascii=False))
    elif args.json:
        if len(inventories) != 1:
            print("error: --json without --reconcile requires one ratchet", file=sys.stderr)
            return 2
        spec, inventory = inventories[0]
        print(json.dumps(controller_plan(spec, inventory, args.slice_size), sort_keys=True, ensure_ascii=False))
    else:
        if len(inventories) != 1:
            print("error: dry-run body output requires one ratchet", file=sys.stderr)
            return 2
        spec, inventory = inventories[0]
        print(render_child_issue(spec, inventory, args.slice_size), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
