#!/usr/bin/env python3
"""Tests for registry_allowlist.py's guarded boundary (REG-030).

A type of an engine crate (the Live engine, its macros, Magnetar), and any
item under suprnova::live, suprnova::render_cache or suprnova::view, reaches
a component only when a rule admits it: the framework re-exports engine
types so an application can name every type a public signature leads to and
adds Live items of its own, and neither lands on the component allowlist by
default.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import registry_allowlist as tool  # noqa: E402


def engine_record(path, kind="struct", chapter="live"):
    return {
        "id": "suprnova_live::internal::" + path.rsplit("::", 1)[-1],
        "crate": "suprnova_live",
        "kind": kind,
        "chapter": chapter,
        "module": "suprnova_live::internal",
        "aliases": [path],
        "details": {},
    }


class EngineBoundary(unittest.TestCase):
    def test_a_new_engine_re_export_is_refused_until_a_rule_admits_it(self):
        record = engine_record("suprnova::live::BrandNewEngineType")
        self.assertEqual(tool.admission(record, "suprnova::live::BrandNewEngineType", "suprnova::live"), tool.REFUSE)

    def test_an_admitted_engine_type_and_its_members_keep_their_capability(self):
        record = engine_record("suprnova::live::ErrorBag")
        self.assertIsNone(tool.admission(record, "suprnova::live::ErrorBag", "suprnova::live"))
        member = engine_record("suprnova::live::ErrorBag::from_issues", kind="fn")
        self.assertIsNone(tool.admission(member, "suprnova::live::ErrorBag::from_issues", "suprnova::live"))
        key = engine_record("suprnova::render_cache::RenderKey", chapter="render-cache")
        self.assertEqual(tool.admission(key, "suprnova::render_cache::RenderKey", "suprnova::render_cache"), "cache")

    def test_an_item_rule_refuses_an_engine_type_the_table_would_admit(self):
        record = engine_record("suprnova::live::HostCapabilities")
        self.assertEqual(tool.admission(record, "suprnova::live::HostCapabilities", "suprnova::live"), tool.REFUSE)

    def test_a_new_framework_live_item_is_refused_until_a_rule_admits_it(self):
        record = {"id": "suprnova::live::BrandNewInstaller", "crate": "suprnova", "kind": "fn", "chapter": "live",
                  "module": "suprnova::live", "details": {}}
        self.assertEqual(tool.admission(record, "suprnova::live::BrandNewInstaller", "suprnova::live"), tool.REFUSE)
        cache = {"id": "suprnova::render_cache::BrandNewStore", "crate": "suprnova", "kind": "struct",
                 "chapter": "render-cache", "module": "suprnova::render_cache", "details": {}}
        self.assertEqual(tool.admission(cache, "suprnova::render_cache::BrandNewStore", "suprnova::render_cache"), tool.REFUSE)

    def test_an_admitted_framework_live_type_keeps_its_decision(self):
        record = {"id": "suprnova::live::LiveRegistry", "crate": "suprnova", "kind": "struct", "chapter": "live",
                  "module": "suprnova::live", "details": {}}
        self.assertIsNone(tool.admission(record, "suprnova::live::LiveRegistry", "suprnova::live"))
        member = {"id": "suprnova::live::LiveRegistry::builder", "crate": "suprnova", "kind": "fn", "chapter": "live",
                  "module": "suprnova::live", "details": {}}
        self.assertIsNone(tool.admission(member, "suprnova::live::LiveRegistry::builder", "suprnova::live"))

    def test_a_framework_type_outside_the_guarded_paths_is_decided_by_its_chapter_and_module(self):
        record = {"id": "suprnova::Cache", "crate": "suprnova", "kind": "struct", "chapter": "cache",
                  "module": "suprnova::cache", "details": {}}
        self.assertEqual(tool.admission(record, "suprnova::Cache", "suprnova::cache"), tool.decide("suprnova::Cache", "suprnova::cache", "cache"))
        self.assertNotEqual(tool.admission(record, "suprnova::Cache", "suprnova::cache"), tool.REFUSE)

    def test_every_admit_rule_names_an_item_path_and_no_guarded_root(self):
        for path in tool.ADMIT:
            self.assertTrue(path.startswith("suprnova::"), path)
            self.assertNotIn(path, tool.GATED_PREFIXES, path)


if __name__ == "__main__":
    unittest.main()
