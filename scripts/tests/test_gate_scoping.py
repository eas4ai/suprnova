import dataclasses
import io
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager, redirect_stdout
import unittest
from pathlib import Path
from unittest import mock


_LOCAL_ROOT = Path(__file__).resolve().parents[2]
_RUNNER_PATH = _LOCAL_ROOT / "scripts" / "gate-runner.py"
_STRUCTURE_PATH = _LOCAL_ROOT / "scripts" / "check-manual-structure.py"
_LOCALES = ("de", "es", "fr", "ja", "pt-BR", "zh-Hans")


def _load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:  # pragma: no cover
        raise RuntimeError(f"failed to load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _git(repo, *argv):
    return subprocess.check_output(
        ["git", *argv], cwd=repo, text=True, stderr=subprocess.DEVNULL
    ).strip()


def _commit(repo, message):
    subprocess.run(["git", "add", "-A"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "--quiet", "-m", message], cwd=repo, check=True)
    return _git(repo, "rev-parse", "HEAD")


@contextmanager
def _working_directory(path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)


class ManualStructureTests(unittest.TestCase):
    def setUp(self):
        self.workspace = Path(tempfile.mkdtemp(prefix="manual-structure-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.structure = _load_module(
            f"suprnova_manual_structure_{id(self)}", _STRUCTURE_PATH
        )
        (self.workspace / "manual").mkdir()
        self.english = """# English title

## Section

- first
  1. nested

| Name | Value |
| --- | --- |
| alpha | beta |

```rust
fn main() {}
```

[Next](other.md#english-anchor)
"""
        self.locale = """# Localized title

## Localized section

- translated
  1. translated nested

| Name localized | Value localized |
| --- | --- |
| translated | translated |

```rust
fn main() {}
```

[Localized next](other.md#localized-anchor)
"""
        self._write_manual_fixture()

    def _registry(self, *, include_security=True):
        allowlist = [
            "manual/**",
            ".manual-translations.lock",
            "README.md",
            "CHANGELOG.md",
            "LICENSE",
        ]
        registered = [
            {"path": path, "checks": ["utf8", "doc-references", "prose-dashes"]}
            for path in ("README.md", "CHANGELOG.md", "LICENSE")
        ]
        if include_security:
            allowlist.append("SECURITY.md")
            registered.append(
                {
                    "path": "SECURITY.md",
                    "checks": ["utf8", "doc-references", "prose-dashes"],
                }
            )
        return {
            "schema": 1,
            "retention": {"max_runs": 50, "max_age_days": 30},
            "documentation_allowlist": allowlist,
            "registered_files": registered,
            "steps": [
                {
                    "id": "docs",
                    "name": "docs",
                    "tiers": ["default", "full", "docs"],
                    "argv": ["true"],
                    "timeout_seconds": 10,
                    "category": "docs",
                    "capabilities": [],
                }
            ],
        }

    def _write_manual_fixture(self):
        manual = self.workspace / "manual"
        (manual / "chapter.md").write_text(self.english, encoding="utf-8")
        (self.workspace / "CHANGELOG.md").write_text(
            "# Changelog\n\n- one\n", encoding="utf-8"
        )
        for name in ("README.md", "LICENSE", "SECURITY.md"):
            (self.workspace / name).write_text(f"{name}\n", encoding="utf-8")
        for locale in _LOCALES:
            locale_root = manual / locale
            locale_root.mkdir()
            (locale_root / "chapter.md").write_text(self.locale, encoding="utf-8")
            (locale_root / "changelog.md").write_text(
                "# Localized changelog\n\n- translated\n", encoding="utf-8"
            )
        registry_path = self.workspace / "scripts" / "gate-steps.json"
        registry_path.parent.mkdir()
        registry_path.write_text(
            json.dumps(self._registry(), indent=2) + "\n", encoding="utf-8"
        )

    def _problems(self):
        return self.structure.check_manual_structure(self.workspace)

    def _replace_chapter_tables(self, english_table, localized_table):
        english_table_current = """| Name | Value |
| --- | --- |
| alpha | beta |"""
        localized_table_current = """| Name localized | Value localized |
| --- | --- |
| translated | translated |"""
        (self.workspace / "manual/chapter.md").write_text(
            self.english.replace(english_table_current, english_table),
            encoding="utf-8",
        )
        for locale in _LOCALES:
            (self.workspace / "manual" / locale / "chapter.md").write_text(
                self.locale.replace(localized_table_current, localized_table),
                encoding="utf-8",
            )

    def _write_link_normalization_fixture(self, *, localized_changelog_target):
        (self.workspace / "CHANGELOG.md").write_text(
            """# Changelog

- [Parity](manual/parity.md#english)
- [External](HTTPS://EXAMPLE.COM/docs#english)
""",
            encoding="utf-8",
        )
        (self.workspace / "manual/parity.md").write_text(
            "# Parity\n", encoding="utf-8"
        )
        (self.workspace / "manual/documentation.md").write_text(
            """# Documentation

- [Changelog](../CHANGELOG.md#english)
- [Parity](parity.md#english)
- [External](HTTPS://EXAMPLE.COM/docs#english)
""",
            encoding="utf-8",
        )
        for locale in _LOCALES:
            locale_root = self.workspace / "manual" / locale
            (locale_root / "changelog.md").write_text(
                f"""# Localized changelog

- [Parity]({localized_changelog_target}#localized)
- [External](https://example.com/docs#localized)
""",
                encoding="utf-8",
            )
            (locale_root / "parity.md").write_text(
                "# Localized parity\n", encoding="utf-8"
            )
            (locale_root / "documentation.md").write_text(
                """# Localized documentation

- [Changelog](changelog.md#localized)
- [Parity](parity.md#localized)
- [External](https://example.com/docs#localized)
""",
                encoding="utf-8",
            )

    def _assert_problem(self, locale, file, kind):
        problems = self._problems()
        self.assertTrue(
            any(
                problem.locale == locale
                and problem.file == file
                and problem.kind == kind
                for problem in problems
            ),
            msg=[str(problem) for problem in problems],
        )
        rendered = "\n".join(str(problem) for problem in problems)
        self.assertIn(locale, rendered)
        self.assertIn(file, rendered)
        self.assertIn(kind, rendered)

    def test_matching_english_and_six_locale_mirrors_pass_without_lock_proof(self):
        (self.workspace / ".manual-translations.lock").write_text(
            "deliberately not a valid proof\n", encoding="utf-8"
        )
        self.assertEqual(self._problems(), [])

    def test_no_edge_table_mirror_passes(self):
        self._replace_chapter_tables(
            """Name | Value
--- | ---
alpha | beta""",
            """Localized name | Localized value
--- | ---
translated | translated""",
        )
        self.assertEqual(self._problems(), [])

    def test_no_edge_table_column_drift_is_reported(self):
        self._replace_chapter_tables(
            """Name | Value
--- | ---
alpha | beta""",
            """Localized name | Localized value
--- | ---
translated | inserted | translated""",
        )
        self._assert_problem("de", "chapter.md", "tables")

    def test_short_aligned_delimiter_cells_are_tables(self):
        self._replace_chapter_tables(
            """Name | Value
:- | -:
alpha | beta""",
            """Localized name | Localized value
:- | -:
translated | inserted | translated""",
        )
        self._assert_problem("fr", "chapter.md", "tables")

    def test_escaped_and_code_pipes_do_not_add_table_columns(self):
        self._replace_chapter_tables(
            """Name \\| alias | `Value | type`
--- | ---
escaped \\| label | `code | value`""",
            """Localized \\| name \\| alias | `Localized | value | type`
--- | ---
translated \\| label \\| extra | `localized | value | extra`""",
        )
        self.assertEqual(self._problems(), [])

    def test_prose_pipes_without_delimiter_are_not_tables(self):
        (self.workspace / "manual/chapter.md").write_text(
            self.english + "\n| English prose | with a separator\n",
            encoding="utf-8",
        )
        for locale in _LOCALES:
            (self.workspace / "manual" / locale / "chapter.md").write_text(
                self.locale + "\nLocalized prose | with a separator\n",
                encoding="utf-8",
            )
        self.assertEqual(self._problems(), [])

    def test_success_output_reports_actual_source_inventory_count(self):
        output = io.StringIO()
        with _working_directory(self.workspace), redirect_stdout(output):
            self.assertEqual(self.structure.main(), 0)
        self.assertIn(
            f"2 sources x {len(_LOCALES)} locales",
            output.getvalue(),
        )

    def test_exact_chapter_inventory_reports_missing_and_extra_locale_files(self):
        (self.workspace / "manual/de/chapter.md").unlink()
        (self.workspace / "manual/de/extra.md").write_text("# extra\n", encoding="utf-8")
        problems = self._problems()
        kinds = {(problem.locale, problem.file, problem.kind) for problem in problems}
        self.assertIn(("de", "chapter.md", "inventory-missing"), kinds)
        self.assertIn(("de", "extra.md", "inventory-extra"), kinds)

    def test_invalid_utf8_names_english_locale_and_registered_root_file(self):
        (self.workspace / "manual/fr/chapter.md").write_bytes(b"\xff")
        (self.workspace / "SECURITY.md").write_bytes(b"\xff")
        problems = self._problems()
        kinds = {(problem.locale, problem.file, problem.kind) for problem in problems}
        self.assertIn(("fr", "chapter.md", "utf8"), kinds)
        self.assertIn(("root", "SECURITY.md", "utf8"), kinds)

    def test_heading_level_sequence_mismatch_is_reported(self):
        path = self.workspace / "manual/ja/chapter.md"
        path.write_text(self.locale.replace("## Localized section", "### Localized section"), encoding="utf-8")
        self._assert_problem("ja", "chapter.md", "headings")

    def test_fenced_code_language_sequence_mismatch_is_reported(self):
        path = self.workspace / "manual/es/chapter.md"
        path.write_text(self.locale.replace("```rust", "```python"), encoding="utf-8")
        self._assert_problem("es", "chapter.md", "fences")

    def test_table_row_and_column_structure_mismatch_is_reported(self):
        path = self.workspace / "manual/pt-BR/chapter.md"
        path.write_text(
            self.locale.replace("| translated | translated |", "| translated |"),
            encoding="utf-8",
        )
        self._assert_problem("pt-BR", "chapter.md", "tables")

    def test_ordered_and_unordered_list_structure_mismatch_is_reported(self):
        path = self.workspace / "manual/zh-Hans/chapter.md"
        path.write_text(self.locale.replace("  1. translated nested", "  - translated nested"), encoding="utf-8")
        self._assert_problem("zh-Hans", "chapter.md", "lists")

    def test_normalized_link_targets_ignore_localized_fragments_but_not_paths(self):
        self.assertEqual(self._problems(), [])
        path = self.workspace / "manual/de/chapter.md"
        path.write_text(self.locale.replace("other.md#localized-anchor", "wrong.md#localized-anchor"), encoding="utf-8")
        self._assert_problem("de", "chapter.md", "links")

    def test_links_normalize_from_physical_locations_for_all_six_locales(self):
        self._write_link_normalization_fixture(
            localized_changelog_target="parity.md"
        )

        link_problems = [
            problem for problem in self._problems() if problem.kind == "links"
        ]
        self.assertEqual(
            link_problems,
            [],
            msg=[str(problem) for problem in link_problems],
        )

    def test_root_relative_literal_in_localized_changelog_fails_for_all_six_locales(
        self,
    ):
        self._write_link_normalization_fixture(
            localized_changelog_target="manual/parity.md"
        )

        link_problems = [
            problem for problem in self._problems() if problem.kind == "links"
        ]
        self.assertEqual(
            [
                (problem.locale, problem.file, problem.kind)
                for problem in link_problems
            ],
            [(locale, "changelog.md", "links") for locale in _LOCALES],
            msg=[str(problem) for problem in link_problems],
        )

    def test_appended_locale_section_is_reported_independently_of_translation_lock(self):
        path = self.workspace / "manual/fr/chapter.md"
        path.write_text(self.locale + "\n### Appended\n\n- extra\n", encoding="utf-8")
        problems = self._problems()
        self.assertTrue(
            any(problem.locale == "fr" and problem.file == "chapter.md" for problem in problems),
            msg=[str(problem) for problem in problems],
        )


class GitFixture(unittest.TestCase):
    def setUp(self):
        self.runner = _load_module(f"suprnova_gate_runner_scope_{id(self)}", _RUNNER_PATH)
        self.workspace = Path(tempfile.mkdtemp(prefix="gate-scope-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.workspace, ignore_errors=True))
        self.repo = self.workspace / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "--quiet", "-b", "main"], cwd=self.repo, check=True)
        subprocess.run(["git", "config", "user.name", "Gate Test"], cwd=self.repo, check=True)
        subprocess.run(["git", "config", "user.email", "gate@example.com"], cwd=self.repo, check=True)
        subprocess.run(["git", "config", "core.hooksPath", ".githooks"], cwd=self.repo, check=True)
        self.bin_dir = self.workspace / "bin"
        self.bin_dir.mkdir()
        rustc = self.bin_dir / "rustc"
        rustc.write_text("#!/bin/sh\nprintf 'rustc 1.94.0 (scope-test)\\n'\n", encoding="utf-8")
        rustc.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": str(self.bin_dir) + os.pathsep + os.environ.get("PATH", ""),
        }

        (self.repo / ".gitignore").write_text(
            "/scripts/\n/.githooks/\n/.cargo/\n", encoding="utf-8"
        )
        (self.repo / "Cargo.toml").write_text("[workspace]\nmembers = []\n", encoding="utf-8")
        (self.repo / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.94.0"\n', encoding="utf-8")
        (self.repo / "src").mkdir()
        (self.repo / "src/lib.rs").write_text("pub fn base() {}\n", encoding="utf-8")
        (self.repo / "manual").mkdir()
        (self.repo / "manual/chapter.md").write_text("# Chapter\n", encoding="utf-8")
        for name in ("README.md", "CHANGELOG.md", "LICENSE", ".manual-translations.lock"):
            (self.repo / name).write_text(f"{name}\n", encoding="utf-8")
        self.base_commit = _commit(self.repo, "public base")
        self.base_tree = _git(self.repo, "rev-parse", "HEAD^{tree}")
        self._install_local_assets()

    def _install_local_assets(self):
        subprocess.run(["git", "switch", "--quiet", "-c", "local/gate-infra"], cwd=self.repo, check=True)
        manifest = json.loads(
            (_LOCAL_ROOT / "scripts/gate-assets.json").read_text(encoding="utf-8")
        )
        assets = manifest["assets"]
        for relative in assets:
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(_LOCAL_ROOT / relative, destination)
        subprocess.run(["git", "add", "-f", *assets], cwd=self.repo, check=True)
        subprocess.run(["git", "commit", "--quiet", "-m", "local tooling"], cwd=self.repo, check=True)
        self.local_commit = _git(self.repo, "rev-parse", "HEAD")
        subprocess.run(["git", "switch", "--quiet", "main"], cwd=self.repo, check=True)
        hashes = {}
        for relative in assets:
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            committed = subprocess.check_output(
                ["git", "show", f"{self.local_commit}:{relative}"], cwd=self.repo
            )
            mode = _git(
                self.repo, "ls-tree", self.local_commit, "--", relative
            ).split(maxsplit=1)[0]
            if mode not in {"100644", "100755"}:
                raise AssertionError(
                    f"local tooling commit asset is not a regular file: {relative}"
                )
            destination.write_bytes(committed)
            destination.chmod(0o755 if mode == "100755" else 0o644)
            hashes[relative] = hashlib.sha256(committed).hexdigest()
        record = {
            "schema": 2,
            "branch": "local/gate-infra",
            "commit": self.local_commit,
            "assets": hashes,
            "capabilities": manifest["capabilities"],
        }
        record_path = self.repo / _git(
            self.repo, "rev-parse", "--git-path", "suprnova-local-gate.json"
        )
        record_path.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")

    def _registry(self):
        return self.runner.load_registry(self.repo)

    def _stamp(self, *, commit=None, tree=None, tier="default", provenance=None):
        return self.runner.GateStamp(
            schema=2,
            tier=tier,
            tree=tree or self.base_tree,
            commit=commit or self.base_commit,
            toolchain="rustc 1.94.0 (scope-test)",
            steps_hash=self.runner.compute_steps_hash(self.repo, tier),
            finished_at="2026-08-24T12:00:00Z",
            run_id="base-run",
            code_provenance=provenance,
            local_tooling_commit=self.local_commit,
        )

    def _commit_docs(self, text="docs change"):
        path = self.repo / "manual/chapter.md"
        path.write_text(path.read_text(encoding="utf-8") + f"\n{text}\n", encoding="utf-8")
        return _commit(self.repo, text)

    def _plan(self, *, stamp=None, delta_base_tree=None):
        return self.runner.select_gate_plan(
            self.repo,
            self._registry(),
            tier="default",
            env=self.env,
            base_stamp=stamp,
            delta_base_tree=delta_base_tree,
        )


class DeltaClassificationTests(GitFixture):
    def _classify_head(self):
        head_tree = _git(self.repo, "rev-parse", "HEAD^{tree}")
        with _working_directory(self.repo):
            return self.runner.classify_delta(self.base_tree, head_tree)

    def test_empty_delta_is_fail_closed_empty(self):
        self.assertIs(self._classify_head(), self.runner.DeltaClass.EMPTY)

    def test_docs_only_delta_uses_only_explicit_allowlist(self):
        self._commit_docs()
        self.assertIs(self._classify_head(), self.runner.DeltaClass.DOCS_ONLY)

    def test_code_and_unknown_top_level_changes_are_code(self):
        for relative in ("src/lib.rs", "new-top-level/data.md"):
            with self.subTest(relative=relative):
                subprocess.run(["git", "reset", "--hard", "--quiet", self.base_commit], cwd=self.repo, check=True)
                path = self.repo / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("changed\n", encoding="utf-8")
                _commit(self.repo, relative)
                self.assertIs(self._classify_head(), self.runner.DeltaClass.CODE)

    def test_mixed_delta_is_not_docs_only(self):
        (self.repo / "manual/chapter.md").write_text("# Changed\n", encoding="utf-8")
        (self.repo / "src/lib.rs").write_text("pub fn changed() {}\n", encoding="utf-8")
        _commit(self.repo, "mixed")
        self.assertIs(self._classify_head(), self.runner.DeltaClass.MIXED)

    def test_registered_root_file_is_docs_but_unregistered_root_markdown_is_code(self):
        (self.repo / "README.md").write_text("registered docs\n", encoding="utf-8")
        _commit(self.repo, "registered")
        self.assertIs(self._classify_head(), self.runner.DeltaClass.DOCS_ONLY)
        subprocess.run(["git", "reset", "--hard", "--quiet", self.base_commit], cwd=self.repo, check=True)
        (self.repo / "SECURITY.md").write_text("not registered\n", encoding="utf-8")
        _commit(self.repo, "unregistered")
        self.assertIs(self._classify_head(), self.runner.DeltaClass.CODE)

    def test_allowlist_edit_invalidates_steps_hash(self):
        before = self.runner.compute_steps_hash(self.repo, "default")
        path = self.repo / "scripts/gate-steps.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["documentation_allowlist"].append("SECURITY.md")
        payload["registered_files"].append(
            {
                "path": "SECURITY.md",
                "checks": ["utf8", "doc-references", "prose-dashes"],
            }
        )
        path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        self.assertNotEqual(before, self.runner.compute_steps_hash(self.repo, "default"))

    def test_checker_and_hook_byte_drift_invalidate_install_and_steps_hash(self):
        required_assets = {
            "scripts/gate-steps.json",
            "scripts/check-manual-structure.py",
            ".githooks/pre-push",
        }
        install = self.runner.verify_local_install(self.repo)
        self.assertTrue(required_assets.issubset(install.assets))

        for relative in ("scripts/check-manual-structure.py", ".githooks/pre-push"):
            with self.subTest(relative=relative):
                path = self.repo / relative
                installed = path.read_bytes()
                before = self.runner.compute_steps_hash(self.repo, "default")
                path.write_bytes(installed + b"\nbyte drift\n")
                self.assertNotEqual(
                    before,
                    self.runner.compute_steps_hash(self.repo, "default"),
                )
                with self.assertRaises(EnvironmentError) as raised:
                    self.runner.verify_local_install(self.repo)
                self.assertIn(relative, str(raised.exception))
                path.write_bytes(installed)
                self.runner.verify_local_install(self.repo)

    def test_allowlisted_root_without_registered_checks_is_definition_error(self):
        path = self.repo / "scripts/gate-steps.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["documentation_allowlist"].append("SECURITY.md")
        path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(EnvironmentError, "registered"):
            self.runner.load_registry(self.repo)

    def test_registered_root_path_cannot_escape_with_backslashes(self):
        path = self.repo / "scripts/gate-steps.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        payload["documentation_allowlist"].append("..\\SECURITY.md")
        payload["registered_files"].append(
            {
                "path": "..\\SECURITY.md",
                "checks": ["utf8", "doc-references", "prose-dashes"],
            }
        )
        path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(EnvironmentError, "registered"):
            self.runner.load_registry(self.repo)


class InheritanceTests(GitFixture):
    def test_resolve_code_provenance_prefers_original_tree(self):
        self.assertEqual(self.runner.resolve_code_provenance(self._stamp()), self.base_tree)
        inherited = self._stamp(provenance="a" * 40)
        self.assertEqual(self.runner.resolve_code_provenance(inherited), "a" * 40)

    def test_valid_docs_only_descendant_inherits_exact_base(self):
        stamp = self._stamp()
        self._commit_docs()
        decision = self.runner.validate_documentation_inheritance(
            self.repo, stamp, env=self.env
        )
        self.assertTrue(decision.valid, msg=decision.message)
        self.assertEqual(decision.code_provenance, self.base_tree)
        self.assertIs(decision.delta_class, self.runner.DeltaClass.DOCS_ONLY)

    def test_two_docs_commits_preserve_original_provenance(self):
        base = self._stamp()
        self._commit_docs("docs one")
        first = self.runner.validate_documentation_inheritance(self.repo, base, env=self.env)
        inherited_stamp = self.runner.build_stamp(
            self.repo,
            tier="default",
            run_id="docs-one",
            code_provenance=first.code_provenance,
            env=self.env,
        )
        self._commit_docs("docs two")
        second = self.runner.validate_documentation_inheritance(
            self.repo, inherited_stamp, env=self.env
        )
        self.assertTrue(second.valid, msg=second.message)
        self.assertEqual(second.code_provenance, self.base_tree)

    def test_later_code_gate_resets_provenance(self):
        self._commit_docs()
        inherited = self.runner.build_stamp(
            self.repo,
            tier="default",
            run_id="docs",
            code_provenance=self.base_tree,
            env=self.env,
        )
        (self.repo / "src/lib.rs").write_text("pub fn later() {}\n", encoding="utf-8")
        _commit(self.repo, "later code")
        plan = self._plan(stamp=inherited)
        self.assertFalse(plan.inherited)
        reset = self.runner.build_stamp(
            self.repo,
            tier="default",
            run_id="code",
            code_provenance=plan.code_provenance,
            env=self.env,
        )
        self.assertIsNone(reset.code_provenance)

    def test_code_added_then_reverted_may_inherit_but_unreverted_code_cannot(self):
        base = self._stamp()
        (self.repo / "src/lib.rs").write_text("pub fn temporary() {}\n", encoding="utf-8")
        _commit(self.repo, "temporary code")
        self._commit_docs("docs with code")
        unreverted = self.runner.validate_documentation_inheritance(
            self.repo, base, env=self.env
        )
        self.assertFalse(unreverted.valid)
        self.assertIs(unreverted.delta_class, self.runner.DeltaClass.MIXED)
        subprocess.run(
            ["git", "checkout", self.base_commit, "--", "src/lib.rs"],
            cwd=self.repo,
            check=True,
        )
        _commit(self.repo, "revert code")
        reverted = self.runner.validate_documentation_inheritance(
            self.repo, base, env=self.env
        )
        self.assertTrue(reverted.valid, msg=reverted.message)
        self.assertEqual(reverted.code_provenance, self.base_tree)

    def test_wrong_or_nonancestor_base_fails_closed(self):
        self._commit_docs()
        invalid_tier = dataclasses.replace(self._stamp(), tier="docs")
        self.assertFalse(
            self.runner.validate_documentation_inheritance(
                self.repo, invalid_tier, env=self.env
            ).valid
        )
        subprocess.run(["git", "branch", "side", self.base_commit], cwd=self.repo, check=True)
        subprocess.run(["git", "switch", "--quiet", "side"], cwd=self.repo, check=True)
        subprocess.run(
            ["git", "commit", "--allow-empty", "--quiet", "-m", "side"],
            cwd=self.repo,
            check=True,
        )
        side_commit = _git(self.repo, "rev-parse", "HEAD")
        subprocess.run(["git", "switch", "--quiet", "main"], cwd=self.repo, check=True)
        nonancestor = self._stamp(commit=side_commit)
        decision = self.runner.validate_documentation_inheritance(
            self.repo, nonancestor, env=self.env
        )
        self.assertFalse(decision.valid)
        self.assertIn("ancestor", decision.message)

    def test_base_stamp_tree_must_match_its_commit_tree(self):
        self._commit_docs()
        stamp = dataclasses.replace(self._stamp(), tree="0" * 40)
        decision = self.runner.validate_documentation_inheritance(
            self.repo, stamp, env=self.env
        )
        self.assertFalse(decision.valid)
        self.assertIn("does not resolve", decision.message)

    def test_missing_original_tree_object_fails_closed(self):
        self._commit_docs()
        stamp = dataclasses.replace(self._stamp(), code_provenance="f" * 40)
        decision = self.runner.validate_documentation_inheritance(
            self.repo, stamp, env=self.env
        )
        self.assertFalse(decision.valid)
        self.assertIn("tree", decision.message)

    def test_invalid_hash_toolchain_and_install_each_fail_closed(self):
        self._commit_docs()
        cases = [
            dataclasses.replace(self._stamp(), steps_hash="0" * 64),
            dataclasses.replace(self._stamp(), toolchain="rustc wrong"),
            dataclasses.replace(self._stamp(), local_tooling_commit="0" * 40),
        ]
        for stamp in cases:
            with self.subTest(stamp=stamp):
                self.assertFalse(
                    self.runner.validate_documentation_inheritance(
                        self.repo, stamp, env=self.env
                    ).valid
                )
        (self.repo / "scripts/gate-steps.json").write_text("drift\n", encoding="utf-8")
        decision = self.runner.validate_documentation_inheritance(
            self.repo, self._stamp(), env=self.env
        )
        self.assertFalse(decision.valid)
        self.assertIn("drift", decision.message)

    def test_empty_delta_and_dirty_tree_do_not_inherit(self):
        empty = self.runner.validate_documentation_inheritance(
            self.repo, self._stamp(), env=self.env
        )
        self.assertFalse(empty.valid)
        self.assertIs(empty.delta_class, self.runner.DeltaClass.EMPTY)
        self._commit_docs()
        (self.repo / "README.md").write_text("dirty\n", encoding="utf-8")
        dirty = self.runner.validate_documentation_inheritance(
            self.repo, self._stamp(), env=self.env
        )
        self.assertFalse(dirty.valid)
        self.assertIn("dirty", dirty.message)


class GateSelectionTests(GitFixture):
    def _ids_for_tiers(self, *tiers):
        tiers = set(tiers)
        return tuple(
            step.id
            for step in self._registry().steps
            if tiers.intersection(step.tiers)
        )

    def test_docs_only_uses_full_docs_subset_and_marks_code_not_executed(self):
        base = self._stamp()
        self._commit_docs()
        plan = self._plan(stamp=base)
        self.assertTrue(plan.inherited)
        self.assertEqual(tuple(step.id for step in plan.steps), self._ids_for_tiers("docs"))
        self.assertEqual(plan.code_provenance, self.base_tree)
        self.assertIn("inherited", plan.message)
        self.assertIn("not executed", plan.message)

    def test_docs_subset_includes_dash_lock_mirror_and_registered_checks(self):
        self._commit_docs()
        selected = {
            step.id for step in self._plan(stamp=self._stamp()).steps
        }
        self.assertTrue(
            {
                "doc-references",
                "prose-dashes",
                "translation-lock",
                "manual-structure",
            }.issubset(selected)
        )

    def test_planted_em_dash_fails_the_real_prose_checker(self):
        (self.repo / "manual/chapter.md").write_text(
            "# Chapter\n\nplanted \N{EM DASH} drift\n", encoding="utf-8"
        )
        _commit(self.repo, "em dash")
        completed = subprocess.run(
            [str(_LOCAL_ROOT / "scripts/check-prose-dashes.sh")],
            cwd=self.repo,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("em dash", completed.stderr)

    def test_stale_locale_blob_fails_the_real_translation_lock_checker(self):
        for locale in _LOCALES:
            locale_root = self.repo / "manual" / locale
            locale_root.mkdir()
            (locale_root / "chapter.md").write_text(
                f"# {locale} chapter\n", encoding="utf-8"
            )
            (locale_root / "changelog.md").write_text(
                f"# {locale} changelog\n", encoding="utf-8"
            )
        checker = str(_LOCAL_ROOT / "scripts/check-manual-translations.sh")
        stamped = subprocess.run(
            [checker, "--stamp"],
            cwd=self.repo,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(stamped.returncode, 0, msg=stamped.stderr)
        (self.repo / "manual/de/chapter.md").write_text(
            "# de stale bytes\n", encoding="utf-8"
        )
        completed = subprocess.run(
            [checker],
            cwd=self.repo,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("STALE    manual/de/chapter.md", completed.stdout)

    def test_docs_execution_writes_default_stamp_with_original_provenance(self):
        self.runner.write_stamp(self.repo, self._stamp())
        self._commit_docs()

        def pass_step(step, context):
            log = context.run_dir / f"{step.id}.log"
            log.write_text("pass\n", encoding="utf-8")
            return self.runner.StepResult(
                step=step.id,
                tier=context.tier,
                outcome=self.runner.Outcome.PASS,
                seconds=0.01,
                exit_code=0,
                argv=step.argv,
                log_path=str(log),
                started=True,
            )

        output = io.StringIO()
        with (
            mock.patch.object(self.runner, "_preflight", return_value=None),
            mock.patch.object(self.runner, "run_step", side_effect=pass_step),
            redirect_stdout(output),
        ):
            result = self.runner.execute_gate(
                self.repo,
                self._registry(),
                tier="default",
                named_step=None,
                env=self.env,
            )
        self.assertEqual(result, 0, msg=output.getvalue())
        stamp = self.runner.load_stamp(self.repo)
        self.assertIsNotNone(stamp)
        self.assertEqual(stamp.tier, "default")
        self.assertEqual(stamp.code_provenance, self.base_tree)
        self.assertIn("inherited", output.getvalue())
        self.assertIn("not executed", output.getvalue())

    def test_missing_or_invalid_base_runs_default_plus_docs_for_docs_delta(self):
        self._commit_docs()
        expected = self._ids_for_tiers("default", "docs")
        missing = self._plan(stamp=None, delta_base_tree=self.base_tree)
        self.assertEqual(tuple(step.id for step in missing.steps), expected)
        invalid = self._plan(
            stamp=dataclasses.replace(self._stamp(), steps_hash="0" * 64),
            delta_base_tree=self.base_tree,
        )
        self.assertEqual(tuple(step.id for step in invalid.steps), expected)

    def test_code_only_runs_exact_default_step_ids(self):
        (self.repo / "src/lib.rs").write_text("pub fn changed() {}\n", encoding="utf-8")
        _commit(self.repo, "code")
        plan = self._plan(stamp=self._stamp(), delta_base_tree=self.base_tree)
        self.assertEqual(tuple(step.id for step in plan.steps), self._ids_for_tiers("default"))
        self.assertIsNone(plan.code_provenance)

    def test_actual_mixed_selection_contains_every_default_and_docs_step_id(self):
        (self.repo / "src/lib.rs").write_text("pub fn changed() {}\n", encoding="utf-8")
        (self.repo / "manual/chapter.md").write_text("# changed\n", encoding="utf-8")
        _commit(self.repo, "mixed")
        plan = self._plan(stamp=self._stamp(), delta_base_tree=self.base_tree)
        expected = self._ids_for_tiers("default", "docs")
        self.assertIs(plan.delta_class, self.runner.DeltaClass.MIXED)
        self.assertEqual(tuple(step.id for step in plan.steps), expected)
        self.assertTrue(set(self._ids_for_tiers("default")).issubset({step.id for step in plan.steps}))
        self.assertTrue(set(self._ids_for_tiers("docs")).issubset({step.id for step in plan.steps}))


class PrePushScopingTests(GitFixture):
    def test_non_head_tip_never_runs_gate_automatically(self):
        self._commit_docs()
        update = (
            f"refs/heads/old {self.base_commit} refs/heads/old {'0' * 40}\n"
        )
        with mock.patch.object(self.runner, "execute_gate") as execute:
            decision = self.runner.authorize_push(
                self.repo,
                "origin",
                "https://github.com/owner/repo",
                update,
                env=self.env,
                auto_gate=True,
            )
        self.assertFalse(decision.allowed)
        self.assertIn("non-HEAD", decision.message)
        execute.assert_not_called()

    def test_current_head_can_gate_automatically_with_remote_delta_tree(self):
        self._commit_docs()
        head = _git(self.repo, "rev-parse", "HEAD")
        update = f"refs/heads/main {head} refs/heads/main {self.base_commit}\n"

        def fake_gate(repo, registry, **kwargs):
            self.assertEqual(kwargs["delta_base_tree"], self.base_tree)
            stamp = self.runner.build_stamp(
                repo,
                tier="default",
                run_id="automatic",
                code_provenance=self.base_tree,
                env=self.env,
            )
            self.runner.write_stamp(repo, stamp)
            return 0

        with mock.patch.object(self.runner, "execute_gate", side_effect=fake_gate) as execute:
            decision = self.runner.authorize_push(
                self.repo,
                "origin",
                "https://github.com/owner/repo",
                update,
                env=self.env,
                auto_gate=True,
            )
        self.assertTrue(decision.allowed, msg=decision.message)
        execute.assert_called_once()

    def test_dirty_current_head_consumes_or_writes_no_stamp(self):
        stamp = self._stamp()
        self.runner.write_stamp(self.repo, stamp)
        (self.repo / "README.md").write_text("dirty\n", encoding="utf-8")
        head = _git(self.repo, "rev-parse", "HEAD")
        update = f"refs/heads/main {head} refs/heads/main {'0' * 40}\n"
        with mock.patch.object(self.runner, "execute_gate") as execute:
            decision = self.runner.authorize_push(
                self.repo,
                "origin",
                "https://github.com/owner/repo",
                update,
                env=self.env,
                auto_gate=True,
            )
        self.assertFalse(decision.allowed)
        self.assertEqual(self.runner.load_stamp(self.repo), stamp)
        execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
