#!/usr/bin/env python3
"""Tests for registry_allowlist.py's engine boundary (REG-030).

A type of an engine crate (the Live engine, its macros, Magnetar) reaches a
component only when a rule admits it: the framework re-exports engine types
so an application can name every type a public signature leads to, and a new
re-export must not land on the component allowlist by default.
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

    def test_a_framework_type_is_decided_by_its_chapter_and_module(self):
        record = {"id": "suprnova::live::LiveRegistry", "crate": "suprnova", "kind": "struct", "chapter": "live",
                  "module": "suprnova::live", "details": {}}
        self.assertIsNone(tool.admission(record, "suprnova::live::LiveRegistry", "suprnova::live"))

    def test_every_admit_rule_names_an_engine_path(self):
        for path in tool.ENGINE_ADMIT:
            self.assertTrue(path.startswith("suprnova::"), path)


if __name__ == "__main__":
    unittest.main()
