"""Lua production coverage shrink-only ratchet over engine coverage metadata."""

from __future__ import annotations

import json
import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ALLOWLIST = "migration/coverage-uncovered.allowlist"
DEFAULT_ARTIFACTS = (".fkst/run/lua-coverage/coverage.json", ".fkst/run/coverage.json")
HASH_RE = re.compile(r"[0-9a-f]{8,128}")


@dataclass(frozen=True, order=True)
class CoverageKey:
    file: str
    line: int
    normalized_line_hash: str

    def label(self) -> str:
        return f"{self.file}:{self.line} {self.normalized_line_hash}"


@dataclass(frozen=True)
class UncoveredLine:
    key: CoverageKey
    text: str

    def label(self) -> str:
        return f"{self.key.file}:{self.key.line}:{self.text}"


def is_production_lua_path(path: str) -> bool:
    if not path.endswith(".lua"):
        return False
    if not (path.startswith("packages/") or path.startswith("std/")):
        return False
    parts = path.split("/")
    if "tests" in parts:
        return False
    return not path.endswith(("_test.lua", "_helpers.lua", "_fake.lua"))


def parse_positive_int(value: Any, context: str) -> int:
    if isinstance(value, bool):
        raise ValueError(f"{context} must be a positive integer")
    try:
        line = int(value)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"{context} must be a positive integer") from exc
    if line < 1:
        raise ValueError(f"{context} must be a positive integer")
    return line


def parse_hash(value: Any, context: str) -> str:
    if not isinstance(value, str) or HASH_RE.fullmatch(value) is None:
        raise ValueError(f"{context} must be a lower-case hex normalized line hash")
    return value


def load_allowlist(path: Path) -> set[CoverageKey]:
    if not path.exists():
        return set()
    entries: set[CoverageKey] = set()
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        try:
            item = json.loads(stripped)
        except json.JSONDecodeError as exc:
            raise ValueError(f"invalid {ALLOWLIST} JSON on line {number}: {exc.msg}") from exc
        if not isinstance(item, dict):
            raise ValueError(f"invalid {ALLOWLIST} line {number}: expected JSON object")
        file = item.get("file")
        reason = item.get("reason")
        if not isinstance(file, str) or not is_production_lua_path(file):
            raise ValueError(f"invalid {ALLOWLIST} line {number}: file must be a production Lua path")
        if not isinstance(reason, str) or reason.strip() == "":
            raise ValueError(f"invalid {ALLOWLIST} line {number}: reason is required")
        entries.add(
            CoverageKey(
                file=file,
                line=parse_positive_int(item.get("line"), f"{ALLOWLIST} line {number} field 'line'"),
                normalized_line_hash=parse_hash(
                    item.get("normalized_line_hash"),
                    f"{ALLOWLIST} line {number} field 'normalized_line_hash'",
                ),
            )
        )
    return entries


def line_number(entry: Any, fallback: int | None, context: str) -> int:
    if isinstance(entry, int):
        return parse_positive_int(entry, context)
    if isinstance(entry, dict):
        return parse_positive_int(entry.get("line", fallback), context)
    return parse_positive_int(fallback, context)


def line_hash(entry: Any, indexed: dict[int, dict[str, Any]], line: int, context: str) -> str:
    if isinstance(entry, dict):
        value = entry.get("normalized_line_hash", entry.get("hash"))
        if value is not None:
            return parse_hash(value, context)
    indexed_entry = indexed.get(line, {})
    return parse_hash(indexed_entry.get("normalized_line_hash", indexed_entry.get("hash")), context)


def line_text(entry: Any, indexed: dict[int, dict[str, Any]], line: int) -> str:
    if isinstance(entry, dict):
        for field in ("text", "source", "line_text", "normalized_line"):
            value = entry.get(field)
            if isinstance(value, str):
                return value
    indexed_entry = indexed.get(line, {})
    for field in ("text", "source", "line_text", "normalized_line"):
        value = indexed_entry.get(field)
        if isinstance(value, str):
            return value
    return ""


def line_entries(value: Any) -> list[Any]:
    if value is None:
        return []
    if isinstance(value, list):
        return value
    raise ValueError("coverage line metadata must be a list")


def index_line_entries(entries: list[Any]) -> dict[int, dict[str, Any]]:
    indexed: dict[int, dict[str, Any]] = {}
    for entry in entries:
        if isinstance(entry, dict) and "line" in entry:
            indexed[parse_positive_int(entry.get("line"), "coverage line field 'line'")] = entry
    return indexed


def covered_line_set(value: Any) -> set[int]:
    covered: set[int] = set()
    for entry in line_entries(value):
        covered.add(line_number(entry, None, "covered line"))
    return covered


def missing_from_file(path: str, data: dict[str, Any]) -> list[UncoveredLine]:
    coverable = line_entries(data.get("coverable_lines", data.get("coverable", data.get("lines"))))
    indexed = index_line_entries(coverable)
    missing = data.get("missing_lines", data.get("uncovered_lines", data.get("missing", data.get("uncovered"))))
    if missing is None and coverable:
        covered = covered_line_set(data.get("covered_lines", data.get("covered")))
        missing = [
            entry for entry in coverable
            if isinstance(entry, dict) and entry.get("coverable", True) is not False
            and not bool(entry.get("covered", line_number(entry, None, "coverable line") in covered))
        ]
    result: list[UncoveredLine] = []
    for entry in line_entries(missing):
        line = line_number(entry, None, "uncovered line")
        result.append(
            UncoveredLine(
                key=CoverageKey(
                    file=path,
                    line=line,
                    normalized_line_hash=line_hash(entry, indexed, line, "uncovered line normalized_line_hash"),
                ),
                text=line_text(entry, indexed, line),
            )
        )
    return result


def coverage_files(data: dict[str, Any]) -> list[dict[str, Any]]:
    files = data.get("files")
    if isinstance(files, list):
        return [item for item in files if isinstance(item, dict)]
    if isinstance(files, dict):
        return [
            {"file": path, **item}
            for path, item in files.items()
            if isinstance(path, str) and isinstance(item, dict)
        ]
    return []


def uncovered_from_artifact(path: Path) -> dict[CoverageKey, UncoveredLine]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("coverage artifact must be a JSON object")
    result: dict[CoverageKey, UncoveredLine] = {}
    files = coverage_files(data)
    top_level_missing = line_entries(data.get("missing_lines", data.get("uncovered_lines")))
    if not files and not top_level_missing:
        raise ValueError("coverage artifact has no engine-authored Lua line metadata")
    for item in top_level_missing:
        if not isinstance(item, dict):
            raise ValueError("top-level uncovered lines must include file metadata")
        file = item.get("file", item.get("path"))
        if isinstance(file, str) and is_production_lua_path(file):
            line = line_number(item, None, "top-level uncovered line")
            uncovered = UncoveredLine(
                CoverageKey(file, line, line_hash(item, {}, line, "top-level uncovered line normalized_line_hash")),
                line_text(item, {}, line),
            )
            result[uncovered.key] = uncovered
    for file_data in files:
        file = file_data.get("file", file_data.get("path"))
        if not isinstance(file, str) or not is_production_lua_path(file):
            continue
        for uncovered in missing_from_file(file, file_data):
            result[uncovered.key] = uncovered
    return result


def artifact_path(root: Path) -> Path | None:
    explicit = os.environ.get("FKST_LUA_COVERAGE_JSON")
    if explicit:
        return Path(explicit)
    for relpath in DEFAULT_ARTIFACTS:
        candidate = root / relpath
        if candidate.exists():
            return candidate
    return None


def allowlist_at_dev_base(root: Path) -> tuple[str, set[CoverageKey] | None]:
    try:
        git = lambda args, **kwargs: subprocess.run(["git", *args], cwd=root, check=False, **kwargs)
        if git(["rev-parse", "--verify", "dev"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            return "unresolved", None
        base = git(["merge-base", "HEAD", "dev"], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        base_commit = base.stdout.strip()
        if base.returncode != 0 or base_commit == "":
            return "unresolved", None
        base_allowlist = base_commit + ":" + ALLOWLIST
        if git(["cat-file", "-e", base_allowlist], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            return "absent", None
        shown = git(["show", base_allowlist], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if shown.returncode != 0:
            return "unresolved", None
        tmp = root / ".git" / "fkst-coverage-base.allowlist"
        tmp.write_text(shown.stdout, encoding="utf-8")
        try:
            return "present", load_allowlist(tmp)
        finally:
            tmp.unlink(missing_ok=True)
    except Exception:
        return "unresolved", None


def ratchet_messages(
    uncovered: dict[CoverageKey, UncoveredLine],
    allowlist: set[CoverageKey],
    base_allowlist: set[CoverageKey] | None = None,
) -> list[str]:
    messages: list[str] = []
    for key in sorted(set(uncovered) - allowlist):
        messages.append(
            f"{uncovered[key].label()} is an uncovered production Lua line not in {ALLOWLIST}"
        )
    for key in sorted(allowlist - set(uncovered)):
        messages.append(f"{key.label()} is no longer uncovered; prune the stale entry from {ALLOWLIST}")
    if base_allowlist is not None:
        for key in sorted(allowlist - base_allowlist):
            messages.append(f"{key.label()} grows {ALLOWLIST} relative to dev; cover the line instead")
    return messages


def repository_messages(root: Path) -> list[str]:
    path = artifact_path(root)
    required = os.environ.get("FKST_LUA_COVERAGE_REQUIRED") == "1" or os.environ.get("FKST_LUA_COVERAGE_JSON") is not None
    if path is None:
        return ["Lua coverage artifact is required but was not found"] if required else []
    if not path.exists():
        return [f"Lua coverage artifact does not exist: {path}"]
    try:
        uncovered = uncovered_from_artifact(path)
        allowlist = load_allowlist(root / ALLOWLIST)
        base_status, base_allowlist = allowlist_at_dev_base(root)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        return [f"invalid Lua coverage ratchet input: {exc}"]
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(uncovered, allowlist, base_allowlist))
    return messages
