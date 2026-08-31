import subprocess
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
EXPECTED_INDEX_MODES = {
    ".cargo/audit.toml": "100644",
    ".github/workflows/ci.yml": "100644",
    ".githooks/pre-push": "100755",
    "scripts/bump-workspace-version.py": "100755",
    "scripts/check-audit.sh": "100755",
    "scripts/check-doc-references.sh": "100755",
    "scripts/check-downstream-dependencies.py": "100755",
    "scripts/check-downstream-dependencies.sh": "100755",
    "scripts/check-feature-matrix.sh": "100755",
    "scripts/check-magnetar-live.sh": "100755",
    "scripts/check-manual-structure.py": "100755",
    "scripts/check-manual-translations.sh": "100755",
    "scripts/check-msrv.sh": "100755",
    "scripts/check-mysql.sh": "100755",
    "scripts/check-postgres.sh": "100755",
    "scripts/check-prose-dashes.sh": "100755",
    "scripts/gate-assets.json": "100644",
    "scripts/gate-runner.py": "100755",
    "scripts/gate-steps.json": "100644",
    "scripts/gate.sh": "100755",
    "scripts/install-gate.py": "100755",
    "scripts/release.sh": "100755",
    "scripts/tests/release-bump-smoke.sh": "100755",
    "scripts/tests/release-normal-smoke.sh": "100755",
    "scripts/tests/test_gate_install.py": "100644",
    "scripts/tests/test_gate_registry.py": "100644",
    "scripts/tests/test_gate_runner.py": "100644",
    "scripts/tests/test_gate_scoping.py": "100644",
    "scripts/tests/test_gate_stamp.py": "100644",
    "scripts/tests/test_repository_tracking.py": "100644",
}


class RepositoryTrackingTests(unittest.TestCase):
    def test_required_gate_and_workflow_assets_are_in_the_index_with_exact_modes(self):
        result = subprocess.run(
            ["git", "ls-files", "--stage", "--", *EXPECTED_INDEX_MODES],
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        actual_modes = {}
        nonzero_stages = []
        for line in result.stdout.splitlines():
            metadata, path = line.split("\t", maxsplit=1)
            mode, _object_id, stage = metadata.split()
            if stage != "0":
                nonzero_stages.append(path)
            actual_modes[path] = mode

        self.assertEqual(nonzero_stages, [], msg="required assets have unmerged index entries")
        self.assertEqual(actual_modes, EXPECTED_INDEX_MODES)


if __name__ == "__main__":
    unittest.main()
