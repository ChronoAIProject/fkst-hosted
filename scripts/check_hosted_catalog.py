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

    errors.extend(validate_package_env(document))
    return errors


# Mirrors the control plane's `### Package Env` grammar. A manifest must not be
# able to express configuration a trigger author would be refused, and a
# malformed block published here would otherwise only surface at session start.
PACKAGE_NAME = re.compile(r"[A-Za-z0-9_.-]{1,64}")
ENV_KEY = re.compile(r"FKST_[A-Z0-9]+(_[A-Z0-9]+)*")
MAX_BLOCKS = 16
MAX_KEYS_PER_BLOCK = 32
MAX_ENTRIES_TOTAL = 64
MAX_VALUE_BYTES = 1024
MAX_KEY_BYTES = 64

# Names the platform sets for every session. A manifest setting one could
# redirect a session's identity or routing, so they are refused here exactly as
# the control plane refuses them in a trigger.
PLATFORM_OWNED = {
    "FKST_GITHUB_AUTHORIZED_LOGINS",
    "FKST_GITHUB_BOT_LOGIN",
    "FKST_GITHUB_CLAIM_MODE",
    "FKST_GITHUB_PROXY_POLL_LABEL_PREFIX",
    "FKST_GITHUB_REPO",
    "FKST_GITHUB_WRITE",
    "FKST_SESSION_CREATOR",
    "FKST_SESSION_CREDS_DIR",
    "FKST_SESSION_DELIVERY_GRANTS",
    "FKST_SESSION_ID",
    "FKST_SESSION_PACKAGE_ENV_JSON",
    "FKST_SESSION_PACKAGE_ROOTS",
    "FKST_SESSION_WORK_LABEL",
    "FKST_SESSION_WORK_LABEL_MAP_JSON",
    "FKST_TRIGGER_ISSUE",
    "FKST_WORK_LABEL_NAMESPACE",
}


def validate_package_env(document: dict) -> list[str]:
    """Validate the OPTIONAL `packageEnv` block. Absent is valid and common."""
    package_env = document.get("packageEnv")
    if package_env is None:
        return []

    errors: list[str] = []
    if not isinstance(package_env, dict):
        return [f"{DEFAULT_MANIFEST}: packageEnv must be an object"]
    if len(package_env) > MAX_BLOCKS:
        errors.append(f"{DEFAULT_MANIFEST}: packageEnv declares more than {MAX_BLOCKS} packages")

    owner: dict[str, str] = {}
    total = 0
    for package, keys in sorted(package_env.items()):
        site = f"{DEFAULT_MANIFEST}: packageEnv[{package!r}]"
        if PACKAGE_NAME.fullmatch(package) is None:
            errors.append(f"{site} is not a valid package name")
            continue
        if not isinstance(keys, dict):
            errors.append(f"{site} must be an object of KEY to value")
            continue
        if len(keys) > MAX_KEYS_PER_BLOCK:
            errors.append(f"{site} sets more than {MAX_KEYS_PER_BLOCK} keys")
        for key, value in sorted(keys.items()):
            total += 1
            if ENV_KEY.fullmatch(key) is None or len(key) > MAX_KEY_BYTES:
                errors.append(f"{site} sets an invalid key {key!r}")
                continue
            if key in PLATFORM_OWNED:
                errors.append(f"{site} sets {key}, which the platform owns")
                continue
            if not isinstance(value, str):
                errors.append(f"{site} sets {key} to a non-string value")
                continue
            if len(value.encode("utf-8")) > MAX_VALUE_BYTES:
                errors.append(f"{site} sets {key} to a value over {MAX_VALUE_BYTES} bytes")
            if any(ch.isspace() and ch != " " for ch in value):
                errors.append(f"{site} sets {key} to a value containing a control character")
            # One flat environment reaches the pod, so the same key under two
            # packages cannot both apply -- one would silently win.
            if key in owner and owner[key] != package:
                errors.append(f"{site} sets {key}, already set by {owner[key]!r}")
            owner[key] = package

    if total > MAX_ENTRIES_TOTAL:
        errors.append(f"{DEFAULT_MANIFEST}: packageEnv sets more than {MAX_ENTRIES_TOTAL} values")

    # Advisory only: a package named here that the catalog does not ship is
    # simply inert, exactly as it is at runtime. Reporting it as an error would
    # stop a manifest from pre-configuring a package it does not itself bundle.
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
