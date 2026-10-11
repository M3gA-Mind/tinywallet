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

    def test_release_uses_the_canonical_module_pipeline(self):
        """Every archive is built by the shared pipeline from the selected tag."""
        self.assertIn("tinyhumansai/.github/.github/workflows/module-release.yml@", self.workflow)
        self.assertIn("module-crate: tinywallet-module", self.workflow)
        self.assertNotIn("git push", self.workflow)
        self.assertNotIn("cargo package", self.workflow)

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

    def test_existing_tag_resolves_without_a_version_bump_ancestor(self):
        """Tag resolution rejects invalid refs and passes valid version tags through."""
        script = self.workflow.split("        run: |\n", 1)[1].split("  module-release:", 1)[0]
        script = "\n".join(line[10:] for line in script.splitlines())
        with tempfile.TemporaryDirectory() as directory:
            for tag, valid in [("v1.0.0", True), ("v1.0.0-beta.1", True), ("main", False),
                               ("v1.0.0; echo injected", False)]:
                output = Path(directory) / "output"
                output.write_text("")
                result = subprocess.run(["bash", "-c", script],
                    env=dict(os.environ, TAG=tag, GITHUB_OUTPUT=str(output)), capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, valid, result.stderr)
                if valid:
                    self.assertEqual(output.read_text(), "tag="+tag+"\nversion="+tag[1:]+"\n")

if __name__ == "__main__":
    unittest.main()
