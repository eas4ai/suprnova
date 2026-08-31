import json
import subprocess
import unittest
from pathlib import Path
import stat


_LOCAL_ROOT = Path(__file__).resolve().parents[2]


class GateRegistryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest_path = _LOCAL_ROOT / "scripts" / "gate-assets.json"
        with open(self.manifest_path, "r", encoding="utf-8") as handle:
            self.manifest = json.load(handle)

        self.assets = set(self.manifest["assets"])

    @staticmethod
    def _tracked_files_in_index(repo: Path) -> set[str]:
        out = subprocess.check_output(
            ["git", "ls-files", "--stage"], cwd=repo, text=True
        )
        tracked = set()
        for line in out.splitlines():
            metadata, path = line.split("\t", maxsplit=1)
            _mode, _object_id, stage = metadata.split()
            if stage == "0":
                tracked.add(path)
        return tracked

    @staticmethod
    def _is_command_like(path: Path) -> bool:
        if not path.is_file():
            return False

        mode = path.stat().st_mode
        if mode & stat.S_IXUSR:
            return True
        first_line = path.read_bytes().splitlines()[:1]
        if not first_line:
            return False
        return first_line[0].startswith(b"#!")

    def _command_assets(self, root: Path) -> set[str]:
        result: set[str] = set()
        for pattern in ("scripts", ".githooks", ".cargo"):
            base = root / pattern
            if not base.is_dir():
                continue
            for file in base.rglob("*"):
                if not self._is_command_like(file):
                    continue
                result.add(file.relative_to(root).as_posix())
        return result

    def test_index_tracks_all_manifest_assets(self):
        tracked = self._tracked_files_in_index(_LOCAL_ROOT)
        missing = sorted(self.assets - tracked)
        self.assertEqual(missing, [], msg=f"index missing manifest asset(s): {missing}")

    def test_commands_in_scripts_or_githooks_are_in_manifest(self):
        discovered = self._command_assets(_LOCAL_ROOT)
        missing = sorted(discovered - self.assets)
        self.assertEqual(missing, [], msg=f"manifest missing command-like asset(s): {missing}")

    def test_magnetar_all_feature_tests_are_in_gate_and_manual_ci(self):
        expected_argv = [
            "cargo",
            "test",
            "-p",
            "suprnova-magnetar",
            "--all-features",
            "--tests",
            "--no-fail-fast",
        ]
        expected_command = " ".join(expected_argv)

        registry_path = _LOCAL_ROOT / "scripts" / "gate-steps.json"
        with open(registry_path, "r", encoding="utf-8") as handle:
            registry = json.load(handle)
        matching_steps = [
            step for step in registry["steps"] if step.get("argv") == expected_argv
        ]

        self.assertEqual(
            len(matching_steps),
            1,
            msg=f"gate must register exactly one Magnetar step: {expected_command}",
        )
        self.assertEqual(matching_steps[0]["tiers"], ["default", "full"])

        workflow_path = _LOCAL_ROOT / ".github" / "workflows" / "ci.yml"
        workflow = workflow_path.read_text(encoding="utf-8")
        workflow_lines = [line.strip() for line in workflow.splitlines()]
        expected_step_name = "name: cargo test -p suprnova-magnetar --all-features"
        expected_run = f"run: {expected_command}"
        self.assertEqual(workflow_lines.count(f"- {expected_step_name}"), 1)
        step_index = workflow_lines.index(f"- {expected_step_name}")
        self.assertEqual(workflow_lines[step_index + 1], expected_run)
        self.assertEqual(workflow_lines.count(expected_run), 1)


if __name__ == "__main__":
    unittest.main()
