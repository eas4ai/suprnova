#!/usr/bin/env python3
"""Tests for upgrade_guide_check.py (MAN-108): the bold lead of a changelog
entry is the heading to search for, whether or not the lead wraps onto a
second line of the changelog."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

TOOL = Path(__file__).resolve().with_name("upgrade_guide_check.py")

CHANGELOG = """# Changelog

## 4.0.0 - 2026-10-05

### Changed

- **Typed factory counts.** `count(n).create()` returns a `Vec`.
- **The CLI's `ssr:start`, `ssr:stop` and `ssr:check` run the application
  binary, and the CLI does not link the framework crate.** The three
  commands shell out.

### Fixed

- **A fix.** Not an upgrade entry.

## 3.2.1 - 2026-10-05

### Changed

- **An older entry.** Not in 4.0.0.
"""

WRAPPED_LEAD = (
    "The CLI's `ssr:start`, `ssr:stop` and `ssr:check` run the application "
    "binary, and the CLI does not link the framework crate."
)


def run(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(TOOL), "4.0.0", str(root)],
        capture_output=True,
        text=True,
    )


def write_repo(root: Path, chapter: str) -> None:
    (root / "manual").mkdir()
    (root / "CHANGELOG.md").write_text(CHANGELOG)
    (root / "manual" / "documentation.md").write_text(
        "- [Upgrading to 4.0](upgrade-4.0.md)\n"
    )
    (root / "manual" / "upgrade-4.0.md").write_text(chapter)


class WrappedLeads(unittest.TestCase):
    def test_a_lead_that_wraps_in_the_changelog_is_still_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_repo(root, "# Upgrading to 4.0\n\n### Typed factory counts.\n\nText.\n")
            result = run(root)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("1 of 2 entries covered", result.stdout)
        self.assertIn(f"no section for the changelog entry: {WRAPPED_LEAD}", result.stdout)

    def test_a_heading_equal_to_the_joined_lead_covers_the_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_repo(
                root,
                "# Upgrading to 4.0\n\n### Typed factory counts.\n\nText.\n\n"
                f"### {WRAPPED_LEAD}\n\nText.\n",
            )
            result = run(root)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("all 2 Changed and Removed entries covered", result.stdout)


if __name__ == "__main__":
    unittest.main()
