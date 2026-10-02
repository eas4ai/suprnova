"""Assertions over recorded observations, not a second package validator."""

from pathlib import PurePosixPath
import re

from live_contract_checks.cases import MATRICES, POSITIVE_CASES


class Incomplete(Exception):
    """Required evidence is unavailable."""


class Violation(Exception):
    """An observed result violates the contract."""


def need(condition, message):
    if not condition:
        raise Incomplete(message)


def check(condition, message):
    if not condition:
        raise Violation(message)


def fields(value, names, label):
    need(type(value) is dict, f"{label}: expected an observation object")
    need(set(names.split()) <= value.keys(), f"{label}: required observations are missing")
    return value


def strings(value, label, *, nonempty=False):
    need(type(value) is list and all(type(item) is str and item for item in value),
         f"{label}: expected nonempty strings")
    need(not nonempty or bool(value), f"{label}: no observations")
    need(len(set(value)) == len(value), f"{label}: duplicate observations")
    return value


def integer(value, label):
    need(type(value) is int and value >= 0, f"{label}: expected a nonnegative integer")
    return value


def digest(value, label):
    need(type(value) is str and re.fullmatch(r"sha256:[0-9a-f]{64}", value) is not None,
         f"{label}: exact byte identity is missing")
    return value


def relative_path(value, label):
    need(type(value) is str and bool(value), f"{label}: path missing")
    path = PurePosixPath(value)
    need(bool(path.parts) and not path.is_absolute() and ".." not in path.parts and
         "\\" not in value and not any(ord(c) < 32 for c in value),
         f"{label}: expected a relative source path")


def matrix(requirement, observation):
    o = fields(observation, "corpus cases", "conformance matrix")
    corpus = strings(o["corpus"], "corpus", nonempty=True)
    need(set(MATRICES[requirement]) <= set(corpus), "required conformance coverage is missing")
    need(type(o["cases"]) is list and bool(o["cases"]), "case observations are missing")
    cases = {}
    for entry in o["cases"]:
        case = fields(entry, "id expected public tooling", "case")
        need(all(type(case[key]) is str and case[key] for key in ("id", "expected", "public", "tooling")),
             "case identity or outcome is malformed")
        need(case["id"] not in cases, "a case was observed more than once")
        if case["id"] in MATRICES[requirement]:
            need((case["expected"] == "ok") == (case["id"] in POSITIVE_CASES),
                 "golden expectations lost their positive or negative control")
        cases[case["id"]] = case
    need(set(cases) == set(corpus), "the complete shared corpus was not exercised")
    for name, case in cases.items():
        check(case["public"] == case["tooling"] == case["expected"],
              f"fixture {name}: public validation, tooling, and golden outcome disagree")
    return cases


def public_boundary(o):
    fields(o, "external_build_exit consumer_dependencies contract_dependencies internal_imports ownership_review", "public boundary")
    check(integer(o["external_build_exit"], "external build") == 0, "external public consumer did not compile")
    consumer = set(strings(o["consumer_dependencies"], "consumer dependencies", nonempty=True))
    check({"suprnova", "suprnova-live-library-contract"} <= consumer,
          "consumer does not use both public dependencies")
    check("suprnova-live" not in consumer, "consumer directly imports the internal engine")
    dependencies = set(strings(o["contract_dependencies"], "contract dependency closure"))
    check(not ({"suprnova", "suprnova-live"} & dependencies),
          "the standalone contract crate depends on framework implementation")
    check(not strings(o["internal_imports"], "internal imports"), "consumer imports engine internals")
    review = fields(o["ownership_review"], "source_identity reviewed_files duplicated_rules evidence", "ownership review")
    digest(review["source_identity"], "review source")
    for path in strings(review["reviewed_files"], "reviewed files", nonempty=True):
        relative_path(path, "reviewed source")
    need(type(review["evidence"]) is str and bool(review["evidence"].strip()), "ownership review needs source citations")
    check(not strings(review["duplicated_rules"], "duplicated rules"), "SDK duplicates a framework-owned rule")


def schema(o):
    matrix("LCT-002", o)


def filesystem(o):
    matrix("LCT-003", o)
    fields(o, "digest_vectors", "digest vectors")
    vectors = o["digest_vectors"]
    need(type(vectors) is list and len(vectors) >= 2, "base and changed-byte digest vectors are missing")
    names, identities = [], set()
    for value in vectors:
        vector = fields(value, "id expected actual", "digest vector")
        need(type(vector["id"]) is str and bool(vector["id"]), "digest vector identity is missing")
        names.append(vector["id"])
        expected = digest(vector["expected"], "golden digest")
        identities.add(expected)
        check(digest(vector["actual"], "observed digest") == expected, "canonical digest differs from the golden vector")
    need(len(set(names)) == len(names) and len(identities) >= 2, "digest vectors omit byte-change coverage")


def bounded_reads(values):
    need(type(values) is list, "bounded read instrumentation is missing")
    expected = {"manifest-bytes": 1048576, "payload-bytes": 1048576, "package-bytes": 67108864}
    names = []
    for value in values:
        sample = fields(value, "id limit read declared_size largest_allocation", "bounded read")
        need(type(sample["id"]) is str and sample["id"] in expected, "unexpected bounded-read probe")
        name = sample["id"]
        names.append(name)
        limit = integer(sample["limit"], "byte limit")
        need(limit == expected[name], "bounded-read probe used another contract limit")
        declared = integer(sample["declared_size"], "declared size")
        need(declared > limit + 65536, "oversized read probe did not exceed its bounded read allowance")
        check(integer(sample["read"], "bytes read") <= limit + 65536, "validator consumed an unbounded oversized input")
        check(integer(sample["largest_allocation"], "largest allocation") < declared,
              "validator allocated the oversized input's declared length before rejecting it")
    need(len(names) == len(set(names)) and set(names) == set(expected), "bounded-read probes are incomplete or duplicated")


def limits(o):
    matrix("LCT-004", o)
    fields(o, "before after side_effects bounded_reads", "inspection effects")
    check(digest(o["before"], "before inspection") == digest(o["after"], "after inspection"),
          "structural inspection mutated the package")
    effects = fields(o["side_effects"], "processes network file_writes", "side effects")
    for name in ("processes", "network", "file_writes"):
        check(integer(effects[name], name) == 0, f"inspection produced {name}")
    bounded_reads(o["bounded_reads"])


def stages(o):
    fields(o, "checked skipped", "report stages")
    check(set(strings(o["checked"], "checked stages")) == {"structure"}, "structural check claims a stage it did not execute")
    check({"runtime", "binary", "installation", "rendering"} <= set(strings(o["skipped"], "skipped stages")),
          "structural report omits an unexecuted stage")


def diagnostics(o):
    cases = matrix("LCT-005", o)
    fields(o, "reports", "diagnostic reports")
    need(type(o["reports"]) is list, "structured reports are missing")
    names = []
    for value in o["reports"]:
        report = fields(value, "id validator_version digest location checked skipped terminal", "diagnostic report")
        need(type(report["id"]) is str and report["id"] in cases, "report names an unknown case")
        names.append(report["id"])
        need(type(report["validator_version"]) is str and bool(report["validator_version"].strip()), "validator version missing")
        relative_path(report["location"], "diagnostic source")
        if report["digest"] is not None or cases[report["id"]]["expected"] == "ok":
            digest(report["digest"], "reported package digest")
        stages(report)
        terminal = report["terminal"]
        need(type(terminal) is str, "terminal observation is missing")
        check(not any((ord(c) < 32 and c not in "\n\t") or 127 <= ord(c) <= 159 for c in terminal),
              "terminal output contains unescaped control characters")
    need(len(names) == len(set(names)) and set(names) == set(cases), "diagnostic reports do not cover the corpus once each")


def legacy(o):
    matrix("LCT-006", o)
    fields(o, "legacy_tests legacy_kind catalog_kind checked skipped", "legacy regression")
    tests = fields(o["legacy_tests"], "discovered passed failed", "legacy test results")
    discovered = strings(tests["discovered"], "discovered legacy tests", nonempty=True)
    check(set(strings(tests["passed"], "passed legacy tests")) == set(discovered), "a discovered legacy regression test did not pass")
    check(not strings(tests["failed"], "failed legacy tests"), "a legacy regression test failed")
    check(o["legacy_kind"] == "legacy-manifest", "legacy input was silently reclassified")
    check(o["catalog_kind"] == "schema1-library", "catalog acceptance did not use the explicit schema1 path")
    stages(o)


OBSERVERS = {
    "LCT-001": public_boundary,
    "LCT-002": schema,
    "LCT-003": filesystem,
    "LCT-004": limits,
    "LCT-005": diagnostics,
    "LCT-006": legacy,
}


def evaluate(requirement, observation):
    need(requirement in OBSERVERS, "unknown Live contract requirement")
    need(type(observation) is dict, "expected a requirement observation")
    OBSERVERS[requirement](observation)
