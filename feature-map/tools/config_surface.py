"""Configuration surface: environment variables and Cargo features.

Env vars come from env_vars.py's JSON (source-confirmed reads, with every
read site). Cargo features come from each published crate's manifest.
"""
import json
import re
import sys
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(sys.argv[1])
REV = sys.argv[2]
ENV = json.loads(Path(sys.argv[3]).read_text())
OUT = Path(sys.argv[4])
CRATES = [
    ("suprnova", "framework"), ("suprnova-live", "crates/suprnova-live"),
    ("suprnova-magnetar", "crates/suprnova-magnetar"),
    ("suprnova-payments-stripe", "crates/suprnova-payments-stripe"),
    ("suprnova-payments-paddle", "crates/suprnova-payments-paddle"),
    ("suprnova-payments-nowpayments", "crates/suprnova-payments-nowpayments"),
    ("suprnova-web-push", "crates/suprnova-web-push"), ("suprnova-cli", "suprnova-cli"),
    ("suprnova-macros", "suprnova-macros"),
]

checked = set()
if OUT.exists():
    for line in OUT.read_text().splitlines():
        m = re.match(r'\s*- \[x\] (?:[a-z ]+ )?`([^`]+)`', line)
        if m:
            checked.add(m.group(1))


def box(key):
    return "x" if key in checked else " "


env = ENV["env_vars"]
L = [
    "# Suprnova feature map: configuration",
    "",
    f"Source: repository at {REV}. Environment variables are every name the crates read "
    "at runtime, found by following each read back to its literal or constant "
    "(`env`, `env_optional`, `env_required`, `std::env::var` and the wrappers and "
    "reader closures built on them). Cargo features come from each crate's manifest.",
    "",
    "A checked box means the documentation for that item has been remediated against the source.",
    "",
    "## Counts",
    "",
    f"- Environment variables: {len(env)}",
]
feature_rows = []
for crate, d in CRATES:
    man = tomllib.loads((ROOT / d / "Cargo.toml").read_text())
    feats = man.get("features", {})
    default = set(feats.get("default", []))
    lines = (ROOT / d / "Cargo.toml").read_text().splitlines()
    start = next((i for i, l in enumerate(lines) if l.strip() == "[features]"), None)
    for name, enables in feats.items():
        if name == "default":
            continue
        at = None
        if start is not None:
            for i in range(start, len(lines)):
                if re.match(rf'^{re.escape(name)}\s*=', lines[i]) or re.match(rf'^"{re.escape(name)}"\s*=', lines[i]):
                    at = i + 1
                    break
        feature_rows.append((crate, d, name, name in default, enables, at))
L.append(f"- Cargo features: {len(feature_rows)}")

L += ["", "## Environment variables", "",
      "Grouped by prefix. Each lists every source location that reads it.", ""]
groups = defaultdict(list)
for name in env:
    groups[name.split("_")[0]].append(name)
for prefix in sorted(groups):
    L += [f"### {prefix}_", ""]
    for name in sorted(groups[prefix]):
        sites = env[name]
        crates = sorted({s[0] for s in sites})
        first = f"{sites[0][1]}:{sites[0][2]}"
        L.append(f"- [{box(name)}] env `{name}` · {first} ({', '.join(crates)})")
        if len(sites) > 1:
            more = ", ".join(f"{s[1]}:{s[2]}" for s in sites[1:8])
            L.append(f"  - also read at {more}" + (f" (+{len(sites) - 8} more)" if len(sites) > 8 else ""))
    L.append("")

L += ["### Not runtime configuration", "",
      "Read at build time through `env!` / `option_env!`, set by Cargo; listed so nothing is dropped silently:", ""]
for name, sites in ENV["build_time"].items():
    L.append(f"- `{name}` · {sites[0][1]}:{sites[0][2]}")
L += ["", "Uppercase string literals that are not environment variables (reviewed):", ""]
for name, sites in ENV["rejected_candidates"].items():
    L.append(f"- `{name}` · {sites[0][1]}:{sites[0][2]}")

L += ["", "## Cargo features", ""]
current = None
for crate, d, name, on, enables, at in feature_rows:
    if crate != current:
        current = crate
        L += ["", f"### `{crate}`", ""]
    key = f"{crate}/{name}"
    where = f"{d}/Cargo.toml:{at}" if at else f"{d}/Cargo.toml"
    state = "default" if on else "off by default"
    L.append(f"- [{box(key)}] feature `{key}` · {where} ({state})")
    if enables:
        L.append("  - enables " + ", ".join(f"`{e}`" for e in enables))

OUT.write_text("\n".join(L) + "\n")
print(json.dumps({"env": len(env), "features": len(feature_rows), "checked_preserved": len(checked)}))
