"""Static guards for package-owned file-watch ingress raisers."""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable

RAISER_PRODUCES_RE = re.compile(r"\bproduces\s*=\s*(?P<quote>[\"'])(?P<queue>[A-Za-z0-9_]+)(?P=quote)")
RAISER_FILE_WATCH_RE = re.compile(r"\btype\s*=\s*(?P<quote>[\"'])file_watch(?P=quote)")
RAISER_GLOB_RE = re.compile(r"\bglob\s*=\s*(?P<quote>[\"'])(?P<glob>[^\"']+)(?P=quote)")


def queue_ingress_segment(package: str, queue: str) -> str:
    prefix = package.split("-", 1)[0].replace("-", "_") + "_"
    if queue.startswith(prefix):
        queue = queue[len(prefix) :]
    return queue.replace("_", "-")


def scoped_file_watch_ingress_violation(
    root: Path,
    path: Path,
    source: str,
    rel: Callable[[Path, Path], str],
) -> str | None:
    if not path.name.endswith("_ingress.lua"):
        return None
    if RAISER_FILE_WATCH_RE.search(source) is None:
        return None

    produces = RAISER_PRODUCES_RE.search(source)
    glob = RAISER_GLOB_RE.search(source)
    if produces is None or glob is None:
        return f"{rel(root, path)} file-watch ingress must declare literal `glob` and `produces` fields"

    package = path.parent.parent.name
    queue = produces.group("queue")
    expected = f".fkst/ingress/{package}/{queue_ingress_segment(package, queue)}/*.json"
    if glob.group("glob") != expected:
        return (
            f"{rel(root, path)} file-watch ingress for queue `{queue}` must be scoped to "
            f"`{expected}`, got `{glob.group('glob')}`"
        )
    return None


def scoped_file_watch_ingress_messages(
    root: Path,
    packages: Path,
    read_text: Callable[[Path], str],
    rel: Callable[[Path, Path], str],
) -> list[str]:
    if not packages.exists():
        return []
    messages: list[str] = []
    for path in sorted(packages.glob("*/raisers/*_ingress.lua")):
        if not path.is_file():
            continue
        violation = scoped_file_watch_ingress_violation(root, path, read_text(path), rel)
        if violation is not None:
            messages.append(violation)
    return messages
