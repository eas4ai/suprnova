"""Live template surface: directives, modifiers, runtime features, components.

Directives and modifier vocabularies come from the reviewed grammar
contract (`fixtures/v4/directive-grammar.json`), the file the checker's
generated contract and the browser runtime are both built from. Runtime
feature modules come from `fixtures/v4/runtime-features.json`. Components
come from each `components/<name>/manifest.json` and the `{% macro %}`
signatures in its Askama files.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
LIVE = ROOT / "crates/suprnova-live"
REV = sys.argv[2]
OUT = Path(sys.argv[3])

checked = set()
if OUT.exists():
    for line in OUT.read_text().splitlines():
        m = re.match(r'\s*- \[x\] (?:[a-z ]+ )?`([^`]+)`', line)
        if m:
            checked.add(m.group(1))


def box(key):
    return "x" if key in checked else " "


def line_of(path: Path, needle: str) -> int:
    for n, line in enumerate(path.read_text().splitlines(), 1):
        if needle in line:
            return n
    raise SystemExit(f"{needle!r} not found in {path}")


grammar_path = LIVE / "fixtures/v4/directive-grammar.json"
g = json.loads(grammar_path.read_text())
rel_g = grammar_path.relative_to(ROOT)
features_path = LIVE / "fixtures/v4/runtime-features.json"
feats = json.loads(features_path.read_text())

L = [
    "# Suprnova feature map: Live template surface",
    "",
    f"Source: `crates/suprnova-live/` at {REV}. Directives and modifiers from "
    f"`{rel_g}` (contract version {g['contract_version']}); runtime features from "
    f"`{features_path.relative_to(ROOT)}`; components from `components/*/manifest.json` "
    "and their `{% macro %}` signatures.",
    "",
    "A checked box means the documentation for that item has been remediated against the source.",
    "",
    "## Counts",
    "",
    f"- Directives: {len(g['directives'])}",
]
comp_dirs = sorted(p for p in (LIVE / "components").iterdir() if (p / "manifest.json").exists())
macro_re = re.compile(r'\{%-?\s*macro\s+([a-z_][a-z0-9_]*)\s*\(([^)]*)\)')
components = []
for d in comp_dirs:
    man = json.loads((d / "manifest.json").read_text())
    macros = []
    for f in man["files"]:
        fp = d / f
        if fp.suffix == ".html":
            for n, line in enumerate(fp.read_text().splitlines(), 1):
                for m in macro_re.finditer(line):
                    macros.append((m.group(1), " ".join(m.group(2).split()), f"{fp.relative_to(ROOT)}:{n}"))
    components.append((d, man, macros))
L.append(f"- Components: {len(components)} ({sum(len(c[2]) for c in components)} macros)")
L.append(f"- Runtime feature modules: {len(feats['features'])}")

L += ["", "## Directives", "",
      f"Prefix `{g['syntax']['prefix']}`. Owner is where the directive may appear "
      "(`island`, `keyed_scope`, `element`); value is the argument form.", ""]
for d in g["directives"]:
    name = g["syntax"]["prefix"] + d["name"]
    bits = [f"owner {d['owner']}", f"value {d['value']}", f"phase {d['phase']}",
            f"fallback {d['fallback']}"]
    if d["modifiers"]:
        bits.append("modifiers " + ", ".join(f"`{m}`" for m in d["modifiers"]))
    if d["conflicts"]:
        bits.append("conflicts " + ", ".join(f"`{c}`" for c in d["conflicts"]))
    if d["roles"]:
        bits.append("roles " + ", ".join(d["roles"]))
    if d["capability"]:
        bits.append(f"needs runtime feature `{d['capability']}`")
    at = line_of(grammar_path, f'"name":"{d["name"]}"')
    L.append(f"- [{box(name)}] directive `{name}` · {rel_g}:{at}")
    L.append(f"  - {'; '.join(bits)}")

L += ["", "## Modifier vocabularies", ""]
for key in ("event_modifiers", "model_modifiers", "feedback_modifiers", "morph_modifiers",
            "transition_modifiers", "navigation_modifiers"):
    at = line_of(grammar_path, f'"{key}"')
    label = key.replace("_", " ")
    L.append(f"- [{box(key)}] {label} `{key}` · {rel_g}:{at}")
    L.append("  - " + ", ".join(f"`{m}`" for m in g[key]))
at = line_of(grammar_path, '"freshness_combinations"')
L.append(f"- [{box('freshness_combinations')}] poll and stream combinations `freshness_combinations` · {rel_g}:{at}")
for c in g["freshness_combinations"]:
    L.append(f"  - poll {str(c['poll']).lower()}, stream {c['stream']}: {c['result']}")

L += ["", "## Runtime feature modules", ""]
for f in feats["features"]:
    at = line_of(features_path, f'"name":"{f["name"]}"')
    L.append(f"- [{box(f['capability'])}] runtime feature `{f['capability']}` · "
             f"{features_path.relative_to(ROOT)}:{at}")
    L.append(f"  - artifacts {', '.join(f['artifact_roles'])}")

L += ["", "## Component library", "",
      "Installed with `suprnova live:add <component>`. Each macro is the component's "
      "public template API; parameters with `=` have defaults.", ""]
for d, man, macros in components:
    mp = (d / "manifest.json").relative_to(ROOT)
    L.append(f"- [{box(man['name'])}] component `{man['name']}` · {mp}:1")
    files = ", ".join(f"`{f}`" for f in man["files"])
    L.append(f"  - files {files}; installs to `{man['root']}`")
    if man.get("elements"):
        L.append("  - custom elements " + ", ".join(f"`{e}`" for e in man["elements"]))
    for name, params, at in macros:
        key = f"{man['name']}::{name}"
        L.append(f"  - [{box(key)}] macro `{key}` · {at}")
        L.append(f"    - `{name}({params})`")

OUT.write_text("\n".join(L) + "\n")
print(json.dumps({"directives": len(g["directives"]), "components": len(components),
                  "macros": sum(len(c[2]) for c in components), "checked_preserved": len(checked)}))
