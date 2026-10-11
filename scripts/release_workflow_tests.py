"""Verify release selection against the real Cargo workspace metadata."""
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
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
        script = (ROOT / "scripts/build-module-in-distro.sh").read_text()
        self.assertRegex(script, r"pacman -Syu --noconfirm --disable-sandbox\b")

    def test_distro_container_script_parses_as_bash(self):
        for script in (
            ROOT / "scripts/test-distro.sh",
            ROOT / "scripts/build-module-in-distro.sh",
        ):
            with self.subTest(script=script.name):
                result = subprocess.run(
                    ["bash", "-n", str(script)],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_distro_runner_invokes_the_mounted_build_script(self):
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="release-workflow-test-", dir=target) as scratch:
            scratch_path = Path(scratch)
            fake_bin = scratch_path / "bin"
            fake_bin.mkdir()
            docker = fake_bin / "docker"
            docker.write_text(
                "#!/usr/bin/env python3\n"
                "import subprocess, sys\n"
                "expected_command = ['bash', '/workspace/scripts/build-module-in-distro.sh']\n"
                "args = sys.argv[1:]\n"
                f"volume_ok = len(args) > 2 and args[args.index('--volume') + 1] == '{ROOT}:/workspace'\n"
                "if not volume_ok or args[-2:] != expected_command:\n"
                "    print(f'unexpected container command: {args}', file=sys.stderr)\n"
                "    raise SystemExit(1)\n"
                f"raise SystemExit(subprocess.call(['bash', '-n', {str(ROOT / 'scripts/build-module-in-distro.sh')!r}]))\n"
            )
            docker.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            result = subprocess.run(
                [
                    "bash",
                    str(ROOT / "scripts/test-distro.sh"),
                    "fedora:43@sha256:a651ddf48ea28a06ed4e1e6519f51c9f47e7a5a138722ade87369b8fbb7e5b42",
                    "--crate",
                    "tinywallet-module",
                    "--version",
                    "0.13.0",
                    "--target",
                    "x86_64-unknown-linux-gnu",
                    "--host",
                    "fedora-43-x86_64",
                    "--output-dir",
                    f"target/{scratch_path.name}/release-assets",
                ],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_module_verification_canonicalizes_a_symlinked_temp_root(self):
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="release-workflow-test-", dir=target) as scratch:
            scratch_path = Path(scratch)
            physical_tmp = scratch_path / "physical-tmp"
            physical_tmp.mkdir()
            alias_tmp = scratch_path / "tmp-alias"
            alias_tmp.symlink_to(physical_tmp, target_is_directory=True)

            archive = scratch_path / "tinywallet-module-test.tar.gz"
            with tarfile.open(archive, "w:gz") as bundle:
                for name, contents in (
                    ("modules.toml", b'"libtinywallet_module.so" = "fixture-hash"\n'),
                    ("libtinywallet_module.so", b"fixture-library"),
                ):
                    info = tarfile.TarInfo(name)
                    info.size = len(contents)
                    bundle.addfile(info, io.BytesIO(contents))

            fake_bin = scratch_path / "bin"
            fake_bin.mkdir()
            cargo = fake_bin / "cargo"
            cargo.write_text(
                "#!/usr/bin/env python3\n"
                "import os, sys\n"
                "path = os.environ['TINYWALLET_TEST_MODULE']\n"
                "resolved = os.path.realpath(path)\n"
                "if path != resolved:\n"
                "    print(f'noncanonical module path: {path} -> {resolved}', file=sys.stderr)\n"
                "    raise SystemExit(1)\n"
            )
            cargo.chmod(0o755)

            env = os.environ.copy()
            env["TMPDIR"] = str(alias_tmp)
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/verify-module.sh"), "--archive", str(archive)],
                cwd=ROOT,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

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
