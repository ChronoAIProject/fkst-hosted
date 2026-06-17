"""Static guards for necessary package-owned file-watch ingress raisers.

The bumped substrate rejects consumed queues without a producer. A file-watch
ingress is allowed here only as the least-privilege package source for a queue
that a department consumes and no package department or other raiser produces.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Callable

RAISER_PRODUCES_RE = re.compile(r"\bproduces\s*=\s*(?P<quote>[\"'])(?P<queue>[A-Za-z0-9_]+)(?P=quote)")
RAISER_FILE_WATCH_RE = re.compile(r"\btype\s*=\s*(?P<quote>[\"'])file_watch(?P=quote)")
RAISER_GLOB_RE = re.compile(r"\bglob\s*=\s*(?P<quote>[\"'])(?P<glob>[^\"']+)(?P=quote)")
SPEC_FIELD_RE = re.compile(r"\b(?P<field>consumes|produces)\s*=\s*\{(?P<body>.*?)\}", re.S)
LITERAL_QUEUE_RE = re.compile(r"(?P<quote>[\"'])(?P<queue>[A-Za-z0-9_.-]+)(?P=quote)")


def queue_ingress_segment(package: str, queue: str) -> str:
    prefix = package.split("-", 1)[0].replace("-", "_") + "_"
    if queue.startswith(prefix):
        queue = queue[len(prefix) :]
    return queue.replace("_", "-")


def spec_queues(source: str, field: str) -> set[str]:
    queues: set[str] = set()
    for match in SPEC_FIELD_RE.finditer(source):
        if match.group("field") != field:
            continue
        for queue in LITERAL_QUEUE_RE.finditer(match.group("body")):
            queues.add(queue.group("queue"))
    return queues


def package_consumed_queues(package_root: Path, read_text: Callable[[Path], str]) -> set[str]:
    queues: set[str] = set()
    for path in sorted((package_root / "departments").glob("**/*.lua")):
        if path.is_file():
            queues.update(spec_queues(read_text(path), "consumes"))
    return queues


def package_internal_produced_queues(
    package_root: Path,
    current_ingress: Path,
    read_text: Callable[[Path], str],
) -> set[str]:
    queues: set[str] = set()
    for path in sorted((package_root / "departments").glob("**/*.lua")):
        if path.is_file():
            queues.update(spec_queues(read_text(path), "produces"))
    for path in sorted((package_root / "raisers").glob("*.lua")):
        if path.is_file() and path != current_ingress:
            produces = RAISER_PRODUCES_RE.search(read_text(path))
            if produces is not None:
                queues.add(produces.group("queue"))
    return queues


def scoped_file_watch_ingress_violation(
    root: Path,
    path: Path,
    source: str,
    read_text: Callable[[Path], str],
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
    package_root = path.parent.parent
    consumed = package_consumed_queues(package_root, read_text)
    produced = package_internal_produced_queues(package_root, path, read_text)
    if queue not in consumed:
        return f"{rel(root, path)} file-watch ingress for queue `{queue}` must target a package-consumed queue"
    if queue in produced:
        return f"{rel(root, path)} file-watch ingress for queue `{queue}` duplicates an internal package producer"
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
        violation = scoped_file_watch_ingress_violation(root, path, read_text(path), read_text, rel)
        if violation is not None:
            messages.append(violation)
    return messages
