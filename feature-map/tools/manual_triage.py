"""Join the manual check's findings with the hand-verified verdicts in manual-triage-verdicts.json.

Usage: manual_triage.py <manual-check.jsonl> <verdicts.json> <out.jsonl> [worklist.json]

Every `missing`, `wrong_path`, `hidden` and `time_claim` finding in the check
output must have a verdict. Locations come from the check output, never from
the verdict file. `extra` holds problems found by hand that the check cannot
see (a real API used wrongly, a wrong claim about behavior); those carry their
own locations. The rules are docs/spec/manual-check.md's.

The run fails, writing nothing to <out.jsonl>, when any of these hold:
  - a finding has no verdict (MAN-005, MAN-107)
  - a verdict no longer matches any finding: it is stale (MAN-006)
  - an `error` or `code_bug` verdict has no `gap` of `none` or `issue` (MAN-101)
  - a `gap: issue` verdict has no `issue` number (MAN-103)
  - a `hidden` verdict has no `audience` (MAN-104)
Every problem is reported, not just the first; with a fourth argument the full
list is written there as JSON, as a worklist.

Verdict fields: `class` and `evidence` always; `fix` optional; `gap` and `issue` as above;
`audience` on `hidden`: framework-internal, reader-needed or public-equivalent.

Verdict classes:
  error       the manual names something the source does not have and nobody intended (fix the manual)
  wrong_path  the item exists at a different path than the manual writes
  internal    names a private or pub(crate) item: true, but not API a reader can use
  hidden      names a #[doc(hidden)] item: public, but not API by intent
  test_suite  describes the framework's own test or bench setup, not an app's
  code_bug    the manual states intended behavior the code lacks (keep the manual, issue filed)
  restate     a true, intended constraint worded as a moment in time: restate it as how Suprnova works
  verified    a time claim checked against the source or the developer, true and kept as written
  noise       the checker's mistake: the manual is correct here
  unverified  a claim about an external tool the source cannot settle
"""
import json
import sys
from collections import Counter, defaultdict

CLASSES = {"error", "wrong_path", "internal", "hidden", "test_suite", "code_bug", "restate", "verified",
           "noise", "unverified"}
AUDIENCES = {"framework-internal", "reader-needed", "public-equivalent"}
REVIEWED = ("missing", "wrong_path", "hidden", "time_claim")

check, verdicts_path, out = sys.argv[1:4]
worklist = sys.argv[4] if len(sys.argv) > 4 else None
data = json.load(open(verdicts_path))
verdicts, extra = data["verdicts"], data.get("extra", [])

locs = defaultdict(list)
outcome = {}
for line in open(check):
    f = json.loads(line)
    if f["outcome"] in REVIEWED:
        locs[f["ref"]].append(f"{f['chapter']}.md:{f['line']}")
        outcome[f["ref"]] = f["outcome"]


def field_problems(ref, v):
    out = []
    if v.get("class") not in CLASSES:
        out.append(("unknown_class", ref))
    if v.get("class") in ("error", "code_bug") and v.get("gap") not in ("none", "issue"):
        out.append(("missing_gap", ref))
    if v.get("gap") == "issue" and not isinstance(v.get("issue"), int):
        out.append(("missing_issue", ref))
    if v.get("class") == "hidden" and v.get("audience") not in AUDIENCES:
        out.append(("missing_audience", ref))
    return out


problems = [("unreviewed_" + outcome[r], r) for r in sorted(set(locs) - set(verdicts))]
problems += [("stale", r) for r in sorted(set(verdicts) - set(locs))]
for ref, v in sorted(verdicts.items()):
    if ref in locs:
        problems += field_problems(ref, v)
for x in extra:
    problems += field_problems(x["ref"], x)

if problems:
    kinds = defaultdict(list)
    for kind, ref in problems:
        kinds[kind].append({"ref": ref, "locations": sorted(set(locs.get(ref, [])))})
    if worklist:
        with open(worklist, "w") as fh:
            json.dump(kinds, fh, indent=1, sort_keys=True, ensure_ascii=False)
    print(json.dumps({"failed": {k: len(v) for k, v in sorted(kinds.items())},
                      "worklist": worklist}), file=sys.stderr)
    for kind, items in sorted(kinds.items()):
        for item in items[:5]:
            print(f"  {kind}: {item['ref'][:100]}", file=sys.stderr)
        if len(items) > 5:
            print(f"  {kind}: ... and {len(items) - 5} more", file=sys.stderr)
    sys.exit(1)

with open(out, "w") as fh:
    for ref in sorted(locs, key=lambda r: (verdicts[r]["class"], locs[r][0], r)):
        v = verdicts[ref]
        fh.write(json.dumps({"ref": ref, "check_outcome": outcome[ref], "class": v["class"],
                             "locations": sorted(set(locs[ref])), "evidence": v["evidence"],
                             "fix": v.get("fix"), "gap": v.get("gap"), "issue": v.get("issue"),
                             "audience": v.get("audience")}, sort_keys=True) + "\n")
    for x in extra:
        fh.write(json.dumps({**x, "check_outcome": None}, sort_keys=True) + "\n")
print(json.dumps({"reviewed": len(locs), "by_class": dict(Counter(verdicts[r]["class"] for r in locs)),
                  "extra": len(extra)}))
