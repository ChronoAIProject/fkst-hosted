"""GitHub authored-content ingress migration backstop.

The GitHub capability seam owns primary enforcement: production code receives an
authorized GitHub handle whose raw egress path applies the authored-content
filter. This scanner is a shrink-only migration backstop that keeps known bypass
shapes from regressing while the Lua surface continues converging.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable


RULE = "G-GITHUB-CONTENT-INGRESS"
PROMPT_EXTERNAL_FETCH_ALLOWLIST = "migration/prompt-external-fetch.allowlist"
PROMPT_EXTERNAL_FETCH_RE = re.compile(
    r"\bgh\s+(?:api\b|issue\s+view\b|pr\s+(?:view|diff)\b)"
    r"|\bgit\s+(?:clone|fetch|pull|ls-remote)\b",
    re.IGNORECASE,
)
WRAPPER_NEEDLES = {
    "libraries/forge/github/exec.lua": ("content_filter.apply_gh_content_filter",),
}
POLICY_FACTORY_NEEDLE = "devloop.github_factory"
AUTHORED_LIST_HELPER_SHAPES = {
    "issue_list_argv": "issue_list",
    "issue_list_cli_argv": "issue_list",
    "issue_list_observe_argv": "issue_list",
    "issue_list_open_assigned_argv": "issue_list",
    "issue_search_argv": "issue_list",
    "pr_list_argv": "pr_list",
    "pr_list_cli_argv": "pr_list",
    "pr_list_head_argv": "pr_list",
    "pr_list_merge_queue_argv": "pr_list",
    "pr_list_observe_argv": "pr_list",
    "pr_list_recent_merged_argv": "pr_list",
}


def matching_call(text: str, open_paren: int) -> str:
    depth = 0
    cursor = open_paren
    while cursor < len(text):
        char = text[cursor]
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
            if depth == 0:
                return text[open_paren : cursor + 1]
        cursor += 1
    return text[open_paren:]


def top_level_commas(text: str) -> int:
    depth = 0
    commas = 0
    for char in text:
        if char in "([{":
            depth += 1
        elif char in ")]}" and depth > 0:
            depth -= 1
        elif char == "," and depth == 1:
            commas += 1
    return commas


def production_lua_sources(
    root: Path,
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    package_lua_files: Callable[[Path], list[tuple[Path, Path]]],
) -> list[tuple[str, str]]:
    paths = [path for _packages, path in package_lua_files(root)]
    for scan_root in (
        root / "libraries" / "forge" / "github",
        root / "libraries" / "forge" / "merge",
        root / "libraries" / "devloop",
    ):
        if scan_root.exists():
            paths.extend(path for path in sorted(scan_root.rglob("*.lua")) if path.is_file())
    for relpath in ("libraries/forge/ports.lua", "libraries/forge/merge.lua"):
        path = root / relpath
        if path.is_file():
            paths.append(path)
    sources = []
    for path in sorted(set(paths)):
        relpath = rel(root, path)
        if "/tests/" in relpath or relpath.endswith("_test.lua"):
            continue
        if relpath in {
            "libraries/forge/github.lua",
            "libraries/forge/github_fake.lua",
        }:
            continue
        sources.append((relpath, read_text(path)))
    return sources


def file_has_obfuscated_gh_head(source: str) -> bool:
    return re.search(
        r"table\s*\.\s*concat\s*\(\s*\{\s*['\"]g['\"]\s*,\s*['\"]h['\"]\s*\}",
        source,
    ) is not None


def is_allowed_policyless_github_construction(relpath: str) -> bool:
    return relpath in {
        "libraries/forge/github.lua",
        "libraries/forge/merge_commands.lua",
        "libraries/devloop/github_factory.lua",
    } or "/tests/" in relpath or relpath.endswith("_test.lua")


def authored_api_path_literal(call: str) -> bool:
    return re.search(r"repos/[^\"']+/[^\"']+/(issues|pulls)(?:\?|/\d+)", call) is not None


def authored_list_helper_shape(call: str) -> tuple[str, str] | None:
    for helper, shape in AUTHORED_LIST_HELPER_SHAPES.items():
        if re.search(r"\b" + re.escape(helper) + r"\s*\(", call) is not None:
            return helper.removesuffix("_argv"), shape
    return None


def has_content_json_shape(call: str, shape: str) -> bool:
    return re.search(
        r"\bstdout_policy\s*\.\s*content_json\s*\(\s*['\"]" + re.escape(shape) + r"['\"]\s*\)",
        call,
    ) is not None


def has_explicit_stdout_policy(call: str) -> bool:
    return (
        "stdout_policy." in call
        or re.search(r"\b(?:api_paginate_slurp_policy|api_method_policy)\s*\(", call) is not None
    )


def is_unmasked_range(source: str, stripped: str, start: int, end: int) -> bool:
    for index in range(start, end):
        if source[index] in (" ", "\n"):
            continue
        if stripped[index] != source[index]:
            return False
    return True


def raw_call_for_stripped_call(source: str, open_paren: int, stripped_call: str) -> str:
    return source[open_paren : open_paren + len(stripped_call)]


def policyless_require_github_constructions(source: str, stripped: str) -> list[tuple[int, str]]:
    found: list[tuple[int, str]] = []
    for pattern in (
        r"require\s*\(\s*[\"']forge\.github[\"']\s*\)\s*\.\s*new\s*\(",
    ):
        for match in re.finditer(pattern, source):
            quote = source.find("\"", match.start(), match.end())
            if quote == -1:
                quote = source.find("'", match.start(), match.end())
            if quote == -1:
                continue
            if not is_unmasked_range(source, stripped, match.start(), quote):
                continue
            call = matching_call(stripped, match.end() - 1) if pattern.endswith(r"\(") else ""
            found.append((match.start(), call))
    return found


def prompt_external_fetch_sites(root: Path, read_text: Callable[[Path], str], rel: Callable[[Path, Path], str]) -> list[tuple[str, int, str]]:
    packages = root / "packages"
    sites: list[tuple[str, int, str]] = []
    if not packages.exists():
        return sites
    for path in sorted(packages.glob("*/prompts/*.lua")):
        text = read_text(path)
        for match in PROMPT_EXTERNAL_FETCH_RE.finditer(text):
            sites.append((rel(root, path), text.count("\n", 0, match.start()) + 1, match.group(0)))
    return sites


def messages(
    root: Path,
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
    package_lua_files: Callable[[Path], list[tuple[Path, Path]]],
    strip_lua_comments_and_strings: Callable[[str], str],
) -> list[str]:
    violations: list[str] = []
    allowlist_path = root / PROMPT_EXTERNAL_FETCH_ALLOWLIST
    if not allowlist_path.is_file():
        violations.append(
            f"{PROMPT_EXTERNAL_FETCH_ALLOWLIST} is missing; prompt external-fetch inventory must remain at zero"
        )
    else:
        entries = [
            line.strip()
            for line in read_text(allowlist_path).splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        ]
        if entries:
            violations.append(
                f"{PROMPT_EXTERNAL_FETCH_ALLOWLIST} must remain empty; raw prompt fetch bypasses are forbidden"
            )

    for relpath, line, command in prompt_external_fetch_sites(root, read_text, rel):
        violations.append(
            f"{relpath}:{line} prompt contains raw external-content fetch {command!r}; "
            "prefetch through filtered forge.github and pass supplied context"
        )

    for relpath, needles in WRAPPER_NEEDLES.items():
        path = root / relpath
        text = read_text(path) if path.is_file() else ""
        for needle in needles:
            if needle not in text:
                violations.append(
                    f"{relpath} must apply the shared GitHub content filter at gh ingress (missing: {needle})"
                )

    for relpath, text in production_lua_sources(root, read_text, rel, package_lua_files):
        stripped = strip_lua_comments_and_strings(text)
        for match in re.finditer(r"\bhandle\s*\.\s*_exec\s*\(", stripped):
            call = matching_call(stripped, match.end() - 1)
            if top_level_commas(call) < 3 or not has_explicit_stdout_policy(call):
                line = text.count("\n", 0, match.start()) + 1
                violations.append(f"{relpath}:{line} gh handle._exec call must declare a stdout_policy")
        if relpath not in WRAPPER_NEEDLES:
            obfuscated_head = file_has_obfuscated_gh_head(text)
            for match in re.finditer(r"\bexec_argv\s*\(", stripped):
                call = matching_call(stripped, match.end() - 1)
                raw = text[match.start() : match.start() + len(call)]
                if "argv" in call and ('"gh"' in raw or "'gh'" in raw or obfuscated_head):
                    line = text.count("\n", 0, match.start()) + 1
                    violations.append(
                        f"{relpath}:{line} raw gh exec_argv egress bypasses the GitHub capability seam; use forge.github.exec.run"
                    )
        if not is_allowed_policyless_github_construction(relpath):
            for start, call in policyless_require_github_constructions(text, stripped):
                line = text.count("\n", 0, start) + 1
                if call == "" or ("trusted_author_policy" not in call and "github_author_policy.github_options" not in call):
                    violations.append(
                        f"{relpath}:{line} production forge.github construction bypasses the GitHub capability seam; use {POLICY_FACTORY_NEEDLE} or pass an explicit trusted_author_policy"
                    )
            for match in re.finditer(r"\bgithub_adapter\s*\.\s*new\s*\(", stripped):
                call = matching_call(stripped, match.end() - 1)
                line = text.count("\n", 0, match.start()) + 1
                if "trusted_author_policy" not in call and "github_author_policy.github_options" not in call:
                    violations.append(
                        f"{relpath}:{line} production forge.github construction bypasses the GitHub capability seam; use {POLICY_FACTORY_NEEDLE} or pass an explicit trusted_author_policy"
                    )
        for match in re.finditer(r"\bhandle\s*\.\s*_exec\s*\(", stripped):
            call = matching_call(stripped, match.end() - 1)
            raw_call = raw_call_for_stripped_call(text, match.end() - 1, call)
            line = text.count("\n", 0, match.start()) + 1
            if authored_api_path_literal(raw_call) and "stdout_policy.content_json" not in call:
                violations.append(f"{relpath}:{line} authored GitHub API read must declare stdout_policy.content_json")
            helper_shape = authored_list_helper_shape(call)
            if helper_shape is not None:
                helper, shape = helper_shape
                expected = f'stdout_policy.content_json("{shape}")'
                if not has_content_json_shape(raw_call, shape):
                    violations.append(f"{relpath}:{line} {helper} must declare {expected}")
    return violations
