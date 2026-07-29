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

    def write_manifest(self, packages: list[object], schema_version: object = 1) -> None:
        document = {
            "schemaVersion": schema_version,
            "name": "default-workflows",
            "description": "fixture",
            "packages": packages,
        }
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


if __name__ == "__main__":
    unittest.main()
