#!/usr/bin/env python3
"""Contract tests for scripts/run.sh repository-check orchestration."""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


def load_check_repo():
    path = Path(__file__).with_name("check_repo.py")
    spec = importlib.util.spec_from_file_location("check_repo", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load check_repo.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


check_repo = load_check_repo()


class RunScriptContractTest(unittest.TestCase):
    def source(self) -> str:
        return Path(__file__).with_name("run.sh").read_text(encoding="utf-8")

    def test_supervise_requires_shared_rate_pool_root(self) -> None:
        source = self.source()

        self.assertIn('if [ -z "${FKST_RATE_POOL_ROOT:-}" ]; then', source)
        self.assertIn("FKST_RATE_POOL_ROOT is required for supervise", source)
        self.assertIn("FKST_RATE_POOL_ROOT must be an absolute host-stable directory path", source)
        self.assertIn('echo "FKST_RATE_POOL_ROOT=$FKST_RATE_POOL_ROOT"', source)

    def test_python_repository_checks_do_not_write_bytecode_cache(self) -> None:
        source = self.source()
        expected = (
            "check_repo.py", "ratchet_base_test.py", "check_repo_fkst_layout.py", "check_repo_dedup_test.py",
            "check_repo_content_truncation_test.py", "check_repo_coverage_test.py",
            "check_repo_integration_coverage_test.py", "check_repo_intake_default_surface_test.py", "check_repo_dead_letter_test.py", "check_repo_producer_liveness_test.py", "check_repo_monotone_gate_test.py", "check_repo_hidden_state_test.py",
            "check_repo_test_graphql.py", "check_repo_interface_test.py", "lua_coverage_to_lcov_test.py", "check_repo_test.py", "check_repo_github_content_ingress_test.py", "check_repo_error_class_test.py", "check_repo_dependency_cycle_test.py",
            "check_repo_std_dependency_model_test.py", "check_repo_devloop_installer_test.py", "check_repo_saga_head_test.py",
            "check_repo_namespaced_queue_test.py", "check_repo_shell_out_to_self_test.py", "check_repo_fkst_layout_test.py",
            "bin_cache_test.py", "bin_bootstrap_test.py", "host_entry_test.py", "host_run_test.py", "host_run_equivalence_test.py",
            "run_sh_coverage_test.py", "run_sh_test_affected_test.py", "board_test.py", "dogfood_board_test.py", "doctor_test.py", "ratchet_migration_slicer_test.py",
            "competence_gate_test.py", "test_parallel_test.py",
        )
        for path in expected:
            self.assertIn(f'python3 -B "$ROOT/scripts/{path}"', source)
            self.assertNotIn(f'python3 "$ROOT/scripts/{path}"', source)

    def test_package_runtime_view_is_regenerated_from_source_packages(self) -> None:
        source = self.source()

        self.assertIn('SOURCE_PACKAGES_ROOT="$ROOT/packages"', source)
        self.assertIn('LOCAL_PACKAGES_ROOT="$FKST_DIR/local-packages"', source)
        self.assertIn('EXTERNAL_PACKAGES_ROOT="$FKST_DIR/packages"', source)
        self.assertIn('ln -sfn ../packages "$LOCAL_PACKAGES_ROOT"', source)
        self.assertIn('for src_pkg in "$SOURCE_PACKAGES_ROOT"/*/; do', source)
        self.assertIn('pkg="$LOCAL_PACKAGES_ROOT/$name"', source)

    def test_full_test_blocks_on_repository_check_before_engine_resolution(self) -> None:
        source = self.source()

        self.assertIn("elif ! _chk_out=\"$(cmd_check 2>&1)\"; then", source)
        self.assertIn("printf '%s\\n' \"$_chk_out\"; exit 1", source)
        self.assertLess(source.index("cmd_check"), source.index("resolve_bin; ensure_fresh_bin; cmd_test"))

    def test_full_test_fails_on_g1_before_bin_resolution(self) -> None:
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as tmp:
            probe = Path(tmp) / "repo"
            scripts = probe / "scripts"
            pkg = probe / "packages" / "oversized"
            scripts.mkdir(parents=True)
            pkg.mkdir(parents=True)

            for name in ("run.sh", "test_affected.sh", "test_parallel.sh", "bin_bootstrap.sh", "host_entry.sh", "host_run.sh", "composed_manifest.sh", "check_repo.py", "check_repo_config.py", "check_repo_runner.py", "check_repo_content_truncation.py", "check_repo_coverage.py", "check_repo_cross_package.py", "check_repo_dead_letter.py", "check_repo_dependency_cycle.py", "check_repo_devloop_godlib.py", "check_repo_devloop_decouple.py", "check_repo_devloop_installer.py", "check_repo_service_locator.py", "check_repo_ambient_surface.py", "check_repo_core_param.py", "check_repo_dedup.py", "check_repo_error_class.py", "check_repo_gh_git_adapter.py", "check_repo_github_content_ingress.py", "check_repo_hidden_state.py", "check_repo_ingress.py", "check_repo_intake_default_surface.py", "check_repo_intake_routing.py", "check_repo_integration_coverage.py", "check_repo_library_layering.py", "check_repo_live_run_dispatch.py", "check_repo_lower_injected_m.py", "check_repo_monotone_gate.py", "check_repo_namespaced_queue.py", "check_repo_ownership_gate.py", "check_repo_perm.py", "check_repo_producer_liveness.py", "check_repo_restart_lifecycle.py", "check_repo_saga_handler.py", "check_repo_saga_head.py", "check_repo_saga_split.py", "check_repo_shell_out_to_self.py", "check_repo_std_dependency_model.py", "check_repo_version_suffix.py", "ratchet_base.py"):
                shutil.copy2(root / "scripts" / name, scripts / name)
            for name in ("check_repo_coverage_test.py", "check_repo_integration_coverage_test.py", "check_repo_intake_default_surface_test.py", "check_repo_dead_letter_test.py", "check_repo_dedup_test.py", "check_repo_content_truncation_test.py", "check_repo_dependency_cycle_test.py", "check_repo_producer_liveness_test.py", "check_repo_monotone_gate_test.py", "check_repo_hidden_state_test.py", "check_repo_test_graphql.py", "check_repo_interface_test.py", "lua_coverage_to_lcov_test.py", "check_repo_test.py", "check_repo_github_content_ingress_test.py", "check_repo_error_class_test.py", "check_repo_library_layering_test.py", "check_repo_std_dependency_model_test.py", "check_repo_devloop_installer_test.py", "check_repo_restart_lifecycle_test.py", "check_repo_saga_head_test.py", "check_repo_namespaced_queue_test.py", "check_repo_shell_out_to_self_test.py", "check_repo_fkst_layout.py", "check_repo_fkst_layout_test.py", "bin_cache_test.py", "bin_bootstrap_test.py", "host_entry_test.py", "host_run_test.py", "host_run_equivalence_test.py", "run_sh_coverage_test.py", "run_sh_test_affected_test.py", "composed_manifest_test.py", "board_test.py", "dogfood_board_test.py", "doctor_test.py", "ratchet_migration_slicer_test.py", "run_script_contract_test.py", "ratchet_base_test.py", "competence_gate_test.py", "test_parallel_test.py"):
                (scripts / name).write_text("#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8")

            core_lines = [
                "local M = {}",
                "function M.persistence_class() return \"stateless_adapter\" end",
                "return M",
            ]
            core_lines.extend("-- filler" for _ in range(check_repo.LINE_LIMIT + 1 - len(core_lines)))
            (pkg / "core.lua").write_text("\n".join(core_lines) + "\n", encoding="utf-8")

            env = os.environ.copy()
            env["BIN"] = str(probe / "missing-fkst-framework")
            result = subprocess.run(
                ["/bin/bash", "scripts/run.sh", "test"],
                cwd=probe,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        combined = result.stdout + result.stderr
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("repository check failed:", combined)
        self.assertIn("G1: packages/oversized/core.lua has 1001 lines; limit is 1000", combined)
        self.assertNotIn("explicit BIN is not executable", combined)


if __name__ == "__main__":
    unittest.main()
