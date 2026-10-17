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


class StorageExemptionTests(unittest.TestCase):
    REGISTRY_CRATE = ("zstd", "")
    PORTS = ("tinystoragedrivers-core", "https://github.com/tinyhumansai/tinystoragedrivers?tag=v1.0.0#141fd9ed")

    def test_default_closure_rejects_the_storage_crates(self):
        self.assertEqual(len(boundary.audit_tree([self.PORTS, self.REGISTRY_CRATE])), 2)

    def test_default_closure_accepts_the_allow_list(self):
        self.assertEqual(boundary.audit_tree([("serde", ""), ("thiserror", "")]), [])

    def test_storage_feature_accepts_the_ports_crate_and_registry_crates(self):
        self.assertEqual(boundary.audit_tree([self.PORTS, self.REGISTRY_CRATE], storage=True), [])

    def test_storage_feature_still_rejects_implementations(self):
        path = ("tinywallet-crypto", "/work/crates/tinywallet-crypto")
        self.assertEqual(len(boundary.audit_tree([path], storage=True)), 1)

    def test_storage_feature_rejects_a_foreign_git_source(self):
        fake = ("tinystoragedrivers-core", "https://example.com/evil")
        self.assertEqual(len(boundary.audit_tree([fake], storage=True)), 1)


if __name__ == "__main__":
    unittest.main()
