"""Content identity must change when consumed checkout bytes change."""

from pathlib import Path
import subprocess
import tempfile
import unittest

from live_contract_checks.acceptance import Incomplete
from live_contract_checks.identity import checkout_identity


class IdentityTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="live-identity-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        (self.root / "source.rs").write_text("const VALUE: u8 = 1;\n")
        (self.root / ".gitignore").write_text("target/\n")
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)

    def test_dirty_and_untracked_source_change_identity_without_a_commit(self):
        first = checkout_identity(self.root)
        (self.root / "source.rs").write_text("const VALUE: u8 = 2;\n")
        second = checkout_identity(self.root)
        self.assertNotEqual(first, second)
        (self.root / "new.rs").write_text("// new source\n")
        self.assertNotEqual(second, checkout_identity(self.root))

    def test_ignored_build_outputs_do_not_change_checkout_source_identity(self):
        first = checkout_identity(self.root)
        (self.root / "target").mkdir()
        (self.root / "target/output").write_bytes(b"build output")
        self.assertEqual(first, checkout_identity(self.root))

    def test_equivalent_checkout_at_another_path_has_the_same_identity(self):
        with tempfile.TemporaryDirectory(prefix="live-moved-test-") as other:
            other = Path(other)
            subprocess.run(["git", "init", "-q", str(other)], check=True)
            for name in ["Cargo.toml", "source.rs", ".gitignore"]:
                (other / name).write_bytes((self.root / name).read_bytes())
            subprocess.run(["git", "-C", str(other), "add", "."], check=True)
            self.assertEqual(checkout_identity(self.root), checkout_identity(other))

    def test_deletion_and_execute_permission_change_identity(self):
        first = checkout_identity(self.root)
        (self.root / "source.rs").chmod(0o755)
        second = checkout_identity(self.root)
        self.assertNotEqual(first, second)
        (self.root / "source.rs").unlink()
        self.assertNotEqual(second, checkout_identity(self.root))

    def test_external_symlink_is_not_claimed_as_captured_source(self):
        (self.root / "external.rs").symlink_to("/etc/hosts")
        with self.assertRaises(Incomplete):
            checkout_identity(self.root)

    def test_missing_checkout_is_unverified(self):
        with self.assertRaises(Incomplete):
            checkout_identity(self.root / "missing")

    def test_explicit_ignored_lockfile_changes_the_captured_identity(self):
        (self.root / ".gitignore").write_text("Cargo.lock\n")
        lock = self.root / "Cargo.lock"
        lock.write_text("version = 3\n")
        paths = ("source.rs", "Cargo.lock")
        first = checkout_identity(self.root, paths)
        lock.write_text("version = 4\n")
        self.assertNotEqual(first, checkout_identity(self.root, paths))

    def test_explicit_ignored_symlink_cannot_hide_from_inventory(self):
        (self.root / ".gitignore").write_text("Cargo.lock\n")
        (self.root / "Cargo.lock").symlink_to("source.rs")
        with self.assertRaises(Incomplete):
            checkout_identity(self.root, ("source.rs", "Cargo.lock"))


if __name__ == "__main__":
    unittest.main()
