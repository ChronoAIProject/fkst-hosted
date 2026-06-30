#!/usr/bin/env python3
"""Behavior tests for scripts/host_run.sh."""

from __future__ import annotations

import os
import json
import signal
import subprocess
import tempfile
import textwrap
import time
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class HostRunHarness:
    def __init__(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.packages_host = self.root / "packages-host"
        self.substrate_host = self.root / "substrate-host"
        self.website_host = self.root / "website-host"
        self.platform = self.root / "platform"
        self.durable = self.root / "durable"
        self.runtime = self.root / "runtime"
        for pkg in ("github-proxy", "consensus"):
            (self.platform / "packages" / pkg).mkdir(parents=True, exist_ok=True)
            (self.platform / "packages" / pkg / "fkst.toml").write_text(
                f'kind = "package"\nname = "{pkg}"\n',
                encoding="utf-8",
            )
            (self.packages_host / "packages" / pkg).mkdir(parents=True, exist_ok=True)
            (self.packages_host / "packages" / pkg / "fkst.toml").write_text(
                f'kind = "package"\nname = "{pkg}"\n',
                encoding="utf-8",
            )
        (self.packages_host / "packages" / "autochrono").mkdir(parents=True)
        (self.packages_host / "packages" / "autochrono" / "fkst.toml").write_text(
            'kind = "package"\nname = "autochrono"\n',
            encoding="utf-8",
        )
        (self.website_host / ".fkst" / "local-packages" / "site-board").mkdir(parents=True)
        self.substrate_host.mkdir()

    def close(self) -> None:
        self.tmp.cleanup()

    def run_helper(self, body: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", "-c", body],
            cwd=REPO_ROOT,
            env=os.environ.copy(),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def package_roots(self, command_args: list[str]) -> subprocess.CompletedProcess[str]:
        quoted = " ".join(shell_quote(arg) for arg in command_args)
        return self.run_helper(
            textwrap.dedent(
                f"""\
                set -euo pipefail
                source scripts/host_run.sh
                host_run_parse_supervise_args {quoted}
                host_run_validate_shape
                host_run_build_package_roots
                host_run_print_package_roots
                """
            )
        )

    def write_external_sources_lock(self, entries: list[tuple[str, Path, str]], *, root: Path | None = None) -> None:
        target_root = root or self.website_host
        (target_root / "fkst.lock").write_text(
            "\n".join(
                textwrap.dedent(
                    f"""\
                    [[external_source]]
                    id = {json.dumps(source_id)}
                    git = {json.dumps(str(repo))}

                    [external_source.resolved]
                    rev = {json.dumps(rev)}
                    tree_sha256 = "sha256-test"
                    """
                )
                for source_id, repo, rev in entries
            )
            + "\n",
            encoding="utf-8",
        )

    def write_workspace_manifest(
        self,
        *,
        root: Path | None = None,
        workspace_units: list[str] | None = None,
        workspace_packages: list[str] | None = None,
        external_sources: list[tuple[str, Path, list[str]]] | None = None,
    ) -> None:
        target_root = root or self.website_host
        units = workspace_units or [".fkst/local-packages/*"]
        chunks = [f"[workspace]\nunits = {json.dumps(units)}\n"]
        for package in workspace_packages or []:
            chunks.append(
                textwrap.dedent(
                    f"""\
                    [[package]]
                    name = {json.dumps(package)}
                    source = "workspace"
                    version = "workspace"
                    """
                )
            )
        for source_id, repo, packages in external_sources or []:
            chunks.append(
                textwrap.dedent(
                    f"""\
                    [[external_sources]]
                    id = {json.dumps(source_id)}
                    git = {json.dumps(str(repo))}
                    packages = {json.dumps(packages)}
                    """
                )
            )
        (target_root / "fkst.workspace.toml").write_text("".join(chunks), encoding="utf-8")


def shell_quote(value: str | Path) -> str:
    text = str(value)
    return "'" + text.replace("'", "'\\''") + "'"


def run_argv(args: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def create_git_source(root: Path, name: str, files: dict[str, str]) -> tuple[Path, str]:
    repo = root / name
    repo.mkdir(parents=True)
    result = run_argv(["git", "init", "-q"], cwd=repo)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    for key, value in {
        "user.email": "host-run-test@example.invalid",
        "user.name": "Host Run Test",
    }.items():
        result = run_argv(["git", "config", key, value], cwd=repo)
        if result.returncode != 0:
            raise AssertionError(result.stderr)
    for rel, content in files.items():
        path = repo / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    result = run_argv(["git", "add", "."], cwd=repo)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    result = run_argv(["git", "commit", "-q", "-m", "seed"], cwd=repo)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    result = run_argv(["git", "rev-parse", "HEAD"], cwd=repo)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return repo, result.stdout.strip()


def pid_is_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def wait_for_dead(pid: int, timeout: float = 5.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not pid_is_alive(pid):
            return True
        time.sleep(0.05)
    return not pid_is_alive(pid)


def start_orphan_sleep(seconds: int = 60) -> int:
    result = subprocess.run(
        ["/bin/sh", "-c", f"sleep {seconds} >/dev/null 2>&1 & echo $!"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return int(result.stdout.strip())


def kill_if_alive(pid: int) -> None:
    if not pid_is_alive(pid):
        return
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    wait_for_dead(pid)


class HostRunTest(unittest.TestCase):
    def test_packages_host_uses_project_packages_for_host_packages(self) -> None:
        h = HostRunHarness()
        try:
            h.write_workspace_manifest(root=h.packages_host, workspace_units=["packages/*"])
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.packages_host),
                    "--platform-root",
                    str(h.packages_host),
                    "--platform-packages",
                    "github-proxy consensus",
                    "--host-packages",
                    "autochrono",
                    "--durable-root",
                    str(h.durable),
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    str((h.packages_host / "packages" / "github-proxy").resolve()),
                    str((h.packages_host / "packages" / "consensus").resolve()),
                    str(h.packages_host / "packages" / "autochrono"),
                ],
            )
        finally:
            h.close()

    def test_workspace_platform_packages_resolve_from_workspace_units(self) -> None:
        h = HostRunHarness()
        try:
            h.write_workspace_manifest(root=h.packages_host, workspace_units=["packages/*"])
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.packages_host),
                    "--platform-root",
                    str(h.packages_host),
                    "--platform-packages",
                    "github-proxy consensus",
                    "--durable-root",
                    str(h.durable),
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    str((h.packages_host / "packages" / "github-proxy").resolve()),
                    str((h.packages_host / "packages" / "consensus").resolve()),
                ],
            )
        finally:
            h.close()

    def test_substrate_host_has_only_platform_packages(self) -> None:
        h = HostRunHarness()
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "substrate-platform-source",
                {
                    "packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n',
                    "packages/consensus/fkst.toml": 'kind = "package"\nname = "consensus"\n',
                },
            )
            h.write_workspace_manifest(
                root=h.substrate_host,
                external_sources=[("fkst-packages-platform", platform_repo, ["github-proxy", "consensus"])],
            )
            h.write_external_sources_lock([("fkst-packages-platform", platform_repo, platform_rev)], root=h.substrate_host)
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.substrate_host),
                    "--platform-root",
                    str(platform_repo),
                    "--platform-packages",
                    "github-proxy consensus",
                    "--durable-root",
                    str(h.durable),
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    str((h.substrate_host / ".fkst" / "run" / "fkst-packages-platform" / "packages" / "github-proxy").resolve()),
                    str((h.substrate_host / ".fkst" / "run" / "fkst-packages-platform" / "packages" / "consensus").resolve()),
                ],
            )
        finally:
            h.close()

    def test_website_host_uses_fkst_local_packages_for_host_packages(self) -> None:
        h = HostRunHarness()
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "website-platform-source",
                {
                    "packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n',
                    "packages/consensus/fkst.toml": 'kind = "package"\nname = "consensus"\n',
                },
            )
            h.write_workspace_manifest(
                external_sources=[("fkst-packages-platform", platform_repo, ["github-proxy", "consensus"])],
            )
            h.write_external_sources_lock([("fkst-packages-platform", platform_repo, platform_rev)])
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.website_host),
                    "--platform-root",
                    str(platform_repo),
                    "--platform-packages",
                    "github-proxy consensus",
                    "--host-packages",
                    "site-board",
                    "--durable-root",
                    str(h.durable),
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    str((h.website_host / ".fkst" / "run" / "fkst-packages-platform" / "packages" / "github-proxy").resolve()),
                    str((h.website_host / ".fkst" / "run" / "fkst-packages-platform" / "packages" / "consensus").resolve()),
                    str(h.website_host / ".fkst" / "local-packages" / "site-board"),
                ],
            )
        finally:
            h.close()

    def test_explicit_local_packages_root_overrides_host_default(self) -> None:
        h = HostRunHarness()
        custom_local = h.root / "custom-local-packages"
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "custom-local-platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            h.write_workspace_manifest(external_sources=[("fkst-packages-platform", platform_repo, ["github-proxy"])])
            h.write_external_sources_lock([("fkst-packages-platform", platform_repo, platform_rev)])
            (custom_local / "site-board").mkdir(parents=True)
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.website_host),
                    "--platform-root",
                    str(platform_repo),
                    "--local-packages",
                    str(custom_local),
                    "--platform-packages",
                    "github-proxy",
                    "--host-packages",
                    "site-board",
                    "--durable-root",
                    str(h.durable),
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    str((h.website_host / ".fkst" / "run" / "fkst-packages-platform" / "packages" / "github-proxy").resolve()),
                    str(custom_local / "site-board"),
                ],
            )
        finally:
            h.close()

    def test_host_supervise_requires_target_workspace_manifest(self) -> None:
        h = HostRunHarness()
        try:
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("target fkst.workspace.toml is required for host supervise", result.stderr)
        finally:
            h.close()

    def test_platform_package_roots_are_resolved_from_target_workspace_external_source(self) -> None:
        h = HostRunHarness()
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "platform-source",
                {
                    "packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n',
                    "packages/consensus/fkst.toml": 'kind = "package"\nname = "consensus"\n',
                },
            )
            tools_repo, tools_rev = create_git_source(
                h.root,
                "tools-source",
                {"tools/probe.txt": "tool source\n"},
            )
            h.write_workspace_manifest(
                external_sources=[
                    ("fkst-packages-platform", platform_repo, ["github-proxy"]),
                    ("site-tools", tools_repo, ["site-tools"]),
                ]
            )
            h.write_external_sources_lock(
                [
                    ("fkst-packages-platform", platform_repo, platform_rev),
                    ("site-tools", tools_repo, tools_rev),
                ]
            )
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(platform_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    printf 'platform=%s\\n' "$HOST_RUN_PLATFORM_ROOT"
                    host_run_print_package_roots
                    """
                )
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            platform_checkout = h.website_host / ".fkst" / "run" / "fkst-packages-platform"
            platform_checkout = platform_checkout.resolve()
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    f"platform={platform_checkout}",
                    str(platform_checkout / "packages" / "github-proxy"),
                ],
            )
            self.assertTrue((platform_checkout / "packages" / "github-proxy").is_dir())
            self.assertFalse((h.website_host / ".fkst" / "run" / "site-tools").exists())
        finally:
            h.close()

    def test_external_platform_source_must_match_trusted_platform_root(self) -> None:
        h = HostRunHarness()
        try:
            trusted_repo, _trusted_rev = create_git_source(
                h.root,
                "trusted-platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            attacker_repo, attacker_rev = create_git_source(
                h.root,
                "attacker-platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            h.write_workspace_manifest(
                external_sources=[("fkst-packages-platform", attacker_repo, ["github-proxy"])],
            )
            h.write_external_sources_lock([("fkst-packages-platform", attacker_repo, attacker_rev)])
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(trusted_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("does not match trusted --platform-root", result.stderr)
            self.assertFalse((h.website_host / ".fkst" / "run" / "fkst-packages-platform").exists())
        finally:
            h.close()

    def test_workspace_platform_package_requires_trusted_platform_root(self) -> None:
        h = HostRunHarness()
        try:
            trusted_repo, _trusted_rev = create_git_source(
                h.root,
                "trusted-platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            (h.website_host / "packages" / "github-proxy").mkdir(parents=True)
            (h.website_host / "packages" / "github-proxy" / "fkst.toml").write_text(
                'kind = "package"\nname = "github-proxy"\n',
                encoding="utf-8",
            )
            h.write_workspace_manifest(workspace_units=["packages/*"])
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(trusted_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires trusted --platform-root", result.stderr)
            self.assertNotIn(str(h.website_host / "packages" / "github-proxy"), result.stdout)
        finally:
            h.close()

    def test_ambiguous_target_workspace_platform_package_fails_closed(self) -> None:
        h = HostRunHarness()
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            h.write_workspace_manifest(
                workspace_packages=["github-proxy"],
                external_sources=[("fkst-packages-platform", platform_repo, ["github-proxy"])],
            )
            h.write_external_sources_lock([("fkst-packages-platform", platform_repo, platform_rev)])
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("ambiguous target fkst.workspace.toml platform package 'github-proxy'", result.stderr)
        finally:
            h.close()

    def test_host_external_sources_are_hydrated_from_lock_before_launch(self) -> None:
        h = HostRunHarness()
        try:
            platform_repo, platform_rev = create_git_source(
                h.root,
                "platform-source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            tools_repo, tools_rev = create_git_source(
                h.root,
                "tools-source",
                {"tools/probe.txt": "tool source\n"},
            )
            h.write_external_sources_lock(
                [
                    ("fkst-packages-platform", platform_repo, platform_rev),
                    ("site-tools", tools_repo, tools_rev),
                ]
            )
            h.write_workspace_manifest(
                external_sources=[
                    ("fkst-packages-platform", platform_repo, ["github-proxy"]),
                    ("site-tools", tools_repo, ["site-tools"]),
                ]
            )
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(platform_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    printf 'platform=%s\\n' "$HOST_RUN_PLATFORM_ROOT"
                    host_run_print_package_roots
                    """
                )
            )
            platform_checkout = (h.website_host / ".fkst" / "run" / "fkst-packages-platform").resolve()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                result.stdout.splitlines(),
                [
                    f"platform={platform_checkout}",
                    str(platform_checkout / "packages" / "github-proxy"),
                ],
            )
            checkout = h.website_host / ".fkst" / "run" / "fkst-packages-platform"
            self.assertTrue((checkout / ".git").is_dir(), f"{checkout} is not a checkout")
            head = run_argv(["git", "rev-parse", "HEAD"], cwd=checkout)
            self.assertEqual(head.returncode, 0, head.stderr)
            self.assertEqual(head.stdout.strip(), platform_rev)
            self.assertTrue((checkout / "packages" / "github-proxy").is_dir())
            self.assertFalse((h.website_host / ".fkst" / "run" / "site-tools").exists())
        finally:
            h.close()

    def test_host_external_source_rehydrates_wrong_existing_checkout(self) -> None:
        h = HostRunHarness()
        try:
            source_repo, source_rev = create_git_source(
                h.root,
                "source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            stale_repo, _stale_rev = create_git_source(
                h.root,
                "stale",
                {"stale.txt": "stale\n"},
            )
            h.write_workspace_manifest(external_sources=[("fkst-packages-platform", source_repo, ["github-proxy"])])
            h.write_external_sources_lock([("fkst-packages-platform", source_repo, source_rev)])
            checkout = h.website_host / ".fkst" / "run" / "fkst-packages-platform"
            checkout.parent.mkdir(parents=True, exist_ok=True)
            result = run_argv(["git", "clone", "-q", str(stale_repo), str(checkout)], cwd=h.root)
            self.assertEqual(result.returncode, 0, result.stderr)

            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(source_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_build_package_roots
                    """
                )
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            head = run_argv(["git", "rev-parse", "HEAD"], cwd=checkout)
            self.assertEqual(head.returncode, 0, head.stderr)
            self.assertEqual(head.stdout.strip(), source_rev)
            origin = run_argv(["git", "config", "--get", "remote.origin.url"], cwd=checkout)
            self.assertEqual(origin.returncode, 0, origin.stderr)
            self.assertEqual(origin.stdout.strip(), str(source_repo))
        finally:
            h.close()

    def test_host_external_source_fails_closed_on_invalid_lock(self) -> None:
        h = HostRunHarness()
        try:
            h.write_workspace_manifest(
                external_sources=[
                    ("fkst-packages-platform", Path("https://example.invalid/fkst-packages.git"), ["github-proxy"])
                ]
            )
            h.write_external_sources_lock(
                [("fkst-packages-platform", Path("https://example.invalid/fkst-packages.git"), "not-a-sha")]
            )
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                result.stderr.strip(),
                "error: fkst.lock external_source(id=fkst-packages-platform) is missing resolved.rev as a full git SHA",
            )
        finally:
            h.close()

    def test_supervise_contract_hydrates_before_byte_identical_launch_args(self) -> None:
        h = HostRunHarness()
        fake_bin = h.root / "fake-framework"
        capture = h.root / "capture.json"
        try:
            source_repo, source_rev = create_git_source(
                h.root,
                "source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            h.write_workspace_manifest(external_sources=[("fkst-packages-platform", source_repo, ["github-proxy"])])
            h.write_external_sources_lock([("fkst-packages-platform", source_repo, source_rev)])
            fake_bin.write_text(
                textwrap.dedent(
                    f"""\
                    #!/usr/bin/env python3
                    import json
                    import os
                    import pathlib
                    import subprocess
                    import sys

                    checkout = pathlib.Path({json.dumps(str(h.website_host / ".fkst" / "run" / "fkst-packages-platform"))})
                    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
                    pathlib.Path({json.dumps(str(capture))}).write_text(json.dumps({{"argv": sys.argv, "head": head, "runtime": os.environ.get("FKST_RUNTIME_ROOT"), "durable": os.environ.get("FKST_DURABLE_ROOT")}}, sort_keys=True) + "\\n", encoding="utf-8")
                    """
                ),
                encoding="utf-8",
            )
            fake_bin.chmod(0o755)
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    BIN={shell_quote(fake_bin)}
                    host_run_supervise_contract --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(source_repo)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    """
                )
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            payload = json.loads(capture.read_text(encoding="utf-8"))
            self.assertEqual(payload["head"], source_rev)
            self.assertEqual(
                payload["argv"],
                [
                    str(fake_bin),
                    "supervise",
                    "--project-root",
                    str(h.website_host),
                    "--package-root",
                    str((h.website_host / ".fkst" / "run" / "fkst-packages-platform").resolve() / "packages" / "github-proxy"),
                    "--framework-bin",
                    str(fake_bin),
                ],
            )
            self.assertEqual(payload["runtime"], str(h.runtime))
            self.assertEqual(payload["durable"], str(h.durable))
        finally:
            h.close()

    def test_supervise_fails_closed_when_target_workspace_does_not_declare_platform_packages(self) -> None:
        h = HostRunHarness()
        try:
            source_repo, source_rev = create_git_source(
                h.root,
                "source",
                {"packages/github-proxy/fkst.toml": 'kind = "package"\nname = "github-proxy"\n'},
            )
            h.write_external_sources_lock([("fkst-packages-platform", source_repo, source_rev)])
            (h.website_host / "fkst.workspace.toml").write_text(
                textwrap.dedent(
                    f"""\
                    [workspace]
                    units = []

                    [[external_sources]]
                    id = "fkst-packages-platform"
                    git = {json.dumps(str(source_repo))}
                    rev = {json.dumps(source_rev)}
                    libraries = ["contract"]
                    """
                ),
                encoding="utf-8",
            )
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.website_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_build_package_roots
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "target fkst.workspace.toml does not declare platform package 'github-proxy'",
                result.stderr,
            )
        finally:
            h.close()

    def test_missing_durable_root_fails_closed(self) -> None:
        h = HostRunHarness()
        try:
            result = h.package_roots(
                [
                    "--project-root",
                    str(h.substrate_host),
                    "--platform-root",
                    str(h.platform),
                    "--platform-packages",
                    "github-proxy",
                    "--runtime-root",
                    str(h.runtime),
                ]
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--durable-root is required", result.stderr)
        finally:
            h.close()

    def test_restart_kills_pid_file_process_without_command_text_matching(self) -> None:
        h = HostRunHarness()
        pid = start_orphan_sleep()
        try:
            h.durable.mkdir()
            (h.durable / ".fkst-supervise.pid").write_text(str(pid) + "\n", encoding="utf-8")
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.substrate_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)} --restart
                    host_run_validate_shape
                    host_run_restart_prior
                    """
                )
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(wait_for_dead(pid), f"pid {pid} still alive")
            self.assertFalse((h.durable / ".fkst-supervise.pid").exists())
            self.assertIn("killing prior supervise pid", result.stderr)
        finally:
            kill_if_alive(pid)
            h.close()

    def test_restart_fails_closed_when_prior_cannot_be_killed(self) -> None:
        h = HostRunHarness()
        pid = start_orphan_sleep()
        try:
            h.durable.mkdir()
            pidfile = h.durable / ".fkst-supervise.pid"
            pidfile.write_text(str(pid) + "\n", encoding="utf-8")
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    kill() {{
                      if [ "${{1:-}}" = "-9" ]; then
                        return 1
                      fi
                      command kill "$@"
                    }}
                    host_run_parse_supervise_args --project-root {shell_quote(h.substrate_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)} --restart
                    host_run_validate_shape
                    host_run_restart_prior
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(pid_is_alive(pid), f"pid {pid} should not have been killed")
            self.assertEqual(pidfile.read_text(encoding="utf-8").strip(), str(pid))
            self.assertIn("failed to SIGKILL prior supervise pid", result.stderr)
        finally:
            kill_if_alive(pid)
            h.close()

    def test_launch_without_restart_fails_closed_when_pidfile_is_live(self) -> None:
        h = HostRunHarness()
        pid = start_orphan_sleep()
        try:
            h.durable.mkdir()
            (h.durable / ".fkst-supervise.pid").write_text(str(pid) + "\n", encoding="utf-8")
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args --project-root {shell_quote(h.substrate_host)} --platform-root {shell_quote(h.platform)} --platform-packages 'github-proxy' --durable-root {shell_quote(h.durable)} --runtime-root {shell_quote(h.runtime)}
                    host_run_validate_shape
                    host_run_claim_supervise_slot
                    """
                )
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(pid_is_alive(pid), f"pid {pid} should still be alive")
            self.assertIn("is still running for durable root", result.stderr)
        finally:
            kill_if_alive(pid)
            h.close()

    def test_explicit_runtime_root_is_used_exactly_for_launch(self) -> None:
        h = HostRunHarness()
        try:
            args = (
                f"--project-root {shell_quote(h.substrate_host)} "
                f"--platform-root {shell_quote(h.platform)} "
                f"--platform-packages 'github-proxy' "
                f"--durable-root {shell_quote(h.durable)} "
                f"--runtime-root {shell_quote(h.runtime)}"
            )
            result = h.run_helper(
                textwrap.dedent(
                    f"""\
                    set -euo pipefail
                    source scripts/host_run.sh
                    host_run_parse_supervise_args {args}
                    host_run_validate_shape
                    first="$HOST_RUN_RUNTIME_ROOT"
                    [ "$first" = {shell_quote(h.runtime)} ]
                    [ -d "$first" ]
                    host_run_parse_supervise_args {args}
                    host_run_validate_shape
                    second="$HOST_RUN_RUNTIME_ROOT"
                    [ "$second" = {shell_quote(h.runtime)} ]
                    printf '%s\\n%s\\n' "$first" "$second"
                    """
                )
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            first, second = result.stdout.splitlines()
            self.assertEqual(first, str(h.runtime))
            self.assertEqual(second, str(h.runtime))
        finally:
            h.close()


if __name__ == "__main__":
    unittest.main()
