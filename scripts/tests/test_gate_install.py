import importlib.util
import hashlib
import os
import json
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path
import sys




REQUIRED_CANONICAL_ASSETS = {
    "scripts/gate-assets.json",
    "scripts/gate.sh",
    "scripts/gate-runner.py",
    "scripts/gate-steps.json",
    "scripts/install-gate.py",
    ".githooks/pre-push",
    ".cargo/audit.toml",
}
_LOCAL_ROOT = Path(__file__).resolve().parents[2]
_MANIFEST_PATH = _LOCAL_ROOT / "scripts" / "gate-assets.json"


def _load_installer():
    module_path = _LOCAL_ROOT / "scripts" / "install-gate.py"
    spec = importlib.util.spec_from_file_location("suprnova_install_gate", module_path)
    if spec is None or spec.loader is None:  # pragma: no cover
        raise RuntimeError("failed to load installer module")

    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


installer = _load_installer()
install = installer.install
verify_install = installer.verify_install


class InstallGateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.local_root = _LOCAL_ROOT
        with open(_MANIFEST_PATH, "r", encoding="utf-8") as handle:
            manifest = json.load(handle)
        self.manifest_assets = manifest["assets"]
        self.manifest_capabilities = manifest["capabilities"]

        self.workspace = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.public_repo = self.workspace / "public"
        self.local_gate_repo = self.workspace / "local-gate-source"

        self._init_git_repo(self.public_repo)
        self._init_git_repo(self.local_gate_repo)
        self._seed_source_repo(self.local_gate_repo)
        subprocess.run(
            ["git", "add", "-f", *self.manifest_assets],
            cwd=self.local_gate_repo,
            check=True,
        )
        subprocess.run(
            ["git", "commit", "--quiet", "-m", "gate source fixture"],
            cwd=self.local_gate_repo,
            check=True,
        )
        self.local_commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=self.local_gate_repo,
            text=True,
        ).strip()
        subprocess.run(
            ["git", "fetch", "--quiet", str(self.local_gate_repo), self.local_commit],
            cwd=self.public_repo,
            check=True,
        )

    def _init_git_repo(self, path: Path) -> None:
        path.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", "--quiet"], cwd=path, check=True)
        subprocess.run(["git", "config", "user.name", "Gate Test"], cwd=path, check=True)
        subprocess.run(["git", "config", "user.email", "gate@example.com"], cwd=path, check=True)

    def _seed_source_repo(self, path: Path) -> None:
        manifest_destination = path / "scripts" / "gate-assets.json"
        manifest_destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(_MANIFEST_PATH, manifest_destination)

        for relative in self.manifest_assets:
            source = self.local_root / relative
            destination = path / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)

    def _seed_bad_source(
        self,
        assets: list[str],
        *,
        manifest_overrides: dict[str, object] | None = None,
    ) -> Path:
        root = self.workspace / "local-gate-bad"
        shutil.rmtree(root, ignore_errors=True)
        root.mkdir(parents=True, exist_ok=True)

        self._init_git_repo(root)
        assets_root = root / "scripts"
        assets_root.mkdir(parents=True, exist_ok=True)

        for relative in assets:
            if relative.startswith(("/", "../", "..")):
                continue
            source = root / relative
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("#!/usr/bin/env sh\necho skipped\n", encoding="utf-8")

        manifest = {
            "schema": 1,
            "branch": "local/gate-infra",
            "assets": assets,
            "capabilities": ["bash", "cargo", "docker", "git", "python3"],
        }
        if manifest_overrides:
            manifest.update(manifest_overrides)

        (assets_root / "gate-assets.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        return root

    @staticmethod
    def _hooks_path_set(repo: Path) -> bool:
        result = subprocess.run(
            ["git", "config", "--get", "core.hooksPath"],
            cwd=repo,
            capture_output=True,
            text=True,
        )
        return result.returncode == 0 and bool(result.stdout.strip())

    def _assert_no_install_side_effects(self, manifest_assets: list[str]) -> None:
        self.assertFalse((self.public_repo / ".git/suprnova-local-gate.json").exists())
        self.assertFalse(self._hooks_path_set(self.public_repo))
        for relative in manifest_assets:
            self.assertFalse((self.public_repo / relative).exists(), msg=f"unexpectedly copied {relative}")

    def _assert_no_install_metadata(self) -> None:
        self.assertFalse((self.public_repo / ".git/suprnova-local-gate.json").exists())
        self.assertFalse(self._hooks_path_set(self.public_repo))

    def _assert_validated_manifest_snapshot_installed(
        self,
        validated_bytes: bytes,
        validated_mode: int,
        record,
    ) -> None:
        installed_manifest = self.public_repo / "scripts/gate-assets.json"
        validated_manifest = json.loads(validated_bytes)

        self.assertEqual(installed_manifest.read_bytes(), validated_bytes)
        self.assertEqual(installed_manifest.stat().st_mode & 0o777, validated_mode)
        self.assertEqual(record.capabilities, validated_manifest["capabilities"])
        self.assertEqual(list(record.assets), validated_manifest["assets"])
        self.assertEqual(
            record.assets["scripts/gate-assets.json"],
            hashlib.sha256(validated_bytes).hexdigest(),
        )
        self.assertEqual(verify_install(self.public_repo), record)

    def test_install_copies_manifest_assets_and_sets_hooks_path(self):
        record = install(self.public_repo, self.local_gate_repo, self.local_commit)

        hooks_path = subprocess.check_output(
            ["git", "config", "--get", "core.hooksPath"],
            cwd=self.public_repo,
            text=True,
        ).strip()

        self.assertEqual(hooks_path, ".githooks")
        self.assertEqual(record.schema, 2)
        self.assertEqual(record.capabilities, self.manifest_capabilities)
        self.assertEqual(len(record.assets), 23)
        self.assertEqual(set(record.assets), set(self.manifest_assets))
        self.assertEqual(record.branch, "local/gate-infra")
        self.assertEqual(record.commit, self.local_commit)

        verify_install(self.public_repo)
        for relative in self.manifest_assets:
            self.assertTrue(
                (self.public_repo / relative).is_file(),
                msg=f"expected installed asset {relative}",
            )


    def test_manifest_contains_itself_and_complete_canonical_closure(self):
        self.assertEqual(len(self.manifest_assets), 23)
        self.assertTrue(REQUIRED_CANONICAL_ASSETS.issubset(self.manifest_assets))

    def test_manifest_missing_required_asset_fails_before_copy(self):
        assets = [
            relative
            for relative in self.manifest_assets
            if relative != ".githooks/pre-push"
        ]
        bad_source = self._seed_bad_source(assets)

        with self.assertRaisesRegex(EnvironmentError, "required gate assets"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(assets)

    def test_install_record_missing_required_asset_fails_closed(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        record_path = self.public_repo / ".git/suprnova-local-gate.json"
        payload = json.loads(record_path.read_text(encoding="utf-8"))
        del payload["assets"]["scripts/gate.sh"]
        record_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(EnvironmentError, "required gate assets"):
            verify_install(self.public_repo)

    def test_old_install_record_schema_is_rejected(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        record_path = self.public_repo / ".git/suprnova-local-gate.json"
        payload = json.loads(record_path.read_text(encoding="utf-8"))
        payload["schema"] = 1
        record_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(EnvironmentError, "install record"):
            verify_install(self.public_repo)

    def test_install_record_capabilities_must_match_installed_manifest(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        record_path = self.public_repo / ".git/suprnova-local-gate.json"
        payload = json.loads(record_path.read_text(encoding="utf-8"))
        payload["capabilities"] = ["bash"]
        record_path.write_text(json.dumps(payload), encoding="utf-8")

        with self.assertRaisesRegex(EnvironmentError, "capabilities"):
            verify_install(self.public_repo)

    def test_source_file_symlink_escape_is_rejected_before_copy(self):
        bad_source = self._seed_bad_source(self.manifest_assets)
        outside = self.workspace / "outside-source.sh"
        outside.write_text("outside\n", encoding="utf-8")
        (bad_source / "scripts/gate.sh").unlink()
        (bad_source / "scripts/gate.sh").symlink_to(outside)

        with self.assertRaisesRegex(EnvironmentError, "symlink"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(self.manifest_assets)

    def test_source_parent_symlink_escape_is_rejected_before_copy(self):
        bad_source = self._seed_bad_source(self.manifest_assets)
        outside = self.workspace / "outside-source-parent"
        outside.mkdir()
        (outside / "audit.toml").write_text("outside\n", encoding="utf-8")
        shutil.rmtree(bad_source / ".cargo")
        (bad_source / ".cargo").symlink_to(outside, target_is_directory=True)

        with self.assertRaisesRegex(EnvironmentError, "symlink"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(self.manifest_assets)

    def test_destination_file_symlink_escape_is_rejected_before_copy(self):
        outside = self.workspace / "outside-destination.sh"
        outside.write_text("outside\n", encoding="utf-8")
        (self.public_repo / "scripts").mkdir()
        (self.public_repo / "scripts/gate.sh").symlink_to(outside)

        with self.assertRaisesRegex(EnvironmentError, "symlink"):
            install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertEqual(outside.read_text(encoding="utf-8"), "outside\n")
        self.assertFalse(
            (self.public_repo / ".git/suprnova-local-gate.json").exists()
        )
        self.assertFalse(self._hooks_path_set(self.public_repo))

    def test_destination_parent_symlink_escape_is_rejected_before_copy(self):
        outside = self.workspace / "outside-destination-parent"
        outside.mkdir()
        (self.public_repo / ".cargo").symlink_to(
            outside, target_is_directory=True
        )

        with self.assertRaisesRegex(EnvironmentError, "symlink"):
            install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertFalse((outside / "audit.toml").exists())
        self.assertFalse((self.public_repo / "scripts/gate.sh").exists())
        self.assertFalse(
            (self.public_repo / ".git/suprnova-local-gate.json").exists()
        )
        self.assertFalse(self._hooks_path_set(self.public_repo))

    def test_precreated_predictable_temp_symlink_is_never_followed(self):
        outside = self.workspace / "outside-temp-sentinel"
        sentinel = b"outside temp sentinel\n"
        outside.write_bytes(sentinel)
        destination_parent = self.public_repo / "scripts"
        destination_parent.mkdir()
        predictable_temp = (
            destination_parent
            / f".gate-assets.json.install-{os.getpid()}-0"
        )
        predictable_temp.symlink_to(outside)

        record = install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertEqual(outside.read_bytes(), sentinel)
        self.assertTrue(predictable_temp.is_symlink())
        self.assertEqual(len(record.assets), 23)
        verify_install(self.public_repo)

    def test_source_swap_to_symlink_immediately_before_open_fails_closed(self):
        outside = self.workspace / "outside-source-race"
        sentinel = b"outside source sentinel\n"
        outside.write_bytes(sentinel)
        source_path = self.local_gate_repo / "scripts/gate.sh"
        real_open = os.open
        swapped = False

        def racing_open(path, flags, mode=0o777, *, dir_fd=None):
            nonlocal swapped
            if not swapped and dir_fd is None and Path(path) == source_path:
                source_path.unlink()
                source_path.symlink_to(outside)
                swapped = True
            return real_open(path, flags, mode, dir_fd=dir_fd)

        with mock.patch.object(installer.os, "open", side_effect=racing_open):
            with self.assertRaisesRegex(EnvironmentError, "local gate source"):
                install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertTrue(swapped)
        self.assertEqual(outside.read_bytes(), sentinel)
        self._assert_no_install_metadata()

    def test_manifest_regular_file_replacement_after_parse_installs_validated_snapshot(
        self,
    ):
        manifest_path = self.local_gate_repo / "scripts/gate-assets.json"
        manifest_path.chmod(0o640)
        validated_bytes = manifest_path.read_bytes()
        replacement_manifest = json.loads(validated_bytes)
        replacement_manifest["capabilities"] = list(
            reversed(replacement_manifest["capabilities"])
        )
        replacement_bytes = (
            json.dumps(replacement_manifest, indent=2).encode("utf-8") + b"\n"
        )
        replacement_path = manifest_path.with_name("gate-assets.replacement.json")
        replacement_path.write_bytes(replacement_bytes)
        replacement_path.chmod(0o600)
        replacement_inode = replacement_path.stat().st_ino
        real_validate_manifest = installer._validate_manifest
        replaced = False

        def racing_validate_manifest(manifest, source):
            nonlocal replaced
            validated = real_validate_manifest(manifest, source)
            os.replace(replacement_path, manifest_path)
            replaced = True
            return validated

        with mock.patch.object(
            installer,
            "_validate_manifest",
            side_effect=racing_validate_manifest,
        ):
            record = install(
                self.public_repo,
                self.local_gate_repo,
                self.local_commit,
            )

        self.assertTrue(replaced)
        self.assertEqual(manifest_path.stat().st_ino, replacement_inode)
        self.assertEqual(manifest_path.read_bytes(), replacement_bytes)
        self._assert_validated_manifest_snapshot_installed(
            validated_bytes,
            0o640,
            record,
        )

    def test_manifest_in_place_rewrite_after_parse_installs_validated_snapshot(
        self,
    ):
        manifest_path = self.local_gate_repo / "scripts/gate-assets.json"
        manifest_path.chmod(0o640)
        validated_bytes = manifest_path.read_bytes()
        source_inode = manifest_path.stat().st_ino
        rewritten_manifest = json.loads(validated_bytes)
        rewritten_manifest["capabilities"] = list(
            reversed(rewritten_manifest["capabilities"])
        )
        rewritten_bytes = (
            json.dumps(rewritten_manifest, indent=2).encode("utf-8") + b"\n"
        )
        real_validate_manifest = installer._validate_manifest
        rewritten = False

        def racing_validate_manifest(manifest, source):
            nonlocal rewritten
            validated = real_validate_manifest(manifest, source)
            with manifest_path.open("r+b") as manifest_file:
                manifest_file.write(rewritten_bytes)
                manifest_file.truncate()
                manifest_file.flush()
                os.fsync(manifest_file.fileno())
            manifest_path.chmod(0o600)
            rewritten = True
            return validated

        with mock.patch.object(
            installer,
            "_validate_manifest",
            side_effect=racing_validate_manifest,
        ):
            record = install(
                self.public_repo,
                self.local_gate_repo,
                self.local_commit,
            )

        self.assertTrue(rewritten)
        self.assertEqual(manifest_path.stat().st_ino, source_inode)
        self.assertEqual(manifest_path.read_bytes(), rewritten_bytes)
        self._assert_validated_manifest_snapshot_installed(
            validated_bytes,
            0o640,
            record,
        )


    def test_destination_parent_swap_after_validation_fails_closed(self):
        outside = self.workspace / "outside-parent-race"
        outside.mkdir()
        sentinel_path = outside / "sentinel"
        sentinel = b"outside parent sentinel\n"
        sentinel_path.write_bytes(sentinel)
        destination_parent = self.public_repo / "scripts"
        destination_parent.mkdir()
        real_open = os.open
        swapped = False

        def racing_open(path, flags, mode=0o777, *, dir_fd=None):
            nonlocal swapped
            if (
                not swapped
                and path == "scripts"
                and dir_fd is not None
                and flags & os.O_DIRECTORY
            ):
                destination_parent.rmdir()
                destination_parent.symlink_to(outside, target_is_directory=True)
                swapped = True
            return real_open(path, flags, mode, dir_fd=dir_fd)

        with mock.patch.object(installer.os, "open", side_effect=racing_open):
            with self.assertRaisesRegex(EnvironmentError, "destination repository"):
                install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertTrue(swapped)
        self.assertEqual(sentinel_path.read_bytes(), sentinel)
        self.assertFalse((outside / "gate-assets.json").exists())
        self._assert_no_install_metadata()

    def test_final_destination_swap_attempt_fails_before_hash_or_record(self):
        outside = self.workspace / "outside-final-race"
        sentinel = b"outside final sentinel\n"
        outside.write_bytes(sentinel)
        destination_path = self.public_repo / "scripts/gate-assets.json"
        real_open = os.open
        swapped = False

        def racing_open(path, flags, mode=0o777, *, dir_fd=None):
            nonlocal swapped
            if (
                not swapped
                and path == "gate-assets.json"
                and dir_fd is not None
                and not flags & os.O_CREAT
                and not flags & os.O_DIRECTORY
            ):
                destination_path.unlink()
                destination_path.symlink_to(outside)
                swapped = True
            return real_open(path, flags, mode, dir_fd=dir_fd)

        with mock.patch.object(installer.os, "open", side_effect=racing_open):
            with self.assertRaisesRegex(EnvironmentError, "installed gate destination"):
                install(self.public_repo, self.local_gate_repo, self.local_commit)

        self.assertTrue(swapped)
        self.assertEqual(outside.read_bytes(), sentinel)
        self.assertTrue(destination_path.is_symlink())
        self.assertFalse(
            any(
                (self.public_repo / "scripts").glob(
                    ".gate-assets.json.install-*"
                )
            )
        )
        self._assert_no_install_metadata()

    def test_verification_rejects_symlinked_installed_asset(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        outside = self.workspace / "outside-verify.sh"
        outside.write_bytes((self.public_repo / "scripts/gate.sh").read_bytes())
        (self.public_repo / "scripts/gate.sh").unlink()
        (self.public_repo / "scripts/gate.sh").symlink_to(outside)

        with self.assertRaisesRegex(EnvironmentError, "symlink"):
            verify_install(self.public_repo)

    def test_installed_byte_drift_fails_verification(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        (self.public_repo / "scripts/gate.sh").write_text("changed\n", encoding="utf-8")

        with self.assertRaisesRegex(EnvironmentError, "installed gate asset drift"):
            verify_install(self.public_repo)

    def test_installed_executable_mode_drift_fails_verification(self):
        record = install(self.public_repo, self.local_gate_repo, self.local_commit)
        self.assertEqual(verify_install(self.public_repo), record)

        (self.public_repo / ".githooks/pre-push").chmod(0o644)

        with self.assertRaisesRegex(EnvironmentError, "executable mode drift"):
            verify_install(self.public_repo)

    def test_missing_helper_fails_before_gate_execution(self):
        install(self.public_repo, self.local_gate_repo, self.local_commit)
        (self.public_repo / "scripts/check-postgres.sh").unlink()

        with self.assertRaisesRegex(EnvironmentError, "missing local gate asset"):
            verify_install(self.public_repo)

    def test_manifest_asset_path_traversal_is_rejected(self):
        bad_assets = ["scripts/gate.sh", "../outside.sh"]
        bad_source = self._seed_bad_source(bad_assets)

        with self.assertRaisesRegex(EnvironmentError, "invalid gate asset path"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(bad_assets)

    def test_manifest_asset_absolute_path_is_rejected(self):
        bad_assets = ["scripts/gate.sh", "/tmp/escape.sh"]
        bad_source = self._seed_bad_source(bad_assets)

        with self.assertRaisesRegex(EnvironmentError, "invalid gate asset path"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(bad_assets)

    def test_manifest_schema_must_be_1(self):
        bad_source = self._seed_bad_source(["scripts/gate.sh"], manifest_overrides={"schema": 2})

        with self.assertRaisesRegex(EnvironmentError, "invalid gate manifest schema"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(["scripts/gate.sh"])

    def test_manifest_branch_must_be_local_gate_infra(self):
        bad_source = self._seed_bad_source(
            ["scripts/gate.sh"],
            manifest_overrides={"branch": "main"},
        )

        with self.assertRaisesRegex(EnvironmentError, "invalid gate manifest branch"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(["scripts/gate.sh"])

    def test_manifest_capabilities_must_be_a_list_of_strings(self):
        bad_source = self._seed_bad_source(
            ["scripts/gate.sh"],
            manifest_overrides={"capabilities": ["bash", 1]},
        )

        with self.assertRaisesRegex(EnvironmentError, "invalid gate manifest capabilities"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(["scripts/gate.sh"])

    def test_manifest_capabilities_must_be_nonempty(self):
        bad_source = self._seed_bad_source(
            self.manifest_assets,
            manifest_overrides={"capabilities": []},
        )

        with self.assertRaisesRegex(EnvironmentError, "invalid gate manifest capabilities"):
            install(self.public_repo, bad_source, self.local_commit)

        self._assert_no_install_side_effects(self.manifest_assets)


if __name__ == "__main__":
    unittest.main()
