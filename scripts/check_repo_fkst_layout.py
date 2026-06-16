#!/usr/bin/env python3
"""Guard the committed/runtime split for the .fkst layout."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


REQUIRED_GITIGNORE_LINES = (
    "/.fkst/packages",
    "/.fkst/local-packages",
    "/.fkst/run/",
    "/.fkst/env",
)
RUNTIME_TRACKED_PREFIXES = (
    ".fkst/packages",
    ".fkst/local-packages",
    ".fkst/run/",
)
LEGACY_TRACKED_PATHS = (
    ".fkst/runtime",
    ".fkst/durable",
    ".fkst/substrate-src",
    ".fkst/board-cache.json",
)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def git_ls_files(root: Path) -> list[str]:
    return git_file_list(root, ["ls-files"])


def git_deleted_files(root: Path) -> set[str]:
    return set(git_file_list(root, ["ls-files", "--deleted"]))


def git_file_list(root: Path, args: list[str]) -> list[str]:
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "git ls-files failed")
    return result.stdout.splitlines()


def is_path_or_child(path: str, prefix: str) -> bool:
    if prefix.endswith("/"):
        return path.startswith(prefix)
    return path == prefix or path.startswith(prefix + "/")


def gitignore_lines(root: Path) -> list[str]:
    path = root / ".gitignore"
    if not path.exists():
        return []
    return [line.strip() for line in path.read_text(encoding="utf-8").splitlines()]


def check_layout(root: Path) -> list[str]:
    violations: list[str] = []
    tracked = git_ls_files(root)
    pending_removal = git_deleted_files(root)

    for path in tracked:
        if path in pending_removal:
            continue
        for prefix in RUNTIME_TRACKED_PREFIXES:
            if is_path_or_child(path, prefix):
                violations.append(f"FKST-LAYOUT: {prefix} must be runtime-only, not tracked: {path}")
        for legacy in LEGACY_TRACKED_PATHS:
            if is_path_or_child(path, legacy):
                violations.append(f"FKST-LAYOUT: legacy generated path is tracked: {path}")

    if (root / ".fkst-substrate-ref").exists():
        violations.append("FKST-LAYOUT: root .fkst-substrate-ref is forbidden; use .fkst/substrate-ref")

    lines = gitignore_lines(root)
    if any(line in ("/.fkst/", ".fkst/", "/.fkst", ".fkst") for line in lines):
        violations.append("FKST-LAYOUT: .gitignore must not blanket-ignore /.fkst/")
    for required in REQUIRED_GITIGNORE_LINES:
        if required not in lines:
            violations.append(f"FKST-LAYOUT: missing required .gitignore line: {required}")

    if not (root / ".fkst" / "substrate-ref").is_file():
        violations.append("FKST-LAYOUT: missing .fkst/substrate-ref")

    return violations


def main() -> int:
    try:
        violations = check_layout(repo_root())
    except RuntimeError as exc:
        print(f"repository layout check failed: {exc}", file=sys.stderr)
        return 1
    if violations:
        print("repository layout check failed:", file=sys.stderr)
        for violation in violations:
            print(f"  {violation}", file=sys.stderr)
        return 1
    print("OK: fkst layout checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
