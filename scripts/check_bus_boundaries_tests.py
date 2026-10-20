"""Regression fixtures for implementation edges and registry identity."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).with_name("check-bus-boundaries.py"))
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)


def metadata(name, kind=None, source=boundary.REGISTRY):
    return {"packages": [{"id": "root", "name": "tinywallet-bus"}, {"id": "dependency", "name": name, "source": source}], "resolve": {"nodes": [{"id": "root", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": kind}]}]}, {"id": "dependency", "deps": []}]}}


class ContractBoundaryTests(unittest.TestCase):
    def test_registry_serde_is_allowed(self):
        self.assertEqual(boundary.audit(metadata("serde")), [])

    def test_implementation_is_rejected_even_as_a_build_dependency(self):
        for kind in (None, "build"):
            self.assertEqual(boundary.audit(metadata("tinywallet-crypto", kind)), ["tinywallet-bus -> tinywallet-crypto"])

    def test_dev_dependencies_are_exempt(self):
        self.assertEqual(boundary.audit(metadata("tinywallet-crypto", "dev")), [])

    def test_local_package_cannot_impersonate_serde(self):
        self.assertTrue(boundary.audit(metadata("serde", source=None)))


class EngineTreeTests(unittest.TestCase):
    def test_versioned_cargo_tree_lines_are_checked_by_package_name(self):
        output = """\
tinywallet-module v0.24.1 (/work/tinywallet)
rusqlite v0.40.2 (registry+https://github.com/rust-lang/crates.io-index)
tinystoragedrivers-api v2.0.0 (https://github.com/senamakel/tinystoragedrivers)
"""
        names = boundary.tree_package_names(output)
        self.assertEqual(
            boundary.engine_violations(names),
            ["rusqlite"],
        )

    def test_storage_api_and_types_are_not_engine_crates(self):
        self.assertEqual(
            boundary.engine_violations(
                ["tinystoragedrivers-api", "tinystoragedrivers-types"]
            ),
            [],
        )

    def test_underscore_names_are_normalized_before_engine_matching(self):
        self.assertEqual(boundary.engine_violations(["libsqlite3_sys"]), ["libsqlite3-sys"])


if __name__ == "__main__":
    unittest.main()
