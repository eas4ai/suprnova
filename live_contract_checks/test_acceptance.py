"""Removing an observer assertion must make its negative test fail."""

import unittest

from live_contract_checks.acceptance import Incomplete, Violation, evaluate
from live_contract_checks.control_examples import CHANGED, example, violating_example


REQUIREMENTS = tuple(f"LCT-{number:03}" for number in range(1, 7))


class AcceptanceTests(unittest.TestCase):
    def test_complete_controls_satisfy_the_assertions(self):
        for requirement in REQUIREMENTS:
            with self.subTest(requirement=requirement):
                evaluate(requirement, example(requirement))

    def test_each_control_detects_its_stated_violation(self):
        for requirement in REQUIREMENTS:
            with self.subTest(requirement=requirement), self.assertRaises(Violation):
                evaluate(requirement, violating_example(requirement))

    def test_missing_observations_never_pass(self):
        for requirement in REQUIREMENTS:
            for value in [None, [], {}, {"cases": []}]:
                with self.subTest(requirement=requirement, value=value), self.assertRaises(Incomplete):
                    evaluate(requirement, value)

    def test_public_boundary_rejects_internal_imports_and_duplicate_rules(self):
        for field, value in [("external_build_exit", 1), ("internal_imports", ["suprnova_live::Engine"]),
                             ("consumer_dependencies", ["suprnova"]), ("contract_dependencies", ["suprnova"])]:
            observation = example("LCT-001")
            observation[field] = value
            with self.subTest(field=field), self.assertRaises(Violation):
                evaluate("LCT-001", observation)
        observation = example("LCT-001")
        observation["ownership_review"]["duplicated_rules"] = ["SDK package parser"]
        with self.assertRaises(Violation):
            evaluate("LCT-001", observation)

    def test_malformed_build_and_review_evidence_are_incomplete(self):
        observation = example("LCT-001")
        observation["external_build_exit"] = False
        with self.assertRaises(Incomplete):
            evaluate("LCT-001", observation)
        for field, value in [("reviewed_files", []), ("source_identity", "same"), ("evidence", "")]:
            observation = example("LCT-001")
            observation["ownership_review"][field] = value
            with self.subTest(field=field), self.assertRaises(Incomplete):
                evaluate("LCT-001", observation)

    def test_matrices_require_complete_unique_corpus_coverage(self):
        for requirement in REQUIREMENTS[1:]:
            for mutation in [lambda o: o["cases"].pop(), lambda o: o["cases"].append(o["cases"][0]),
                             lambda o: o.update(corpus=[]), lambda o: o["corpus"].append(o["corpus"][0])]:
                observation = example(requirement)
                mutation(observation)
                with self.subTest(requirement=requirement), self.assertRaises(Incomplete):
                    evaluate(requirement, observation)

    def test_reject_everything_and_accept_everything_cannot_pass(self):
        for requirement in ["LCT-002", "LCT-003", "LCT-004", "LCT-005"]:
            for outcome in ["ok", "refused"]:
                observation = example(requirement)
                for case in observation["cases"]:
                    case["public"] = case["tooling"] = outcome
                with self.subTest(requirement=requirement, outcome=outcome), self.assertRaises(Violation):
                    evaluate(requirement, observation)

    def test_corrupt_golden_expectations_are_not_complete_evidence(self):
        for expected in ["ok", "refused"]:
            observation = example("LCT-002")
            for case in observation["cases"]:
                case["expected"] = case["public"] = case["tooling"] = expected
            with self.subTest(expected=expected), self.assertRaises(Incomplete):
                evaluate("LCT-002", observation)

    def test_digest_vectors_require_distinct_valid_identities(self):
        for vectors in [[], [{"id": "base", "expected": "x", "actual": "x"}]]:
            observation = example("LCT-003")
            observation["digest_vectors"] = vectors
            with self.subTest(vectors=vectors), self.assertRaises(Incomplete):
                evaluate("LCT-003", observation)

    def test_limits_detect_mutation_side_effects_and_unbounded_reads(self):
        for field, value in [("after", CHANGED), ("side_effects", {"processes": 0, "network": 1, "file_writes": 0})]:
            observation = example("LCT-004")
            observation[field] = value
            with self.subTest(field=field), self.assertRaises(Violation):
                evaluate("LCT-004", observation)
        for field in ["read", "largest_allocation"]:
            observation = example("LCT-004")
            observation["bounded_reads"][0][field] = 1 << 40
            with self.subTest(field=field), self.assertRaises(Violation):
                evaluate("LCT-004", observation)

    def test_limit_instrumentation_cannot_be_missing_or_name_another_limit(self):
        for mutation in [lambda o: o.update(bounded_reads=[]),
                         lambda o: o["bounded_reads"][0].update(limit=1),
                         lambda o: o["bounded_reads"][0].update(read=-1)]:
            observation = example("LCT-004")
            mutation(observation)
            with self.assertRaises(Incomplete):
                evaluate("LCT-004", observation)

    def test_diagnostics_require_complete_reports_and_real_metadata(self):
        for mutation in [lambda o: o["reports"].pop(), lambda o: o["reports"][0].update(validator_version=""),
                         lambda o: o["reports"][0].update(location="/private/path"),
                         lambda o: o["reports"][0].update(digest=None)]:
            observation = example("LCT-005")
            mutation(observation)
            with self.assertRaises(Incomplete):
                evaluate("LCT-005", observation)

    def test_raw_terminal_controls_and_false_stage_claims_fail(self):
        for field, value in [("terminal", "\x1b[31munsafe"), ("checked", ["structure", "rendering"]),
                             ("skipped", [])]:
            observation = example("LCT-005")
            observation["reports"][0][field] = value
            with self.subTest(field=field), self.assertRaises(Violation):
                evaluate("LCT-005", observation)

    def test_legacy_regressions_and_reclassification_fail(self):
        for field, value in [("legacy_kind", "schema1-library"), ("catalog_kind", "legacy-manifest")]:
            observation = example("LCT-006")
            observation[field] = value
            with self.subTest(field=field), self.assertRaises(Violation):
                evaluate("LCT-006", observation)
        for mutation in [lambda t: t["passed"].pop(), lambda t: t["failed"].append("edited")]:
            observation = example("LCT-006")
            mutation(observation["legacy_tests"])
            with self.assertRaises(Violation):
                evaluate("LCT-006", observation)

    def test_empty_legacy_discovery_is_unverified(self):
        observation = example("LCT-006")
        observation["legacy_tests"] = {"discovered": [], "passed": [], "failed": []}
        with self.assertRaises(Incomplete):
            evaluate("LCT-006", observation)


if __name__ == "__main__":
    unittest.main()
