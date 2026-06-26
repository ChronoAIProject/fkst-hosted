#!/usr/bin/env python3
"""Tests for manifest-based stateless-generator G10 exemption."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_repo


class StatelessGeneratorSagaExemptionTest(unittest.TestCase):
    def write_department(self, root: Path, package: str, source: str, manifest: str = "") -> Path:
        pkg = root / "packages" / package
        dept = pkg / "departments" / "render"
        dept.mkdir(parents=True)
        (pkg / "fkst.toml").write_text(manifest, encoding="utf-8")
        path = dept / "main.lua"
        path.write_text(source, encoding="utf-8")
        return path

    def g10_violations(self, root: Path) -> list[str]:
        violations: list[str] = []
        warnings: list[str] = []
        check_repo.check_saga_handler_ratchet(root, violations, warnings, enforce_base=False)
        self.assertEqual(warnings, [])
        return violations

    def test_manifest_stateless_generator_package_exempts_free_form_department(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_department(
                root,
                "generator",
                "function pipeline(event)\n  return event\nend\n",
                'name = "generator"\npersistence_class = "stateless_generator"\n',
            )

            self.assertEqual(self.g10_violations(root), [])

    def test_source_shape_does_not_control_stateless_generator_exemption(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_department(
                root,
                "generator",
                """
M = { spec = { kind = "stateless_generator", consumes = { "q" }, produces = { "p" } } }
function pipeline(event)
  exec_sync("printf side-effect-looking")
  raise("p", event)
end
""",
                'name = "generator"\npersistence_class = "stateless_generator"\n',
            )

            self.assertEqual(self.g10_violations(root), [])

    def test_non_generator_package_is_not_exempt(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_department(
                root,
                "worker",
                "function pipeline(event)\n  return event\nend\n",
                'name = "worker"\n',
            )

            violations = self.g10_violations(root)

        self.assertEqual(len(violations), 1)
        self.assertIn("free-form department not on saga-handler allowlist", violations[0])


if __name__ == "__main__":
    unittest.main()
