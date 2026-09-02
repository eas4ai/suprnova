import dataclasses
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


_LOCAL_ROOT = Path(__file__).resolve().parents[2]
_RUNNER_PATH = _LOCAL_ROOT / "scripts" / "gate-runner.py"


def _load_runner():
    spec = importlib.util.spec_from_file_location("suprnova_gate_runner_stamp", _RUNNER_PATH)
    if spec is None or spec.loader is None:  # pragma: no cover
        raise RuntimeError("failed to load gate runner module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _run(repo, *argv):
    return subprocess.check_output(["git", *argv], cwd=repo, text=True).strip()


class StampV2Tests(unittest.TestCase):
    def setUp(self):
        self.runner = _load_runner()
        self.workspace = Path(tempfile.mkdtemp(prefix="gate-stamp-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.source = self.workspace / "local-source"
        self.repo = self.workspace / "public"
        self.bin_dir = self.workspace / "bin"
        self.bin_dir.mkdir()
        rustc = self.bin_dir / "rustc"
        rustc.write_text("#!/bin/sh\nprintf 'rustc 1.94.0 (gate-test)\\n'\n", encoding="utf-8")
        rustc.chmod(0o755)
        self.env = {**os.environ, "PATH": str(self.bin_dir) + os.pathsep + os.environ.get("PATH", "")}

        self.assets = [
            "scripts/gate-assets.json",
            "scripts/gate-runner.py",
            "scripts/gate-steps.json",
            ".githooks/pre-push",
            "scripts/gate.sh",
            "scripts/install-gate.py",
            "scripts/helper.sh",
            ".cargo/audit.toml",
        ]
        self.capabilities = ["bash", "cargo", "docker", "git", "python3"]
        self._seed_local_source()
        self._seed_public_repo()
        self._install_assets()

    def _init_repo(self, path, branch):
        path.mkdir(parents=True)
        subprocess.run(["git", "init", "--quiet", "-b", branch], cwd=path, check=True)
        subprocess.run(["git", "config", "user.name", "Gate Test"], cwd=path, check=True)
        subprocess.run(["git", "config", "user.email", "gate@example.com"], cwd=path, check=True)

    def _seed_local_source(self):
        self._init_repo(self.source, "local/gate-infra")
        for relative in self.assets:
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            if relative in {"scripts/gate-runner.py", ".githooks/pre-push"}:
                shutil.copy2(_LOCAL_ROOT / relative, path)
            elif relative == "scripts/gate-steps.json":
                shutil.copy2(_LOCAL_ROOT / relative, path)
            elif relative == "scripts/gate-assets.json":
                path.write_text(
                    json.dumps(
                        {
                            "schema": 1,
                            "branch": "local/gate-infra",
                            "assets": self.assets,
                            "capabilities": self.capabilities,
                        },
                        indent=2,
                    )
                    + "\n",
                    encoding="utf-8",
                )
            else:
                path.write_text(f"local asset {relative}\n", encoding="utf-8")
        subprocess.run(["git", "add", "-f", *self.assets], cwd=self.source, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "local tooling"], cwd=self.source, check=True)
        self.local_commit = _run(self.source, "rev-parse", "HEAD")

    def _seed_public_repo(self):
        self._init_repo(self.repo, "main")
        (self.repo / ".gitignore").write_text("/scripts/\n/.githooks/\n/.cargo/\n", encoding="utf-8")
        (self.repo / "Cargo.toml").write_text("[workspace]\nmembers = []\n", encoding="utf-8")
        (self.repo / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.94.0"\n', encoding="utf-8")
        (self.repo / "README.md").write_text("public tree\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "public"], cwd=self.repo, check=True)
        subprocess.run(
            [
                "git",
                "fetch",
                "--quiet",
                str(self.source),
                "local/gate-infra:refs/heads/local/gate-infra",
            ],
            cwd=self.repo,
            check=True,
        )

    def _install_assets(self):
        hashes = {}
        for relative in self.assets:
            source = self.source / relative
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            hashes[relative] = hashlib.sha256(destination.read_bytes()).hexdigest()
        record = {
            "schema": 2,
            "branch": "local/gate-infra",
            "commit": self.local_commit,
            "assets": hashes,
            "capabilities": self.capabilities,
        }
        record_path = self.repo / _run(self.repo, "rev-parse", "--git-path", "suprnova-local-gate.json")
        record_path.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
        subprocess.run(["git", "config", "core.hooksPath", ".githooks"], cwd=self.repo, check=True)

    def _stamp(self, tier="default"):
        return self.runner.GateStamp(
            schema=2,
            tier=tier,
            tree=_run(self.repo, "rev-parse", "HEAD^{tree}"),
            commit=_run(self.repo, "rev-parse", "HEAD"),
            toolchain="rustc 1.94.0 (gate-test)",
            steps_hash=self.runner.compute_steps_hash(self.repo, tier),
            finished_at="2026-08-24T12:00:00Z",
            run_id="stamp-test",
            code_provenance=None,
            local_tooling_commit=self.local_commit,
        )

    def _validate(self, stamp, **kwargs):
        return self.runner.validate_stamp(self.repo, stamp, env=self.env, **kwargs)

    def _run_pre_push(self, updates):
        return subprocess.run(
            [
                str(self.repo / ".githooks/pre-push"),
                "origin",
                "https://github.com/owner/repo",
            ],
            cwd=self.repo,
            env=self.env,
            input=updates,
            capture_output=True,
            text=True,
        )

    def test_schema_2_round_trip_uses_git_path(self):
        stamp = self._stamp()

        self.runner.write_stamp(self.repo, stamp)

        self.assertEqual(self.runner.load_stamp(self.repo), stamp)
        stamp_path = self.repo / _run(self.repo, "rev-parse", "--git-path", "suprnova-gate-pass")
        payload = json.loads(stamp_path.read_text(encoding="utf-8"))
        self.assertEqual(payload["schema"], 2)
        self.assertEqual(set(payload), set(dataclasses.asdict(stamp)))

    def test_runner_rejects_old_install_record_schema(self):
        stamp = self._stamp()
        record_path = self.repo / _run(
            self.repo, "rev-parse", "--git-path", "suprnova-local-gate.json"
        )
        payload = json.loads(record_path.read_text(encoding="utf-8"))
        payload["schema"] = 1
        record_path.write_text(json.dumps(payload), encoding="utf-8")

        decision = self._validate(stamp)

        self.assertFalse(decision.valid)
        self.assertIn("install record", decision.message)

    def test_runner_rejects_record_missing_canonical_asset(self):
        stamp = self._stamp()
        record_path = self.repo / _run(
            self.repo, "rev-parse", "--git-path", "suprnova-local-gate.json"
        )
        payload = json.loads(record_path.read_text(encoding="utf-8"))
        del payload["assets"]["scripts/gate.sh"]
        record_path.write_text(json.dumps(payload), encoding="utf-8")

        decision = self._validate(stamp)

        self.assertFalse(decision.valid)
        self.assertIn("required gate assets", decision.message)

    def test_legacy_stamp_is_invalid(self):
        stamp_path = self.repo / _run(self.repo, "rev-parse", "--git-path", "suprnova-gate-pass")
        stamp_path.write_text(_run(self.repo, "rev-parse", "HEAD^{tree}") + "\n", encoding="utf-8")
        self.assertIsNone(self.runner.load_stamp(self.repo))

    def test_wrong_tree_toolchain_tier_and_hash_are_rejected(self):
        base = self._stamp()
        cases = [
            dataclasses.replace(base, tree="0" * 40),
            dataclasses.replace(base, toolchain="rustc other"),
            dataclasses.replace(base, tier="docs"),
            dataclasses.replace(base, steps_hash="0" * 64),
        ]
        for stamp in cases:
            with self.subTest(stamp=stamp):
                validation = self._validate(stamp)
                self.assertFalse(validation.valid)
                self.assertTrue(validation.message)

    def test_dirty_tree_neither_writes_nor_consumes_stamp(self):
        stamp = self._stamp()
        (self.repo / "README.md").write_text("dirty\n", encoding="utf-8")

        validation = self._validate(stamp)
        self.assertFalse(validation.valid)
        self.assertIn("dirty", validation.message)
        with self.assertRaisesRegex(EnvironmentError, "dirty"):
            self.runner.build_stamp(
                self.repo,
                tier="default",
                run_id="dirty",
                code_provenance=None,
                env=self.env,
            )

    def test_full_satisfies_default_but_default_never_satisfies_release(self):
        full = self._stamp("full")
        default = self._stamp("default")

        self.assertTrue(self._validate(full, required_tier="default").valid)
        self.assertFalse(self._validate(default, required_tier="full", release=True).valid)
        self.assertTrue(
            self._validate(
                full,
                required_tier="full",
                release=True,
                pushed_commit=full.commit,
            ).valid
        )

    def test_local_tooling_byte_drift_changes_hash_and_invalidates_stamp(self):
        stamp = self._stamp()
        old_hash = stamp.steps_hash
        (self.repo / "scripts/helper.sh").write_text("drift\n", encoding="utf-8")

        self.assertNotEqual(self.runner.compute_steps_hash(self.repo, "default"), old_hash)
        validation = self._validate(stamp)
        self.assertFalse(validation.valid)
        self.assertIn("tooling", validation.message)

    def test_tracked_cargo_and_toolchain_inputs_change_steps_hash(self):
        old_hash = self.runner.compute_steps_hash(self.repo, "default")
        (self.repo / "Cargo.toml").write_text("[workspace]\nresolver = \"2\"\nmembers = []\n", encoding="utf-8")
        subprocess.run(["git", "add", "Cargo.toml"], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "cargo input"], cwd=self.repo, check=True)

        self.assertNotEqual(self.runner.compute_steps_hash(self.repo, "default"), old_hash)

    def test_local_tooling_commit_is_validated_as_provenance(self):
        stamp = dataclasses.replace(self._stamp(), local_tooling_commit="0" * 40)
        validation = self._validate(stamp)
        self.assertFalse(validation.valid)
        self.assertIn("local tooling commit", validation.message)

    def test_every_non_deletion_tip_tree_must_match_stamp(self):
        stamp = self._stamp()
        self.runner.write_stamp(self.repo, stamp)
        old_commit = stamp.commit
        (self.repo / "README.md").write_text("different tree\n", encoding="utf-8")
        subprocess.run(["git", "add", "README.md"], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "different"], cwd=self.repo, check=True)
        new_commit = _run(self.repo, "rev-parse", "HEAD")
        updates = (
            f"refs/heads/old {old_commit} refs/heads/old {'0' * 40}\n"
            f"refs/heads/new {new_commit} refs/heads/new {'0' * 40}\n"
        )

        authorization = self.runner.authorize_push(
            self.repo, "origin", "git@github.com:owner/repo.git", updates, env=self.env
        )

        self.assertFalse(authorization.allowed)
        self.assertIn("refs/heads/new", authorization.message)

    def test_ref_deletions_are_exempt_without_a_stamp(self):
        updates = f"(delete) {'0' * 40} refs/heads/old {'1' * 40}\n"
        authorization = self.runner.authorize_push(
            self.repo, "origin", "git@github.com:owner/repo.git", updates, env=self.env
        )
        self.assertTrue(authorization.allowed)

    def test_github_local_ref_is_rejected_even_in_multi_ref_push(self):
        stamp = self._stamp()
        self.runner.write_stamp(self.repo, stamp)
        updates = (
            f"refs/heads/main {stamp.commit} refs/heads/main {'0' * 40}\n"
            f"refs/heads/local/gate-infra {self.local_commit} refs/heads/local/gate-infra {'0' * 40}\n"
        )

        authorization = self.runner.authorize_push(
            self.repo, "origin", "https://github.com/owner/repo", updates, env=self.env
        )

        self.assertFalse(authorization.allowed)
        self.assertIn("refs/heads/local/", authorization.message)

    def test_github_local_remote_ref_is_rejected_from_head_in_multi_ref_push(self):
        stamp = self._stamp()
        self.runner.write_stamp(self.repo, stamp)
        updates = (
            f"refs/heads/main {stamp.commit} refs/heads/main {'0' * 40}\n"
            f"HEAD {stamp.commit} refs/heads/local/gate-infra {'0' * 40}\n"
        )

        authorization = self.runner.authorize_push(
            self.repo, "origin", "https://github.com/owner/repo", updates, env=self.env
        )

        self.assertFalse(authorization.allowed)
        self.assertIn("refs/heads/local/", authorization.message)

    def test_pre_push_hook_rejects_every_local_ref_shape_in_multi_ref_push(self):
        stamp = self._stamp()
        self.runner.write_stamp(self.repo, stamp)
        valid_main = (
            f"refs/heads/main {stamp.commit} refs/heads/main {'0' * 40}\n"
        )
        local_updates = {
            "local source": (
                f"refs/heads/local/gate-infra {self.local_commit} "
                f"refs/heads/archive/gate {'0' * 40}\n"
            ),
            "local target": (
                f"HEAD {stamp.commit} refs/heads/local/gate-infra {'0' * 40}\n"
            ),
            "local target deletion": (
                f"(delete) {'0' * 40} refs/heads/local/gate-infra "
                f"{self.local_commit}\n"
            ),
        }

        for shape, local_update in local_updates.items():
            with self.subTest(shape=shape):
                completed = self._run_pre_push(valid_main + local_update)
                self.assertNotEqual(completed.returncode, 0)
                self.assertIn("refs/heads/local/", completed.stdout)

    def test_pre_push_hook_exempts_non_local_deletion_without_stamp(self):
        completed = self._run_pre_push(
            f"(delete) {'0' * 40} refs/heads/archive/old {self.local_commit}\n"
        )

        self.assertEqual(completed.returncode, 0, msg=completed.stderr)

    def test_release_tag_requires_full_stamp_for_the_exact_commit(self):
        default = self._stamp("default")
        self.runner.write_stamp(self.repo, default)
        update = f"refs/tags/v1.2.3 {default.commit} refs/tags/v1.2.3 {'0' * 40}\n"
        rejected = self.runner.authorize_push(
            self.repo, "origin", "git@github.com:owner/repo.git", update, env=self.env
        )
        self.assertFalse(rejected.allowed)

        full = self._stamp("full")
        self.runner.write_stamp(self.repo, full)
        accepted = self.runner.authorize_push(
            self.repo, "origin", "git@github.com:owner/repo.git", update, env=self.env
        )
        self.assertTrue(accepted.allowed)

        subprocess.run(["git", "commit", "--allow-empty", "--quiet", "-m", "same tree"], cwd=self.repo, check=True)
        different_commit = _run(self.repo, "rev-parse", "HEAD")
        different = f"refs/tags/v1.2.4 {different_commit} refs/tags/v1.2.4 {'0' * 40}\n"
        exact_rejected = self.runner.authorize_push(
            self.repo, "origin", "git@github.com:owner/repo.git", different, env=self.env
        )
        self.assertFalse(exact_rejected.allowed)
        self.assertIn("release commit", exact_rejected.message)


if __name__ == "__main__":
    unittest.main()
