"""Configuration surface as JSON records: environment variables and Cargo features.

Env vars come from env_vars.py's JSON (source-confirmed reads, with every
read site). Cargo features come from each published crate's manifest. The
names env_vars.py saw and rejected go to an exclusions file, not away.
"""
import hashlib
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(sys.argv[1])
ENV = json.loads(Path(sys.argv[2]).read_text())
OUT = Path(sys.argv[3])
EXCL = Path(sys.argv[4])
CRATES = [
    ("suprnova", "framework"), ("suprnova-live", "crates/suprnova-live"),
    ("suprnova-magnetar", "crates/suprnova-magnetar"),
    ("suprnova-payments-stripe", "crates/suprnova-payments-stripe"),
    ("suprnova-payments-paddle", "crates/suprnova-payments-paddle"),
    ("suprnova-payments-nowpayments", "crates/suprnova-payments-nowpayments"),
    ("suprnova-web-push", "crates/suprnova-web-push"), ("suprnova-cli", "suprnova-cli"),
    ("suprnova-macros", "suprnova-macros"),
]


def digest(value):
    text = value if isinstance(value, str) else json.dumps(value, sort_keys=True)
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def source_line(file, line):
    return " ".join((ROOT / file).read_text().splitlines()[line - 1].split())


recs = []
for name, sites in ENV["env_vars"].items():
    # Hash the read sites' text, not their line numbers, so unrelated edits don't count as drift.
    texts = sorted(source_line(f, ln) for _, f, ln in sites)
    recs.append({"id": name, "kind": "env", "family": "config", "parent": None,
                 "crate": sites[0][0], "module": name.split("_")[0], "file": sites[0][1], "line": sites[0][2],
                 "details": {"read_sites": [f"{f}:{ln}" for _, f, ln in sites],
                             "crates": sorted({c for c, _, _ in sites})},
                 "sig_hash": digest(texts), "body_hash": None})

for crate, d in CRATES:
    manifest = ROOT / d / "Cargo.toml"
    feats = tomllib.loads(manifest.read_text()).get("features", {})
    default = set(feats.get("default", []))
    lines = manifest.read_text().splitlines()
    start = next((i for i, l in enumerate(lines) if l.strip() == "[features]"), 0)
    for name, enables in feats.items():
        if name == "default":
            continue
        at = next((i + 1 for i in range(start, len(lines))
                   if re.match(rf'^"?{re.escape(name)}"?\s*=', lines[i])), None)
        recs.append({"id": f"{crate}/{name}", "kind": "cargo-feature", "family": "config", "parent": None,
                     "crate": crate, "module": crate, "file": f"{d}/Cargo.toml", "line": at,
                     "details": {"default": name in default, "enables": enables},
                     "sig_hash": digest({"default": name in default, "enables": enables}), "body_hash": None})

OUT.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in recs))
EXCL.write_text(json.dumps({
    "build_time_env": {k: [f"{f}:{ln}" for _, f, ln in v] for k, v in ENV["build_time"].items()},
    "not_env_vars": {k: [f"{f}:{ln}" for _, f, ln in v] for k, v in ENV["rejected_candidates"].items()},
}, indent=1, sort_keys=True))
print(json.dumps({"records": len(recs), "env": len(ENV["env_vars"])}))
