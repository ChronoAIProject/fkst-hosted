#!/usr/bin/env python3
"""Tests for the hosted package catalog manifest validator."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from check_hosted_catalog import CATALOG_PREFIX, validate_catalog


class HostedCatalogTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "manifests").mkdir()
        package = self.root / "packages" / "workflow-dev"
        package.mkdir(parents=True)
        (package / "fkst.toml").write_text('name = "workflow-dev"\n', encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_manifest(
        self,
        packages: list[object],
        schema_version: object = 1,
        package_env: object = None,
    ) -> None:
        document = {
            "schemaVersion": schema_version,
            "name": "default-workflows",
            "description": "fixture",
            "packages": packages,
        }
        if package_env is not None:
            document["packageEnv"] = package_env
        (self.root / "manifests" / "default-workflows.json").write_text(
            json.dumps(document), encoding="utf-8"
        )

    def test_accepts_self_owned_existing_package(self) -> None:
        self.write_manifest([f"{CATALOG_PREFIX}packages/workflow-dev"])
        self.assertEqual(validate_catalog(self.root), [])

    def test_rejects_foreign_catalog_reference(self) -> None:
        self.write_manifest(
            ["ChronoAIProject/fkst-packages@fkst-hosted:packages/workflow-dev"]
        )
        self.assertIn("must start with", validate_catalog(self.root)[0])

    def test_rejects_duplicate_reference(self) -> None:
        package_ref = f"{CATALOG_PREFIX}packages/workflow-dev"
        self.write_manifest([package_ref, package_ref])
        self.assertIn("duplicates", validate_catalog(self.root)[0])

    def test_rejects_missing_package_manifest(self) -> None:
        self.write_manifest([f"{CATALOG_PREFIX}packages/not-present"])
        self.assertIn("does not resolve", validate_catalog(self.root)[0])

    def test_rejects_unknown_schema_version(self) -> None:
        self.write_manifest([f"{CATALOG_PREFIX}packages/workflow-dev"], schema_version=2)
        self.assertIn("schemaVersion", validate_catalog(self.root)[0])




    # --- packageEnv -------------------------------------------------------
    # A manifest must not be able to publish configuration a trigger author
    # would be refused; a malformed block would otherwise only surface when a
    # session starts.

    def _with_env(self, package_env: object) -> list[str]:
        self.write_manifest(
            [f"{CATALOG_PREFIX}packages/workflow-dev"], package_env=package_env
        )
        return validate_catalog(self.root)

    def test_absent_package_env_still_passes(self) -> None:
        self.write_manifest([f"{CATALOG_PREFIX}packages/workflow-dev"])
        self.assertEqual(validate_catalog(self.root), [])

    def test_accepts_a_valid_package_env(self) -> None:
        self.assertEqual(
            self._with_env({"github-devloop": {"FKST_DEVLOOP_AUTO_REFINE_MAX": "2"}}), []
        )

    def test_rejects_a_non_object_package_env(self) -> None:
        self.assertIn("must be an object", self._with_env(["nope"])[0])

    def test_rejects_an_invalid_key(self) -> None:
        self.assertIn("invalid key", self._with_env({"pkg": {"lowercase": "x"}})[0])

    def test_rejects_a_platform_owned_key(self) -> None:
        self.assertIn(
            "the platform owns",
            self._with_env({"pkg": {"FKST_SESSION_ID": "x"}})[0],
        )

    def test_rejects_a_non_string_value(self) -> None:
        self.assertIn(
            "non-string", self._with_env({"pkg": {"FKST_DEVLOOP_MAX_INFLIGHT": 2}})[0]
        )

    def test_rejects_the_same_key_under_two_packages(self) -> None:
        # One flat environment reaches the pod, so one side would silently win.
        errors = self._with_env(
            {"a": {"FKST_DEVLOOP_TEST_COMMAND": "x"}, "b": {"FKST_DEVLOOP_TEST_COMMAND": "y"}}
        )
        self.assertTrue(any("already set by" in e for e in errors), errors)

    def test_an_unknown_package_is_advisory_not_an_error(self) -> None:
        # A manifest may pre-configure a package it does not itself bundle; at
        # runtime an unknown block is simply inert.
        self.assertEqual(
            self._with_env({"not-in-this-catalog": {"FKST_DEVLOOP_MAX_INFLIGHT": "4"}}), []
        )


if __name__ == "__main__":
    unittest.main()
