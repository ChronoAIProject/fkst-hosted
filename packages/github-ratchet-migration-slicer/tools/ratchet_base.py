"""Shared dev-base resolver for shrink-only ratchets."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path


def _git(root: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=root,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )


def resolve_dev_ref(root: Path) -> str | None:
    refs: list[str] = []
    override = os.environ.get("FKST_RATCHET_DEV_REF")
    if override:
        refs.append(override)
    refs.extend(("refs/remotes/origin/dev", "origin/dev", "refs/heads/dev", "dev"))

    for ref in refs:
        result = _git(root, ["rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"])
        commit = result.stdout.strip()
        if result.returncode == 0 and commit:
            return commit
    return None


def resolve_dev_merge_base(root: Path) -> str | None:
    commit = resolve_dev_ref(root)
    if commit is None:
        return None
    result = _git(root, ["merge-base", "HEAD", commit])
    base = result.stdout.strip()
    if result.returncode != 0 or not base:
        return None
    return base


def show_file_at(root: Path, commit: str, path: str) -> str | None:
    result = _git(root, ["show", f"{commit}:{path}"])
    if result.returncode != 0:
        return None
    return result.stdout


def file_at_base(root: Path, path: str) -> tuple[str, str | None]:
    base_commit = resolve_dev_merge_base(root)
    if base_commit is None:
        return "unresolved", None

    exists = _git(root, ["cat-file", "-e", f"{base_commit}:{path}"])
    if exists.returncode != 0:
        return "absent", None

    result = _git(root, ["show", f"{base_commit}:{path}"])
    if result.returncode != 0:
        return "unresolved", None
    return "present", result.stdout
