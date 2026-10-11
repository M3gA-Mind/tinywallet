"""Verify release selection against the real Cargo workspace metadata."""
import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent


class ReleaseWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = (ROOT / ".github/workflows/release.yml").read_text()
        cls.metadata = subprocess.check_output(
            ["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"], cwd=ROOT
        )

    def test_packaging_selects_a_contract_without_unpublished_path_dependencies(self):
        command = re.search(r"run: cargo package --locked --package (\S+)", self.workflow)
        self.assertIsNotNone(command)
        package = next(p for p in json.loads(self.metadata)["packages"] if p["name"] == command[1])
        paths = [d["name"] for d in package["dependencies"] if d.get("path")]
        self.assertEqual(paths, [], "cargo package must not resolve unpublished sibling crates from crates.io")
        self.assertEqual(package["name"], "tinywallet-bus")

    def test_release_job_selects_an_explicit_rust_toolchain(self):
        step = re.search(
            r"- uses: dtolnay/rust-toolchain@[^\n]+\n(?P<with>\s+with:\n(?:\s+[^\n]+\n)+)",
            self.workflow,
        )
        self.assertIsNotNone(step)
        self.assertRegex(step["with"], r"(?m)^\s+toolchain:\s+stable\s*$")

    def test_module_build_accepts_versioned_host_labels_without_paths(self):
        result = subprocess.run(
            [
                "bash",
                str(ROOT / "scripts/build-module.sh"),
                "--crate",
                "tinywallet-module",
                "--version",
                "0.10.0",
                "--target",
                "unsupported-target",
                "--host",
                "ubuntu-22.04-x86_64",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unsupported module target", result.stderr)

        unsafe_host = subprocess.run(
            [
                "bash",
                str(ROOT / "scripts/build-module.sh"),
                "--crate",
                "tinywallet-module",
                "--version",
                "0.10.0",
                "--target",
                "unsupported-target",
                "--host",
                "../escape",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(unsafe_host.returncode, 2)
        self.assertIn("usage:", unsafe_host.stderr)

    def test_arch_release_install_disables_unsupported_container_sandbox(self):
        script = (ROOT / "scripts/test-distro.sh").read_text()
        self.assertRegex(script, r"pacman -Syu --noconfirm --disable-sandbox\b")

    def test_existing_tag_recovery_does_not_skip_bundle_job(self):
        job = self.workflow.split("  module-release:\n", 1)[1]
        condition = re.search(r"^    if: (.+)$", job, re.MULTILINE)[1]
        self.assertIn("always()", condition, "a skipped versioning ancestor must not implicitly skip recovery bundles")
        self.assertIn("needs.release-target.result == 'success'", condition, "recovery must still require successful target resolution")

    def test_release_identity_comes_from_the_root_library(self):
        selectors = re.findall(r"jq -r '([^']+)' <<< \"\$metadata\"", self.workflow)
        self.assertGreaterEqual(len(selectors), 2)
        selected = [subprocess.check_output(["jq", "-r", selector], input=self.metadata).decode().strip() for selector in selectors[:2]]
        root = next(p for p in json.loads(self.metadata)["packages"] if p["name"] == "tinywallet")
        self.assertEqual(selected, [root["name"], root["version"]])


if __name__ == "__main__":
    unittest.main()
