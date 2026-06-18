"""Production Lua duplicate-function ratchet."""

from __future__ import annotations

import hashlib
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


ALLOWLIST = "migration/code-dedup.allowlist"
MIN_NORMALIZED_BODY_CHARS = 60
FUNCTION_RE = re.compile(
    r"^(?P<indent>[ \t]*)(?:local\s+)?function\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*(?:\s*[.:]\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*\("
)
SAME_INDENT_END_RE = re.compile(r"^(?P<indent>[ \t]*)end(?:\s*(?:--.*)?)?$")


@dataclass(frozen=True, order=True)
class DedupEntry:
    name: str
    body_hash: str
    files: tuple[str, ...]

    @classmethod
    def parse(cls, line: str) -> "DedupEntry":
        parts = line.split()
        if len(parts) < 3:
            raise ValueError(f"invalid {ALLOWLIST} line: {line}")
        name, body_hash, *files = parts
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            raise ValueError(f"invalid {ALLOWLIST} function name: {line}")
        if not re.fullmatch(r"[0-9a-f]{32}", body_hash):
            raise ValueError(f"invalid {ALLOWLIST} body hash: {line}")
        if len(files) < 2 or len(set(files)) != len(files):
            raise ValueError(f"invalid {ALLOWLIST} file list: {line}")
        return cls(name=name, body_hash=body_hash, files=tuple(sorted(files)))

    def allowlist_line(self) -> str:
        return " ".join((self.name, self.body_hash, *self.files))

    def label(self) -> str:
        return f"{self.name} {self.body_hash} {' '.join(self.files)}"


@dataclass(frozen=True)
class FunctionBody:
    name: str
    path: str
    normalized_body: str

    @property
    def key(self) -> tuple[str, str]:
        digest = hashlib.md5(self.normalized_body.encode("utf-8")).hexdigest()
        return self.name, digest


def is_production_lua_path(path: Path) -> bool:
    if path.suffix != ".lua":
        return False
    parts = path.parts
    if "tests" in parts:
        return False
    return not path.name.endswith(("_test.lua", "_helpers.lua", "_fake.lua"))


def sources(root: Path, packages: Path, read_text, rel) -> dict[str, str]:
    paths: list[Path] = []
    if packages.exists():
        paths.extend(path for path in sorted(packages.rglob("*.lua")) if path.is_file())
    std = root / "std"
    if std.exists():
        paths.extend(path for path in sorted(std.rglob("*.lua")) if path.is_file())
    return {rel(root, path): read_text(path) for path in paths if is_production_lua_path(path)}


def function_basename(name: str) -> str:
    return re.split(r"[.:]", name.replace(" ", ""))[-1]


def code_without_comments_and_strings(text: str) -> str:
    chars = list(text)
    cursor = 0
    while cursor < len(text):
        if text.startswith("--", cursor):
            newline = text.find("\n", cursor)
            end = len(text) if newline == -1 else newline
            _mask(chars, cursor, end)
            cursor = end
            continue
        char = text[cursor]
        if char in ("'", '"'):
            end = _quoted_string_end(text, cursor)
            _mask(chars, cursor, end)
            cursor = end
            continue
        cursor += 1
    return "".join(chars)


def _mask(chars: list[str], start: int, end: int) -> None:
    for index in range(start, end):
        if chars[index] != "\n":
            chars[index] = " "


def _quoted_string_end(text: str, start: int) -> int:
    quote = text[start]
    cursor = start + 1
    while cursor < len(text):
        if text[cursor] == "\\":
            cursor += 2
            continue
        if text[cursor] == quote:
            return cursor + 1
        cursor += 1
    return len(text)


def normalized_body(lines: list[str]) -> str:
    return "\n".join(stripped for line in lines if (stripped := line.strip()))


def module_scope_functions(path: str, source: str) -> list[FunctionBody]:
    original_lines = source.splitlines()
    code_lines = code_without_comments_and_strings(source).splitlines()
    bodies: list[FunctionBody] = []
    index = 0
    while index < len(code_lines):
        line = code_lines[index]
        match = FUNCTION_RE.match(line)
        if match is not None:
            body_start = index + 1
            cursor = body_start
            while cursor < len(code_lines):
                end_match = SAME_INDENT_END_RE.match(code_lines[cursor])
                if end_match is not None and end_match.group("indent") == match.group("indent"):
                    body = normalized_body(original_lines[body_start:cursor])
                    if len(body) >= MIN_NORMALIZED_BODY_CHARS:
                        bodies.append(FunctionBody(function_basename(match.group("name")), path, body))
                    break
                cursor += 1
        index += 1
    return bodies


def duplicate_groups(sources_by_path: dict[str, str]) -> set[DedupEntry]:
    grouped: dict[tuple[str, str], set[str]] = {}
    for path, source in sorted(sources_by_path.items()):
        for body in module_scope_functions(path, source):
            grouped.setdefault(body.key, set()).add(path)

    entries: set[DedupEntry] = set()
    for (name, body_hash), files in grouped.items():
        if len(files) >= 2:
            entries.add(DedupEntry(name=name, body_hash=body_hash, files=tuple(sorted(files))))
    return entries


def load_allowlist(path: Path) -> set[DedupEntry]:
    if not path.exists():
        return set()
    entries: set[DedupEntry] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        stripped = raw.strip()
        if not stripped or stripped.startswith("#"):
            continue
        entries.add(DedupEntry.parse(stripped))
    return entries


def ratchet_messages(
    sources_by_path: dict[str, str],
    allowlist: set[DedupEntry],
    base_allowlist: set[DedupEntry] | None = None,
) -> list[str]:
    current = duplicate_groups(sources_by_path)
    messages: list[str] = []
    for entry in sorted(current - allowlist):
        messages.append(
            f"{entry.label()} is a cross-file byte-identical production function body not in the allowlist baseline"
        )
    for entry in sorted(allowlist - current):
        messages.append(
            f"{entry.label()} no longer matches a duplicate group in {ALLOWLIST}; prune the stale entry"
        )
    if base_allowlist is not None:
        for entry in sorted(allowlist - base_allowlist):
            messages.append(
                f"{entry.label()} grows code-dedup allowlist relative to dev; deduplicate instead"
            )
    return messages


def repository_messages(root: Path, packages: Path, read_text, rel) -> list[str]:
    source_map = sources(root, packages, read_text, rel)
    allowlist = load_allowlist(root / ALLOWLIST)
    base_status, base_allowlist = allowlist_at_dev_base(root)
    messages: list[str] = []
    if base_status == "unresolved":
        messages.append("cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref")
    messages.extend(ratchet_messages(source_map, allowlist, base_allowlist))
    return messages


def allowlist_at_dev_base(root: Path) -> tuple[str, set[DedupEntry] | None]:
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
        entries = {
            DedupEntry.parse(line.strip())
            for line in shown.stdout.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }
        return "present", entries
    except Exception:
        return "unresolved", None
