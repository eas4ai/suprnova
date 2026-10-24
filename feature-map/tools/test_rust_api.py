"""Tests for rust_api.py's return types (`details.returns`).

Run: python3 feature-map/tools/test_rust_api.py

The registry scan types the value a Suprnova function returns from these,
so a method chain on it can be classified (REG-030). The rustdoc JSON
below is the shape rustdoc's format 61 gives a crate with one builder.
"""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import rust_api  # noqa: E402


def path(name, id_, args=None):
    return {"resolved_path": {"path": name, "id": id_, "args": args}}


def angle(*types):
    return {"angle_bracketed": {"args": [{"type": t} for t in types], "constraints": []}}


def function(name, output, id_):
    return {
        "id": id_, "crate_id": 0, "name": name, "attrs": [], "visibility": "public",
        "inner": {"function": {"sig": {"inputs": [], "output": output, "is_c_variadic": False},
                               "generics": {"params": [], "where_predicates": []},
                               "header": {}, "has_body": True}},
    }


def rustdoc():
    """A crate `probe` with `pub struct Builder`, `pub struct Policy` and
    `pub fn make() -> Builder`; Builder's inherent methods return `Self`, a
    local type, `Result<Policy, String>`, `Option<&Policy>`, `impl Iterator`
    and nothing."""
    index = {
        "0": {"id": 0, "crate_id": 0, "name": "probe", "attrs": [], "visibility": "public",
              "inner": {"module": {"items": [1, 2, 3], "is_crate": True, "is_stripped": False}}},
        "1": {"id": 1, "crate_id": 0, "name": "Builder", "attrs": [], "visibility": "public",
              "inner": {"struct": {"kind": {"unit": None}, "generics": {"params": [], "where_predicates": []},
                                   "impls": [10]}}},
        "2": {"id": 2, "crate_id": 0, "name": "Policy", "attrs": [], "visibility": "public",
              "inner": {"struct": {"kind": {"unit": None}, "generics": {"params": [], "where_predicates": []},
                                   "impls": []}}},
        "3": function("make", path("Builder", 1), 3),
        "10": {"id": 10, "crate_id": 0, "name": None, "attrs": [], "visibility": "default",
               "inner": {"impl": {"is_synthetic": False, "blanket_impl": None, "trait": None,
                                  "items": [11, 12, 13, 14, 15, 16]}}},
        "11": function("limit", {"generic": "Self"}, 11),
        "12": function("build", path("Policy", 2), 12),
        "13": function("try_build", path("Result", 90, angle(path("Policy", 2), path("String", 91))), 13),
        "14": function("first", path("Option", 92, angle({"borrowed_ref": {"lifetime": None, "is_mutable": False,
                                                                           "type": path("Policy", 2)}})), 14),
        "15": function("iter", {"impl_trait": []}, 15),
        "16": function("reset", None, 16),
    }
    paths = {
        "1": {"crate_id": 0, "path": ["probe", "Builder"], "kind": "struct"},
        "2": {"crate_id": 0, "path": ["probe", "Policy"], "kind": "struct"},
        "90": {"crate_id": 1, "path": ["core", "result", "Result"], "kind": "enum"},
        "91": {"crate_id": 2, "path": ["alloc", "string", "String"], "kind": "struct"},
        "92": {"crate_id": 1, "path": ["core", "option", "Option"], "kind": "enum"},
    }
    return {"root": 0, "format_version": 61, "index": index, "paths": paths,
            "external_crates": {"1": {"name": "core"}, "2": {"name": "alloc"}}}


class ReturnTypes(unittest.TestCase):
    def setUp(self):
        directory = tempfile.mkdtemp()
        self.file = Path(directory) / "probe.json"
        self.file.write_text(json.dumps(rustdoc()))

    def test_every_function_record_names_its_return_type(self):
        crate = rust_api.Crate(self.file)
        records, _ = rust_api.records(crate, None)
        returns = {r["id"]: r["details"].get("returns") for r in records if r["kind"] == "fn"}
        self.assertEqual(returns, {
            "probe::make": "probe::Builder",
            "probe::Builder::limit": "probe::Builder",
            "probe::Builder::build": "probe::Policy",
            "probe::Builder::try_build": "core::result::Result<probe::Policy,alloc::string::String>",
            "probe::Builder::first": "core::option::Option<&probe::Policy>",
            "probe::Builder::iter": "_",
            "probe::Builder::reset": "()",
        })

    def test_types_that_name_no_single_type_are_unknown(self):
        crate = rust_api.Crate(self.file)
        self.assertEqual(rust_api.type_text(crate, {"generic": "T"}, "probe::Builder"), "_")
        self.assertEqual(rust_api.type_text(crate, {"generic": "Self"}, None), "_")
        self.assertEqual(rust_api.type_text(crate, {"dyn_trait": {}}, None), "_")


if __name__ == "__main__":
    unittest.main()
