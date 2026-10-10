#!/usr/bin/env python3
"""Tests for manual_check.py's stale-surface refusal (MAN-002).

Run: python3 feature-map/tools/test_manual_check.py. The tool imports
markdown-it-py and tree-sitter, so it runs under the venv manual_check_env.sh
creates at target/feature-map/venv when that exists, else under
TEST_MANUAL_CHECK_PYTHON, else under this interpreter.

The tool takes the repository root as its argument, so each test builds a
small git repository with its own feature-map/meta.json and runs the tool
against it. MAN-002: when meta.json's source_rev is not the last commit that
touched the source paths, the tool refuses to run before it reads anything
else; when it is, the tool runs.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().parent / "manual_check.py"
VENV_PYTHON = TOOL.parents[2] / "target" / "feature-map" / "venv" / "bin" / "python"
PYTHON = os.environ.get("TEST_MANUAL_CHECK_PYTHON") or (str(VENV_PYTHON) if VENV_PYTHON.exists() else sys.executable)


def git(root, *args):
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=True).stdout.strip()


def repository(root, source_rev):
    """A repository with one source commit, a surface and a manual, stamped source_rev."""
    (root / "src").mkdir()
    (root / "src" / "lib.rs").write_text("pub fn f() {}\n")
    (root / "feature-map").mkdir()
    (root / "feature-map" / "surface.jsonl").write_text("")
    (root / "feature-map" / "exclusions.json").write_text(json.dumps({"build_time_env": {}}))
    (root / "manual").mkdir()
    git(root, "init", "-q")
    git(root, "-c", "user.email=t@example.com", "-c", "user.name=t", "add", ".")
    git(root, "-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-q", "-m", "source")
    head = git(root, "log", "-1", "--format=%h", "--", "src")
    rev = head if source_rev == "current" else source_rev
    (root / "feature-map" / "meta.json").write_text(json.dumps({"source_paths": ["src"], "source_rev": rev, "records": 0}))
    return head


def run_tool(root):
    """Run the tool against `root`, writing its findings beside the repository."""
    out = root.parent / f"{root.name}-findings.jsonl"
    return subprocess.run([PYTHON, str(TOOL), str(root), str(out)], capture_output=True, text=True)


class StaleSurfaceRefusal(unittest.TestCase):
    def test_a_map_stamped_behind_the_source_is_refused_before_anything_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            head = repository(root, "0000000")
            result = run_tool(root)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("surface built from 0000000", result.stderr)
        self.assertIn(f"source is now at {head}", result.stderr)
        self.assertEqual(result.stdout, "", "the refusal must come before any finding or summary")

    def test_a_map_stamped_at_the_source_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repository(root, "current")
            result = run_tool(root)
        self.assertNotIn("surface built from", result.stderr)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
