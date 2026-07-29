#!/usr/bin/env python3
"""Validate the self-owned FKST Cloud package catalog manifest."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


CATALOG_PREFIX = "ChronoAIProject/fkst-hosted@packages:"
DEFAULT_MANIFEST = Path("manifests/default-workflows.json")
PACKAGE_PATH = re.compile(r"packages/[a-z0-9](?:[a-z0-9-]*[a-z0-9])?")


def validate_catalog(root: Path) -> list[str]:
    manifest_path = root / DEFAULT_MANIFEST
    try:
        document = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return [f"{DEFAULT_MANIFEST}: cannot load JSON: {exc}"]

    errors: list[str] = []
    if not isinstance(document, dict):
        return [f"{DEFAULT_MANIFEST}: root must be a JSON object"]
    if document.get("schemaVersion") != 1:
        errors.append(f"{DEFAULT_MANIFEST}: schemaVersion must equal 1")

    packages = document.get("packages")
    if not isinstance(packages, list) or not 1 <= len(packages) <= 64:
        errors.append(f"{DEFAULT_MANIFEST}: packages must contain 1 to 64 references")
        return errors

    seen: set[str] = set()
    for index, package_ref in enumerate(packages):
        site = f"{DEFAULT_MANIFEST}: packages[{index}]"
        if not isinstance(package_ref, str) or not package_ref.startswith(CATALOG_PREFIX):
            errors.append(f"{site} must start with {CATALOG_PREFIX}")
            continue
        if package_ref in seen:
            errors.append(f"{site} duplicates {package_ref}")
            continue
        seen.add(package_ref)

        relative_path = package_ref.removeprefix(CATALOG_PREFIX)
        if PACKAGE_PATH.fullmatch(relative_path) is None:
            errors.append(f"{site} has invalid package path {relative_path!r}")
            continue
        if not (root / relative_path / "fkst.toml").is_file():
            errors.append(f"{site} does not resolve to {relative_path}/fkst.toml")

    return errors


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    errors = validate_catalog(root)
    for error in errors:
        print(f"HOSTED-CATALOG: {error}", file=sys.stderr)
    if errors:
        return 1
    print("OK: hosted package catalog manifest is self-owned and resolvable")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
