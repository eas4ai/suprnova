"""Live template surface as JSON records: directives, modifiers, runtime features, components.

Directives and modifier vocabularies come from the reviewed grammar
contract (`fixtures/v4/directive-grammar.json`), the file the checker's
generated contract and the browser runtime are both built from. Runtime
feature modules come from `fixtures/v4/runtime-features.json`. Components
come from each `components/<name>/manifest.json` and the `{% macro %}`
signatures in its Askama files.
"""
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
LIVE = ROOT / "crates/suprnova-live"
OUT = Path(sys.argv[2])


def digest(value):
    text = value if isinstance(value, str) else json.dumps(value, sort_keys=True)
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def line_of(path: Path, needle: str) -> int:
    for n, line in enumerate(path.read_text().splitlines(), 1):
        if needle in line:
            return n
    raise SystemExit(f"{needle!r} not found in {path}")


grammar_path = LIVE / "fixtures/v4/directive-grammar.json"
g = json.loads(grammar_path.read_text())
rel_g = str(grammar_path.relative_to(ROOT))
features_path = LIVE / "fixtures/v4/runtime-features.json"
feats = json.loads(features_path.read_text())
rel_f = str(features_path.relative_to(ROOT))
base = {"family": "live-templates", "crate": "suprnova_live", "parent": None}
recs = []

for d in g["directives"]:
    name = g["syntax"]["prefix"] + d["name"]
    details = {k: d[k] for k in ("owner", "value", "phase", "fallback", "modifiers", "conflicts", "roles", "capability")}
    recs.append({**base, "id": name, "kind": "directive", "module": "directives", "file": rel_g,
                 "line": line_of(grammar_path, f'"name":"{d["name"]}"'), "details": details,
                 "sig_hash": digest(d), "body_hash": None})

for key in ("event_modifiers", "model_modifiers", "feedback_modifiers", "morph_modifiers",
            "transition_modifiers", "navigation_modifiers", "freshness_combinations"):
    recs.append({**base, "id": key, "kind": "vocabulary", "module": "modifiers", "file": rel_g,
                 "line": line_of(grammar_path, f'"{key}"'), "details": {"values": g[key]},
                 "sig_hash": digest(g[key]), "body_hash": None})

for f in feats["features"]:
    recs.append({**base, "id": f["capability"], "kind": "runtime-feature", "module": "runtime features",
                 "file": rel_f, "line": line_of(features_path, f'"name":"{f["name"]}"'),
                 "details": {"name": f["name"], "artifact_roles": f["artifact_roles"],
                             "compatible_core": f["compatible_core"]},
                 "sig_hash": digest(f), "body_hash": None})

macro_re = re.compile(r'\{%-?\s*macro\s+([a-z_][a-z0-9_]*)\s*\(([^)]*)\)')
components = macros = 0
for d in sorted(p for p in (LIVE / "components").iterdir() if (p / "manifest.json").exists()):
    man = json.loads((d / "manifest.json").read_text())
    rel_m = str((d / "manifest.json").relative_to(ROOT))
    body = "".join((d / f).read_text() for f in man["files"])
    recs.append({**base, "id": man["name"], "kind": "component", "module": "component library",
                 "file": rel_m, "line": 1,
                 "details": {"files": man["files"], "installs_to": man["root"], "elements": man.get("elements", [])},
                 "sig_hash": digest(man), "body_hash": digest(body)})
    components += 1
    for f in man["files"]:
        fp = d / f
        if fp.suffix != ".html":
            continue
        for n, line in enumerate(fp.read_text().splitlines(), 1):
            for m in macro_re.finditer(line):
                sig = f"{m.group(1)}({' '.join(m.group(2).split())})"
                recs.append({**base, "id": f"{man['name']}::{m.group(1)}", "kind": "template-macro",
                             "parent": man["name"], "module": "component library",
                             "file": str(fp.relative_to(ROOT)), "line": n, "details": {"signature": sig},
                             "sig_hash": digest(sig), "body_hash": None})
                macros += 1

OUT.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in recs))
print(json.dumps({"records": len(recs), "directives": len(g["directives"]), "components": components,
                  "macros": macros}))
