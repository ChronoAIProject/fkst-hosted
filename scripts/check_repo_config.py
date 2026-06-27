#!/usr/bin/env python3
"""Configuration for the published repository conformance seam."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path


OWN_REPO_ROOT = Path(__file__).resolve().parents[1]
GENERIC_RATCHETS = (
    "line/file limits",
    "test shape/helper reachability",
    "fkst package layout",
    "gh/git adapter boundary",
    "dedup",
    "producer-liveness",
    "ingress",
    "monotone-gate",
    "content-truncation",
    "coverage",
    "saga-head/free-form-saga",
    "namespaced-queue",
    "permission-control",
)
LIBRARY_B_SPECIFIC_RATCHETS = (
    "dogfood_boundary",
    "std_dependency_model",
    "devloop product knowledge",
    "github-devloop saga-split guards",
    "github-devloop-intake-default surface guard",
)


@dataclass(frozen=True)
class CheckRepoConfig:
    project_root: Path
    allowlist_dir: Path | None
    platform_root: Path | None = None
    own_repo_root: Path = OWN_REPO_ROOT

    @property
    def is_own_repo(self) -> bool:
        return same_path(self.project_root, self.own_repo_root)


def same_path(left: Path, right: Path) -> bool:
    return left.resolve() == right.resolve()


def resolve_dir(path: str | Path) -> Path:
    return Path(path).expanduser().resolve()


def default_project_root() -> Path:
    return OWN_REPO_ROOT


def parse_args(argv: list[str] | None = None) -> CheckRepoConfig:
    parser = argparse.ArgumentParser(description="Run fkst package repository conformance ratchets.")
    parser.add_argument(
        "--project-root",
        type=resolve_dir,
        default=default_project_root(),
        help="repository tree to check; defaults to this fkst-packages checkout",
    )
    parser.add_argument(
        "--allowlist-dir",
        type=resolve_dir,
        help="directory containing *.allowlist waiver files; defaults to <project-root>/migration",
    )
    parser.add_argument(
        "--platform-root",
        type=resolve_dir,
        help="fkst-packages checkout to include for host-owned integration coverage edges",
    )
    args = parser.parse_args(argv)
    return CheckRepoConfig(project_root=args.project_root, allowlist_dir=args.allowlist_dir, platform_root=args.platform_root)


def package_roots(project_root: Path) -> list[Path]:
    packages = project_root / "packages"
    if same_path(project_root, OWN_REPO_ROOT):
        return [packages]
    local_packages = project_root / ".fkst" / "local-packages"
    roots = [packages, local_packages]
    existing = [root for root in roots if root.exists()]
    return existing if existing else [packages]


def package_root(project_root: Path) -> Path:
    return package_roots(project_root)[0]


def allowlist_path(root: Path, allowlist_dir: Path | None, relpath: str) -> Path:
    if allowlist_dir is None:
        return root / relpath
    return allowlist_dir / Path(relpath).name
