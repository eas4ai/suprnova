"""Synthetic observer inputs. These never qualify the framework."""

from copy import deepcopy

from live_contract_checks.cases import MATRICES, POSITIVE_CASES


IDENTITY = "sha256:" + "1" * 64
CHANGED = "sha256:" + "2" * 64


def example(requirement):
    if requirement == "LCT-001":
        return {
            "external_build_exit": 0,
            "consumer_dependencies": ["suprnova", "suprnova-live-library-contract"],
            "contract_dependencies": ["serde", "sha2"],
            "internal_imports": [],
            "ownership_review": {"source_identity": IDENTITY,
                                 "reviewed_files": ["src/main.rs"],
                                 "duplicated_rules": [],
                                 "evidence": "Synthetic control; no real source review."},
        }
    names = MATRICES[requirement]
    cases = []
    for name in names:
        expected = "ok" if name in POSITIVE_CASES else "control.rejected"
        cases.append({"id": name, "expected": expected, "public": expected, "tooling": expected})
    result = {"corpus": list(names), "cases": cases}
    if requirement == "LCT-003":
        result["digest_vectors"] = [{"id": "base", "expected": IDENTITY, "actual": IDENTITY},
                                    {"id": "changed", "expected": CHANGED, "actual": CHANGED}]
    elif requirement == "LCT-004":
        result.update({"before": IDENTITY, "after": IDENTITY,
                       "side_effects": {"processes": 0, "network": 0, "file_writes": 0},
                       "bounded_reads": [{"id": name, "limit": limit, "read": limit + 1,
                                          "declared_size": 1 << 40, "largest_allocation": limit}
                                         for name, limit in [("manifest-bytes", 1048576),
                                                             ("payload-bytes", 1048576),
                                                             ("package-bytes", 67108864)]]})
    elif requirement == "LCT-005":
        result["reports"] = [{"id": case["id"], "validator_version": "control-only",
                              "digest": IDENTITY if case["expected"] == "ok" else None,
                              "location": "library.json", "checked": ["structure"],
                              "skipped": ["runtime", "binary", "installation", "rendering"],
                              "terminal": "control \\u001b[31m"} for case in cases]
    elif requirement == "LCT-006":
        result.update({"legacy_tests": {"discovered": ["official", "third-party", "edited"],
                                        "passed": ["official", "third-party", "edited"], "failed": []},
                       "legacy_kind": "legacy-manifest", "catalog_kind": "schema1-library",
                       "checked": ["structure"],
                       "skipped": ["runtime", "binary", "installation", "rendering"]})
    return deepcopy(result)


def violating_example(requirement):
    result = example(requirement)
    if requirement == "LCT-001":
        result["contract_dependencies"].append("suprnova-live")
    elif requirement == "LCT-002":
        result["cases"][1]["public"] = "ok"
    elif requirement == "LCT-003":
        result["digest_vectors"][1]["actual"] = IDENTITY
    elif requirement == "LCT-004":
        result["side_effects"]["processes"] = 1
    elif requirement == "LCT-005":
        result["cases"][1]["tooling"] = "different.diagnostic"
    elif requirement == "LCT-006":
        result["checked"].append("runtime")
    return result
