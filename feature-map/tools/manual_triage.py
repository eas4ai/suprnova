"""Join the manual check's findings with the hand-verified verdicts in manual-triage-verdicts.json.

Usage: manual_triage.py <manual-check.jsonl> <verdicts.json> <out.jsonl>

Every `missing`, `wrong_path` and `hidden` reference in the check output must
have a verdict; the run fails otherwise, so a new finding can't slip through
unreviewed. Verdicts that no longer match any finding are reported as stale.
Locations come from the check output, never from the verdict file. `extra`
holds problems found by hand that the check cannot see (a real API used
wrongly, a wrong default value); those carry their own locations.

Verdict classes:
  error       the manual names something the source does not have and nobody intended (fix the manual)
  wrong_path  the item exists at a different path than the manual writes
  internal    names a private or pub(crate) item: true, but not API a reader can use
  hidden      names a #[doc(hidden)] item: public, but not API by intent
  test_suite  describes the framework's own test or bench setup, not an app's
  code_bug    the manual states intended behavior the code lacks (keep the manual, issue filed)
  noise       the checker's mistake: the manual is correct here
  unverified  a claim about an external tool the source cannot settle
"""
import json
import sys
from collections import Counter, defaultdict

CLASSES = {"error", "wrong_path", "internal", "hidden", "test_suite", "code_bug", "noise", "unverified"}
REVIEWED = ("missing", "wrong_path", "hidden")

check, verdicts_path, out = sys.argv[1:4]
verdicts = json.load(open(verdicts_path))["verdicts"]
bad = [r for r, v in verdicts.items() if v["class"] not in CLASSES]
if bad:
    sys.exit(f"unknown verdict class for: {bad}")

locs = defaultdict(list)
outcome = {}
for line in open(check):
    f = json.loads(line)
    if f["outcome"] in REVIEWED:
        locs[f["ref"]].append(f"{f['chapter']}.md:{f['line']}")
        outcome[f["ref"]] = f["outcome"]

unreviewed = sorted(set(locs) - set(verdicts))
if unreviewed:
    sys.exit("findings without a verdict:\n  " + "\n  ".join(unreviewed))
stale = sorted(set(verdicts) - set(locs))

with open(out, "w") as fh:
    for ref in sorted(locs, key=lambda r: (verdicts[r]["class"], locs[r][0], r)):
        v = verdicts[ref]
        fh.write(json.dumps({"ref": ref, "check_outcome": outcome[ref], "class": v["class"],
                             "locations": sorted(set(locs[ref])), "evidence": v["evidence"],
                             "fix": v.get("fix")}, sort_keys=True) + "\n")
    for x in json.load(open(verdicts_path)).get("extra", []):
        if x["class"] not in CLASSES:
            sys.exit(f"unknown verdict class for extra: {x['ref']}")
        fh.write(json.dumps({**x, "check_outcome": None}, sort_keys=True) + "\n")
print(json.dumps({"reviewed": len(locs), "by_class": dict(Counter(verdicts[r]["class"] for r in locs)),
                  "stale_verdicts": stale}))
