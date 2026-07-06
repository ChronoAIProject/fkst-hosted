#!/usr/bin/env python3
"""Dead-letter topology guard for reliable-consuming packages."""

from __future__ import annotations

from pathlib import Path
from typing import Callable

import check_repo_config
import check_repo_ingress


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def package_dirs(root: Path) -> list[Path]:
    return [
        path
        for packages in check_repo_config.package_roots(root)
        if packages.exists()
        for path in sorted(packages.iterdir())
        if path.is_dir()
    ]


def repository_messages(root: Path, read_text: Callable[[Path], str] = _read_text) -> list[str]:
    messages: list[str] = []
    for pkg in package_dirs(root):
        reliable: set[str] = set()
        has_dead_letter = False
        for path in sorted((pkg / "departments").glob("*/main.lua")):
            if not path.is_file():
                continue
            source = read_text(path)
            consumes = check_repo_ingress.spec_queues(source, "consumes")
            ephemeral = check_repo_ingress.spec_queues(source, "ephemeral")
            if "dead_letter" in consumes:
                has_dead_letter = True
            reliable.update(consumes - ephemeral - {"dead_letter"})
        if reliable and not has_dead_letter:
            messages.append(
                f"packages/{pkg.name} consumes reliable queues but has no department consuming `dead_letter`: {', '.join(sorted(reliable))}"
            )
    return messages
