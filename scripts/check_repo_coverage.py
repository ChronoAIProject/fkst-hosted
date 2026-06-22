"""Lua production coverage shrink-only ratchet over engine coverage metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import subprocess
import tempfile
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ALLOWLIST = "migration/coverage-uncovered.allowlist"
REQUIRED_FLAG = "migration/coverage-uncovered.required"
DEFAULT_ARTIFACTS = (".fkst/run/lua-coverage/coverage.json", ".fkst/run/coverage.json")
HASH_RE = re.compile(r"[0-9a-f]{8,128}")
BASE_REF_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/\-]*")
LUA_KEYWORDS = {
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function",
    "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
    "true", "until", "while",
}
LUA_NAME_RE = r"[A-Za-z_][A-Za-z0-9_]*(?:[.:][A-Za-z_][A-Za-z0-9_]*)*"
CLOSING_DELIMITER_ONLY_RE = re.compile(r"^[)\]\},\s]+$")
PURE_FUNCTION_SIGNATURE_RE = re.compile(
    rf"^(?:"
    rf"local function {LUA_NAME_RE}"
    rf"|function {LUA_NAME_RE}"
    rf"|local {LUA_NAME_RE}\s*=\s*function"
    rf"|{LUA_NAME_RE}\s*=\s*function"
    rf"|return function"
    rf"|function"
    rf")\s*\([^)]*\)$"
)


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
    if not (path.startswith("packages/") or path.startswith("libraries/forge/") or path.startswith("libraries/")):
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


def normalize_source_line(text: str) -> str:
    return re.sub(r"\s+", " ", text.strip())


def normalized_source_hash(text: str) -> str:
    return hashlib.sha256(normalize_source_line(text).encode("utf-8")).hexdigest()[:16]


def without_lua_string_literals(text: str) -> str:
    result: list[str] = []
    quote: str | None = None
    escape = False
    for char in text:
        if quote is not None:
            if escape:
                escape = False
            elif char == "\\":
                escape = True
            elif char == quote:
                quote = None
            result.append(" ")
        elif char in {"'", '"'}:
            quote = char
            result.append(" ")
        else:
            result.append(char)
    return "".join(result)


def strip_lua_line_comment(text: str) -> str:
    candidate = without_lua_string_literals(text)
    idx = candidate.find("--")
    if idx == -1:
        return text
    return text[:idx]


def is_candidate_executable_lua_line(text: str) -> bool:
    comment_stripped = strip_lua_line_comment(text)
    stripped = normalize_source_line(comment_stripped)
    if stripped == "":
        return False
    if stripped in {"end", "else", "then", "do", "until"}:
        return False
    if stripped in {"end,", "end)", "end}", "else,"}:
        return False
    if CLOSING_DELIMITER_ONLY_RE.fullmatch(stripped) is not None:
        return False
    if PURE_FUNCTION_SIGNATURE_RE.fullmatch(stripped) is not None:
        return False
    if stripped.startswith("--"):
        return False
    if stripped in LUA_KEYWORDS:
        return False
    without_strings = re.sub(r"\s+", "", without_lua_string_literals(comment_stripped))
    if without_strings == "" or re.fullmatch(r"[)\]\},]+", without_strings) is not None:
        return False
    return True


def source_lines_for_file(root: Path, file: str) -> list[str]:
    path = root / file
    if not path.is_file():
        raise ValueError(f"coverage artifact references missing source file: {file}")
    return path.read_text(encoding="utf-8").splitlines()


def candidate_lines(root: Path, file: str, lines: list[str]) -> set[int]:
    return {
        idx for idx, text in enumerate(lines, start=1)
        if is_candidate_executable_lua_line(text)
    }


def coverage_map_files(data: dict[str, Any]) -> dict[str, dict[str, Any]]:
    if "files" in data:
        return {}
    result: dict[str, dict[str, Any]] = {}
    for file, item in data.items():
        if isinstance(file, str) and isinstance(item, dict):
            result[file] = item
    return result


def repository_coverage_path(file: str, package_name: str | None = None) -> str:
    if is_production_lua_path(file):
        return file
    if package_name is None or file.startswith("../") or file.startswith("/"):
        return file
    if file.startswith("libraries/forge/") or file.startswith("libraries/"):
        return file
    return f"packages/{package_name}/{file}"


def uncovered_from_covered_line_map(
    root: Path,
    data: dict[str, Any],
    package_name: str | None = None,
) -> dict[CoverageKey, UncoveredLine]:
    if not data:
        raise ValueError("coverage artifact has no covered-line metadata")
    result: dict[CoverageKey, UncoveredLine] = {}
    for artifact_file, file_data in coverage_map_files(data).items():
        file = repository_coverage_path(artifact_file, package_name)
        if not is_production_lua_path(file):
            continue
        covered = covered_line_set(file_data.get("covered_lines", file_data.get("covered")))
        lines = source_lines_for_file(root, file)
        for idx in sorted(candidate_lines(root, file, lines)):
            if idx in covered:
                continue
            text = lines[idx - 1]
            key = CoverageKey(file=file, line=idx, normalized_line_hash=normalized_source_hash(text))
            result[key] = UncoveredLine(key=key, text=normalize_source_line(text))
    return result


def all_production_lua_paths(root: Path) -> list[str]:
    paths: list[str] = []
    for base in ("packages", "libraries"):
        start = root / base
        if not start.exists():
            continue
        for path in start.rglob("*.lua"):
            if path.is_symlink():
                continue
            relpath = path.relative_to(root).as_posix()
            if is_production_lua_path(relpath):
                paths.append(relpath)
    return sorted(paths)


def uncovered_from_covered_sets(
    root: Path,
    covered_by_file: dict[str, set[int]],
) -> dict[CoverageKey, UncoveredLine]:
    if not covered_by_file:
        raise ValueError("coverage artifact has no covered-line metadata")
    result: dict[CoverageKey, UncoveredLine] = {}
    for file in all_production_lua_paths(root):
        lines = source_lines_for_file(root, file)
        covered = covered_by_file.get(file, set())
        for idx in sorted(candidate_lines(root, file, lines)):
            if idx in covered:
                continue
            text = lines[idx - 1]
            key = CoverageKey(file=file, line=idx, normalized_line_hash=normalized_source_hash(text))
            result[key] = UncoveredLine(key=key, text=normalize_source_line(text))
    return result


def canonical_coverage_artifact(
    root: Path,
    covered_by_file: dict[str, set[int]],
) -> dict[str, Any]:
    if not covered_by_file:
        raise ValueError("coverage artifact has no covered-line metadata")
    files: list[dict[str, Any]] = []
    for file in all_production_lua_paths(root):
        lines = source_lines_for_file(root, file)
        covered = covered_by_file.get(file, set())
        coverable_lines = [
            {
                "line": idx,
                "normalized_line_hash": normalized_source_hash(lines[idx - 1]),
                "text": normalize_source_line(lines[idx - 1]),
                "covered": idx in covered,
            }
            for idx in sorted(candidate_lines(root, file, lines))
        ]
        if coverable_lines:
            files.append({"file": file, "coverable_lines": coverable_lines})
    return {"schema": "fkst.lua.coverage.v1", "files": files}


def covered_sets_from_artifact(path: Path, package_name: str | None = None) -> dict[str, set[int]]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("coverage artifact must be a JSON object")
    result: dict[str, set[int]] = {}
    for artifact_file, file_data in coverage_map_files(data).items():
        file = repository_coverage_path(artifact_file, package_name)
        if not is_production_lua_path(file):
            continue
        result.setdefault(file, set()).update(
            covered_line_set(file_data.get("covered_lines", file_data.get("covered")))
        )
    return result


def merge_covered_sets(artifacts: list[tuple[Path, str | None]]) -> dict[str, set[int]]:
    covered_by_file: dict[str, set[int]] = {}
    for artifact, package_name in artifacts:
        for file, covered in covered_sets_from_artifact(artifact, package_name).items():
            covered_by_file.setdefault(file, set()).update(covered)
    return covered_by_file


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


def uncovered_from_artifact(
    path: Path,
    root: Path | None = None,
    package_name: str | None = None,
) -> dict[CoverageKey, UncoveredLine]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ValueError("coverage artifact must be a JSON object")
    result: dict[CoverageKey, UncoveredLine] = {}
    files = coverage_files(data)
    top_level_missing = line_entries(data.get("missing_lines", data.get("uncovered_lines")))
    if not files and not top_level_missing:
        if root is None:
            raise ValueError("coverage artifact has no engine-authored Lua line metadata")
        mapped = uncovered_from_covered_line_map(root, data, package_name)
        if not mapped and data:
            return {}
        if not mapped and not data:
            raise ValueError("coverage artifact has no engine-authored Lua line metadata")
        return mapped
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


def is_safe_base_ref(ref: str) -> bool:
    return ref not in {"", "HEAD"} and ".." not in ref and BASE_REF_RE.fullmatch(ref) is not None


def git_ref_exists(root: Path, ref: str) -> bool:
    return subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", "--end-of-options", ref],
        cwd=root,
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0


def selected_base_ref(root: Path) -> str | None:
    for env_name in ("FKST_LUA_COVERAGE_BASE_REF", "GITHUB_BASE_REF"):
        value = os.environ.get(env_name)
        if value:
            if not is_safe_base_ref(value):
                return None
            remote = f"origin/{value}"
            return remote if git_ref_exists(root, remote) else value
    for ref in ("origin/integration", "integration"):
        if git_ref_exists(root, ref):
            return ref
    return None


def allowlist_at_base(root: Path, base_ref: str) -> tuple[str, set[CoverageKey] | None]:
    try:
        git = lambda args, **kwargs: subprocess.run(["git", *args], cwd=root, check=False, **kwargs)
        if not is_safe_base_ref(base_ref):
            return "unresolved", None
        if not git_ref_exists(root, base_ref):
            return "unresolved", None
        base = git(["merge-base", "HEAD", base_ref], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        base_commit = base.stdout.strip()
        if base.returncode != 0 or base_commit == "":
            return "unresolved", None
        base_allowlist = base_commit + ":" + ALLOWLIST
        if git(["cat-file", "-e", base_allowlist], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            return "absent", None
        shown = git(["show", base_allowlist], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if shown.returncode != 0:
            return "unresolved", None
        tmp_file = tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            prefix="fkst-coverage-base.",
            suffix=".allowlist",
            delete=False,
        )
        tmp = Path(tmp_file.name)
        try:
            tmp_file.write(shown.stdout)
            tmp_file.close()
            return "present", load_allowlist(tmp)
        finally:
            tmp_file.close()
            tmp.unlink(missing_ok=True)
    except Exception:
        return "unresolved", None


def required_flag_at_base(root: Path, base_ref: str) -> str:
    try:
        git = lambda args, **kwargs: subprocess.run(["git", *args], cwd=root, check=False, **kwargs)
        if not is_safe_base_ref(base_ref):
            return "unresolved"
        if not git_ref_exists(root, base_ref):
            return "unresolved"
        base = git(["merge-base", "HEAD", base_ref], text=True, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        base_commit = base.stdout.strip()
        if base.returncode != 0 or base_commit == "":
            return "unresolved"
        base_flag = base_commit + ":" + REQUIRED_FLAG
        if git(["cat-file", "-e", base_flag], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            return "absent"
        return "present"
    except Exception:
        return "unresolved"


def content_key(key: CoverageKey) -> tuple[str, str]:
    """Identity used to compare allowlist/uncovered lines by CONTENT, not position.

    The allowlist still records `line` for readability and regeneration, but the
    ratchet matches on (file, normalized_line_hash) so that inserting or deleting
    lines elsewhere in a file — which shifts an already-uncovered, already-allowlisted
    line to a new line number with identical content — is recognized as the SAME
    uncovered line, not a new one. Matching is count-aware (a multiset), so N
    identical-content uncovered lines still require N allowlist entries and an
    (N+1)th novel duplicate is still flagged; a covered line becoming uncovered is
    still caught because its content is absent from the allowlist.
    """
    return (key.file, key.normalized_line_hash)


def ratchet_messages(
    uncovered: dict[CoverageKey, UncoveredLine],
    allowlist: set[CoverageKey],
    base_allowlist: set[CoverageKey] | None = None,
    base_ref: str = "base",
) -> list[str]:
    messages: list[str] = []
    allow_counts = Counter(content_key(k) for k in allowlist)
    seen: Counter = Counter()
    for key in sorted(uncovered):
        ck = content_key(key)
        seen[ck] += 1
        if seen[ck] > allow_counts.get(ck, 0):
            messages.append(
                f"{uncovered[key].label()} is an uncovered production Lua line not in {ALLOWLIST}"
            )
    if base_allowlist is not None:
        base_counts = Counter(content_key(k) for k in base_allowlist)
        grown: Counter = Counter()
        for key in sorted(allowlist):
            ck = content_key(key)
            grown[ck] += 1
            if grown[ck] > base_counts.get(ck, 0):
                messages.append(f"{key.label()} grows {ALLOWLIST} relative to {base_ref}; cover the line instead")
    return messages


def stale_allowlist_messages(
    uncovered: dict[CoverageKey, UncoveredLine],
    allowlist: set[CoverageKey],
) -> list[str]:
    uncovered_counts = Counter(content_key(k) for k in uncovered)
    consumed: Counter = Counter()
    messages: list[str] = []
    for key in sorted(allowlist):
        ck = content_key(key)
        consumed[ck] += 1
        if consumed[ck] > uncovered_counts.get(ck, 0):
            messages.append(f"{key.label()} is no longer uncovered; prune the stale entry from {ALLOWLIST}")
    return messages


def allowlist_entry(key: CoverageKey) -> dict[str, Any]:
    return {
        "file": key.file,
        "line": key.line,
        "normalized_line_hash": key.normalized_line_hash,
        "reason": "baseline",
    }


def allowlist_text(uncovered: dict[CoverageKey, UncoveredLine]) -> str:
    lines = [
        json.dumps(allowlist_entry(key), separators=(",", ":"), ensure_ascii=False)
        for key in sorted(uncovered)
    ]
    if not lines:
        return ""
    return "\n".join(lines) + "\n"


def write_uncovered_baseline(
    uncovered: dict[CoverageKey, UncoveredLine],
    allowlist_path: Path,
) -> int:
    allowlist_path.parent.mkdir(parents=True, exist_ok=True)
    allowlist_path.write_text(allowlist_text(uncovered), encoding="utf-8")
    return len(uncovered)


def write_current_uncovered(
    coverage_json: Path,
    allowlist_path: Path,
    root: Path | None = None,
    package_name: str | None = None,
) -> int:
    uncovered = uncovered_from_artifact(coverage_json, root, package_name)
    return write_uncovered_baseline(uncovered, allowlist_path)


def write_current_uncovered_from_covered_sets(
    covered_by_file: dict[str, set[int]],
    allowlist_path: Path,
    root: Path,
) -> int:
    uncovered = uncovered_from_covered_sets(root, covered_by_file)
    return write_uncovered_baseline(uncovered, allowlist_path)


def write_canonical_coverage_json(
    covered_by_file: dict[str, set[int]],
    output_path: Path,
    root: Path,
) -> int:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    artifact = canonical_coverage_artifact(root, covered_by_file)
    output_path.write_text(json.dumps(artifact, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")
    return len(artifact["files"])


def parse_covered_json_arg(value: str) -> tuple[Path, str | None]:
    if "=" not in value:
        return Path(value), None
    package_name, path = value.split("=", 1)
    if package_name == "" or path == "":
        raise ValueError("coverage input entries must be PATH or PACKAGE=PATH")
    return Path(path), package_name


def warn_disabled(message: str) -> None:
    print(f"warning: Lua coverage ratchet not enabled (no {REQUIRED_FLAG}); {message}", file=sys.stderr)


def warn_deferred(message: str) -> None:
    print(f"warning: Lua coverage ratchet deferred; {message}", file=sys.stderr)


def repository_messages_for_uncovered(
    root: Path,
    uncovered: dict[CoverageKey, UncoveredLine],
) -> list[str]:
    required = (root / REQUIRED_FLAG).exists()
    if not required:
        if uncovered:
            warn_disabled(f"{len(uncovered)} uncovered line(s) would block once enabled")
        return []
    try:
        allowlist = load_allowlist(root / ALLOWLIST)
        base_ref = selected_base_ref(root)
        if base_ref is None:
            base_status, base_allowlist = "unresolved", None
        else:
            base_required = required_flag_at_base(root, base_ref)
            if base_required == "absent":
                base_status, base_allowlist = "absent", None
            elif base_required == "present":
                base_status, base_allowlist = allowlist_at_base(root, base_ref)
            else:
                base_status, base_allowlist = "unresolved", None
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        return [f"invalid Lua coverage ratchet input: {exc}"]
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve coverage base allowlist to enforce shrink-only ratchet; ensure CI provides GITHUB_BASE_REF or FKST_LUA_COVERAGE_BASE_REF")
    messages.extend(ratchet_messages(uncovered, allowlist, base_allowlist, base_ref or "base"))
    for message in stale_allowlist_messages(uncovered, allowlist):
        print(f"warning: {message}", file=sys.stderr)
    return messages


def repository_messages(root: Path) -> list[str]:
    path = artifact_path(root)
    required = (root / REQUIRED_FLAG).exists()
    # Coverage is advisory (reference, not enforced): absent REQUIRED_FLAG reports
    # uncovered lines without blocking, and removing the flag is allowed. The
    # enforce path is retained behind REQUIRED_FLAG for any repo that opts back in.
    if path is None:
        if not required:
            warn_disabled("coverage artifact is absent")
            return []
        warn_deferred(
            f"{REQUIRED_FLAG} is present but no coverage artifact is available; "
            "set FKST_LUA_COVERAGE_JSON or produce a standard coverage artifact to enforce it"
        )
        return []
    if not path.exists():
        if not required:
            warn_disabled(f"coverage artifact is missing: {path}")
            return []
        return [f"Lua coverage artifact does not exist: {path}"]
    try:
        uncovered = uncovered_from_artifact(path, root)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        if not required:
            warn_disabled(f"coverage artifact would not parse once enabled: {exc}")
            return []
        return [f"invalid Lua coverage ratchet input: {exc}"]
    return repository_messages_for_uncovered(root, uncovered)


def cli(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--coverage-json",
        type=Path,
        help="engine-authored coverage.json to read; defaults to FKST_LUA_COVERAGE_JSON or the standard artifact path",
    )
    parser.add_argument(
        "--write-current-uncovered",
        type=Path,
        metavar="ALLOWLIST_PATH",
        help="write the current uncovered production-Lua line baseline as JSONL",
    )
    parser.add_argument(
        "--package-name",
        help="map package-root-relative coverage paths to packages/<name>/... while leaving libraries/forge/... unchanged",
    )
    args = parser.parse_args(argv)

    if args.write_current_uncovered is None:
        try:
            messages = repository_messages(Path.cwd())
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            print(f"error: invalid Lua coverage ratchet input: {exc}", file=sys.stderr)
            return 1
        for message in messages:
            print(message)
        return 1 if messages else 0

    try:
        coverage_json = args.coverage_json or artifact_path(Path.cwd())
        if coverage_json is None:
            print("error: Lua coverage artifact was not found", file=sys.stderr)
            return 1
        count = write_current_uncovered(
            coverage_json,
            args.write_current_uncovered,
            Path.cwd(),
            args.package_name,
        )
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"error: could not write current uncovered allowlist: {exc}", file=sys.stderr)
        return 1
    print(f"wrote {count} uncovered line(s) to {args.write_current_uncovered}")
    return 0


if __name__ == "__main__":
    raise SystemExit(cli())
