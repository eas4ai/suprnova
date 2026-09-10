import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path


_LOCAL_ROOT = Path(__file__).resolve().parents[2]
_CHECKER_PATH = _LOCAL_ROOT / "scripts" / "check-manual-structure.py"

_CHANGELOG = "# Changelog\n\nNothing to declare.\n"

# Two distinct chapter names the span tests use to tell one chapter's problems
# from another's. Neither carries any special meaning to the checker.
_CHAPTER = "render-cache.md"
_OTHER_CHAPTER = "cli.md"


def _load_checker():
    spec = importlib.util.spec_from_file_location(
        "suprnova_manual_structure", _CHECKER_PATH
    )
    if spec is None or spec.loader is None:  # pragma: no cover
        raise RuntimeError("failed to load manual structure checker module")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class ManualCodeSpanTests(unittest.TestCase):
    """The inline code span mirror rule, exercised through the public entry point."""

    def setUp(self):
        self.checker = _load_checker()
        self.root = Path(tempfile.mkdtemp(prefix="manual-structure-test-"))
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "gate-steps.json").write_text(
            json.dumps({"registered_files": []}) + "\n", encoding="utf-8"
        )
        (self.root / "CHANGELOG.md").write_text(_CHANGELOG, encoding="utf-8")

    def _check(self, chapters):
        """Write `{name: (english, mirror)}`, give every locale the mirror, check."""

        manual = self.root / "manual"
        manual.mkdir(exist_ok=True)
        for name, (english, _mirror) in chapters.items():
            (manual / name).write_text(english, encoding="utf-8")
        for locale in self.checker.LOCALES:
            directory = manual / locale
            directory.mkdir(parents=True, exist_ok=True)
            for name, (_english, mirror) in chapters.items():
                (directory / name).write_text(mirror, encoding="utf-8")
            (directory / "changelog.md").write_text(_CHANGELOG, encoding="utf-8")
        return self.checker.check_manual_structure(self.root)

    def _one(self, english, mirror, *, name=_CHAPTER):
        return self._check({name: (english, mirror)})

    def _span_messages(self, problems):
        return [problem.message for problem in problems if problem.kind == "spans"]

    def test_clean_mirror_reports_nothing(self):
        english = (
            "# Guide\n"
            "\n"
            "The `RenderCache` stores a `Representation` per `variance_key`.\n"
            "\n"
            "```rust\n"
            "let cache = RenderCache::new(`ignored inside a fence`);\n"
            "```\n"
        )
        mirror = (
            "# Guide\n"
            "\n"
            "Der `RenderCache` speichert eine `Representation` je\n"
            "`variance_key`.\n"
            "\n"
            "```rust\n"
            "let cache = RenderCache::new(`auch hier ignoriert`);\n"
            "```\n"
        )

        self.assertEqual(self._one(english, mirror), [])

    def test_lost_span_is_reported(self):
        english = (
            "# Guide\n"
            "\n"
            "The `RenderCache` stores a `Representation` per `variance_key`.\n"
        )
        mirror = "# Guide\n\nDer `RenderCache` speichert eine `Representation`.\n"

        problems = self._one(english, mirror)
        messages = self._span_messages(problems)

        self.assertEqual(len(problems), len(self.checker.LOCALES))
        self.assertEqual(len(messages), len(self.checker.LOCALES))
        for message in messages:
            self.assertIn("`variance_key`", message)
            self.assertIn("drops", message)

    def test_translated_span_is_reported_in_both_directions(self):
        english = "# Guide\n\nThe `RenderCache` holds one entry.\n"
        mirror = "# Guide\n\nDer `RenderZwischenspeicher` haelt einen Eintrag.\n"

        messages = self._span_messages(self._one(english, mirror))

        self.assertEqual(len(messages), 2 * len(self.checker.LOCALES))
        dropped = [message for message in messages if "drops" in message]
        added = [message for message in messages if "adds" in message]
        self.assertEqual(len(dropped), len(self.checker.LOCALES))
        self.assertEqual(len(added), len(self.checker.LOCALES))
        for message in dropped:
            self.assertIn("`RenderCache`", message)
        for message in added:
            self.assertIn("`RenderZwischenspeicher`", message)

    def test_backticks_added_around_prose_are_reported(self):
        english = "# Guide\n\nThe `RenderCache` writes the entry migrations.\n"
        mirror = "# Guide\n\nDer `RenderCache` schreibt die `entry` Migrationen.\n"

        messages = self._span_messages(self._one(english, mirror))

        self.assertEqual(len(messages), len(self.checker.LOCALES))
        for message in messages:
            self.assertIn("`entry`", message)
            self.assertIn("adds", message)

    def test_mirror_may_repeat_an_english_span(self):
        english = "# Guide\n\nThe `RenderCache` stores and evicts entries.\n"
        mirror = (
            "# Guide\n"
            "\n"
            "Der `RenderCache` speichert Eintraege. Der `RenderCache`\n"
            "entfernt sie wieder.\n"
        )

        self.assertEqual(self._one(english, mirror), [])

    def test_a_span_defect_is_reported_for_every_chapter(self):
        english = "# Guide\n\nThe `RenderCache` stores a `Representation`.\n"
        mirror = "# Guide\n\nDer `RenderCache` speichert etwas.\n"

        problems = self._check(
            {_CHAPTER: (english, mirror), _OTHER_CHAPTER: (english, mirror)}
        )
        spans = [problem for problem in problems if problem.kind == "spans"]

        self.assertEqual(len(spans), 2 * len(self.checker.LOCALES))
        self.assertEqual(
            {problem.file for problem in spans}, {_CHAPTER, _OTHER_CHAPTER}
        )
        for problem in spans:
            self.assertIn("`Representation`", problem.message)

    def test_a_chapter_is_still_checked_for_every_other_shape(self):
        english = "# Guide\n\n- one\n- two\n"
        mirror = "# Guide\n\n- eins\n"

        problems = self._check({_OTHER_CHAPTER: (english, mirror)})

        self.assertEqual(len(problems), len(self.checker.LOCALES))
        self.assertEqual({problem.kind for problem in problems}, {"lists"})

    def test_a_mis_nested_span_in_one_paragraph_does_not_taint_later_ones(self):
        # The stray backtick never finds a match within its own paragraph, so
        # a naive whole-document scan would keep hunting past the blank line
        # and pair it with the next opening backtick it finds, inventing a
        # bogus span out of everything in between and losing the real spans
        # entirely. Bounding extraction to one paragraph must confine the
        # damage there: the later paragraph's real spans are still found, and
        # a genuine drop in that paragraph is still reported, on its own.
        english = (
            "# Guide\n"
            "\n"
            "A stray backtick ` never closes in this paragraph.\n"
            "\n"
            "The `RenderCache` stores a `Representation`.\n"
        )
        mirror = (
            "# Guide\n"
            "\n"
            "A stray backtick ` never closes in this paragraph.\n"
            "\n"
            "The `RenderCache` stores something.\n"
        )

        problems = self._one(english, mirror)
        messages = self._span_messages(problems)

        self.assertEqual(len(problems), len(self.checker.LOCALES))
        self.assertEqual(len(messages), len(self.checker.LOCALES))
        for message in messages:
            self.assertIn("drops", message)
            self.assertIn("`Representation`", message)
            self.assertNotIn("stores a", message)
            self.assertNotIn("never closes", message)

    def test_a_span_wrapped_across_two_lines_of_one_paragraph_is_one_span(self):
        # The delimiters land on different physical lines of the same
        # paragraph, with no blank line, heading, table row, or list item
        # boundary between them. They must still be read as one span with
        # the line break collapsed to a single space, matching a mirror that
        # happens to wrap the same content onto a single line.
        english = (
            "# Guide\n"
            "\n"
            "The `long identifier that\n"
            "wraps` stays one span.\n"
        )
        mirror = "# Guide\n\nDer `long identifier that wraps` bleibt eine Spanne.\n"

        self.assertEqual(self._one(english, mirror), [])

    def test_a_span_wrapped_across_two_quoted_lines_drops_the_quote_marker(self):
        # A block quote marker is structure, not content: CommonMark strips it
        # before parsing the quoted block, so a span whose delimiters sit on
        # two quoted lines must not swallow the marker of the second one. A
        # mirror that wraps the same content onto one quoted line is not drift.
        english = (
            "# Guide\n"
            "\n"
            "> The `long identifier that\n"
            "> wraps` stays one span.\n"
        )
        mirror = (
            "# Guide\n"
            "\n"
            "> Der `long identifier that wraps` bleibt eine Spanne.\n"
        )

        self.assertEqual(self._one(english, mirror), [])

    def test_the_real_manual_tree_reports_no_problems(self):
        # No fixture, no ratchet: run the checker over this worktree's actual
        # `manual/` tree, so a regression anywhere in the corpus fails this
        # suite and not only the gate.
        problems = self.checker.check_manual_structure(_LOCAL_ROOT)

        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
