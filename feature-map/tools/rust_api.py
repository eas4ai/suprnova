"""Walk a crate's rustdoc JSON and emit its public API as a checklist.

The walk starts at the crate root and follows every public module and
`pub use`, so an item appears at each public path a user can name it by.
It is listed once, under its shortest public path, with the other paths
noted. rustdoc has already removed private and `#[doc(hidden)]` items.

Per type: inherent methods, associated constants and associated types each
get a checkbox. Implemented traits defined in the same crate family are
listed by name. Traits list their required and provided items, then their
implementors. Enum variants and public struct fields are listed inline.

Feature gates come from rustdoc's CfgTrace attributes, inherited down the
module and re-export chain; `--default` names the default-features JSON so
the gate labels can be cross-checked against what a default build exposes.
"""
import argparse
import json
import re
from collections import defaultdict
from pathlib import Path

LOCAL_CRATES = {
    "suprnova", "suprnova_live", "suprnova_magnetar", "suprnova_payments_stripe",
    "suprnova_payments_paddle", "suprnova_payments_nowpayments", "suprnova_web_push",
    "suprnova_macros",
}
KIND_ORDER = ["proc_macro", "macro", "function", "struct", "enum", "union", "trait", "trait_alias",
              "type_alias", "constant", "static"]
KIND_LABEL = {"proc_macro": "proc macro", "macro": "macro", "function": "fn", "struct": "struct", "enum": "enum",
              "union": "union", "trait": "trait", "trait_alias": "trait alias",
              "type_alias": "type", "constant": "const", "static": "static"}
VOCAB = {}  # proc macro name -> [{"syntax", "at"}], from macro_vocab.py
SIBLING_PUBLIC = {}  # canonical path in a sibling crate -> its shortest public path
CFG = re.compile(r'CfgTrace\((.*)\)\]')


def default_features(manifest):
    """Default features of a crate, expanded through local feature references."""
    import tomllib
    feats = tomllib.loads(Path(manifest).read_text()).get("features", {})
    on, todo = set(), list(feats.get("default", []))
    while todo:
        f = todo.pop()
        if f in on or f.startswith("dep:") or "/" in f:
            continue
        on.add(f)
        todo.extend(feats.get(f, []))
    return on


def gate_on(gate, enabled):
    names = re.findall(r"`([^`]+)`", gate)
    return any(n in enabled for n in names) if " or " in gate else all(n in enabled for n in names)


def label_gates(gates, enabled):
    out = []
    for g in dict.fromkeys(gates):
        out.append(g if enabled is None or gate_on(g, enabled) else f"{g}, off by default")
    return "; ".join(out)


def kind(item):
    return next(iter(item["inner"]))


def cfg_of(item):
    """Human-readable cfg conditions on this item (feature gates only)."""
    out = []
    for a in item.get("attrs") or []:
        s = a["other"] if isinstance(a, dict) and "other" in a else ""
        m = CFG.search(s)
        if not m:
            continue
        feats = re.findall(r'name: "feature", value: Some\("([^"]+)"\)', m.group(1))
        if not feats:
            continue
        joiner = " or " if m.group(1).lstrip("[").startswith("Any") else " and "
        out.append(joiner.join(f"`{f}`" for f in feats))
    return out


def span(item):
    sp = item.get("span")
    if not sp:
        return ""
    return f'{sp["filename"]}:{sp["begin"][0]}'


class Crate:
    def __init__(self, path):
        d = json.loads(Path(path).read_text())
        self.d = d
        self.idx = d["index"]
        self.paths = d["paths"]
        self.ext = d["external_crates"]
        self.root = str(d["root"])
        self.name = self.idx[self.root]["name"]
        self.public_paths = defaultdict(set)   # item id -> {(path tuple, gates tuple)}
        self.external_reexports = []           # (path, source, gates)
        self.modules = {}                      # path tuple -> (id, gates)
        self._walk(self.root, (self.name,), (), set())

    def resolve_external(self, u):
        """Defining crate path and kind of an external re-export target."""
        p = self.paths.get(str(u["id"])) if u["id"] is not None else None
        if p is None:
            return u["source"], ""
        crate = self.ext.get(str(p["crate_id"]), {}).get("name", "?")
        path = list(p["path"])
        if path and path[0] != crate:
            path = [crate] + path[1:]
        return "::".join(path), p["kind"].replace("_", " ")

    def item(self, i):
        return self.idx.get(str(i))

    def _walk(self, mod_id, path, gates, seen):
        key = (mod_id, path)
        if key in seen:
            return
        seen.add(key)
        mod = self.idx[mod_id]
        self.modules.setdefault(path, (mod_id, gates))
        for iid in mod["inner"]["module"]["items"]:
            it = self.item(iid)
            if it is None:
                continue
            g = gates + tuple(cfg_of(it))
            k = kind(it)
            if k == "use":
                u = it["inner"]["use"]
                target = self.item(u["id"]) if u["id"] is not None else None
                if target is None:
                    src, k2 = self.resolve_external(u)
                    name = u["name"] if not u["is_glob"] else "*"
                    self.external_reexports.append((path + (name,), src, k2, g))
                    continue
                g2 = g + tuple(cfg_of(target))
                if u["is_glob"]:
                    if kind(target) == "module":
                        self._walk(str(u["id"]), path, g2, seen)
                    continue
                if kind(target) == "module":
                    self._walk(str(u["id"]), path + (u["name"],), g2, seen)
                else:
                    self.public_paths[str(u["id"])].add((path + (u["name"],), g2))
            elif k == "module":
                self._walk(str(iid), path + (it["name"],), g, seen)
            elif k in KIND_LABEL:
                self.public_paths[str(iid)].add((path + (it["name"],), g))

    # ---- presentation -------------------------------------------------
    def best(self, iid):
        """Shortest public path, and the union-free gate set of that path."""
        cands = sorted(self.public_paths[iid], key=lambda pg: (len(pg[0]), pg[0]))
        return cands[0]

    def trait_name(self, trait_ref):
        tid = str(trait_ref.get("id"))
        if tid in self.public_paths:
            return self.name, "::".join(self.best(tid)[0])
        p = self.paths.get(tid)
        if p is None:
            return None, trait_ref.get("path")
        crate = self.ext.get(str(p["crate_id"]), {}).get("name") if p["crate_id"] != 0 else self.name
        return crate, "::".join(p["path"])

    def members(self, iid):
        """Checkbox members of a type or trait: (label, path_suffix, span, extra)."""
        it = self.idx[iid]
        k = kind(it)
        out, notes = [], []
        if k in ("struct", "enum", "union"):
            inner = it["inner"][k]
            if k == "enum":
                names = [self.idx[str(v)]["name"] for v in inner["variants"] if self.item(v)]
                if names:
                    notes.append("Variants: " + ", ".join(f"`{n}`" for n in names))
            if k == "struct":
                sk = inner["kind"]
                if "plain" in sk:
                    names = [self.idx[str(f)]["name"] for f in sk["plain"]["fields"] if self.item(f)]
                    if names:
                        notes.append("Public fields: " + ", ".join(f"`{n}`" for n in names))
                elif "tuple" in sk:
                    n = sum(1 for f in sk["tuple"] if f is not None)
                    if n:
                        notes.append(f"Public tuple fields: {n}")
            local_traits = set()
            for imp_id in inner["impls"]:
                imp = self.item(imp_id)
                if imp is None:
                    continue
                im = imp["inner"]["impl"]
                if im["is_synthetic"] or im["blanket_impl"] is not None:
                    continue
                if im["trait"] is None:
                    for mid in im["items"]:
                        m = self.item(mid)
                        if m is None or m.get("visibility") != "public":
                            continue
                        mk = kind(m)
                        if mk in ("function", "assoc_const", "assoc_type"):
                            out.append((mk, m["name"], span(m), m.get("deprecation")))
                else:
                    crate, tname = self.trait_name(im["trait"])
                    if tname and tname in SIBLING_PUBLIC:
                        tname = SIBLING_PUBLIC[tname]
                    if crate in LOCAL_CRATES and tname:
                        local_traits.add(tname if tname.split("::")[0] in LOCAL_CRATES else f"{crate}::{tname}")
            if local_traits:
                notes.append("Implements: " + ", ".join(f"`{t}`" for t in sorted(local_traits)))
        elif k == "proc_macro":
            pm = it["inner"]["proc_macro"]
            nm = it["name"]
            form = {"bang": f"function-like `{nm}!(...)`", "attr": f"attribute `#[{nm}]`",
                    "derive": f"derive `#[derive({nm})]`"}.get(pm["kind"], pm["kind"])
            notes.append(f"Form: {form}")
            if pm.get("helpers"):
                notes.append("Helper attributes: " + ", ".join(f"`#[{h}]`" for h in pm["helpers"]))
            for arg in VOCAB.get(nm, []):
                out.append(("argument", arg["syntax"], arg["at"], None))
        elif k == "trait":
            inner = it["inner"]["trait"]
            for mid in inner["items"]:
                m = self.item(mid)
                if m is None:
                    continue
                mk = kind(m)
                req = ""
                if mk == "function":
                    req = "provided" if m["inner"]["function"]["has_body"] else "required"
                out.append((mk, m["name"], span(m), m.get("deprecation"), req))
            impls = []
            for imp_id in inner["implementations"]:
                imp = self.item(imp_id)
                if imp is None:
                    continue
                target = imp["inner"]["impl"]["for"]
                rp = target.get("resolved_path")
                if rp and rp.get("id") is not None and str(rp["id"]) in self.public_paths:
                    impls.append("::".join(self.best(str(rp["id"]))[0][1:]))
                elif rp:
                    impls.append(rp["path"])
            if impls:
                notes.append("Implemented here by: " + ", ".join(f"`{i}`" for i in sorted(set(impls))))
        return out, notes


def render(crate, default_crate, checked, header, enabled=None):
    groups = defaultdict(list)
    for iid in crate.public_paths:
        path, gates = crate.best(iid)
        canon = crate.paths.get(iid, {}).get("path") or list(path)
        groups[tuple(canon[:-1])].append((iid, path, gates))
    default_paths = None
    if default_crate is not None:
        default_paths = {"::".join(crate_p) for i in default_crate.public_paths
                         for crate_p, _ in default_crate.public_paths[i]}
    lines = list(header)
    stats = defaultdict(int)
    mismatches = []
    # External re-exports first, they are part of the surface too.
    if crate.external_reexports:
        lines += ["", "## Re-exported from other crates", "",
                  "Items from sibling Suprnova crates are mapped in those crates' files.", ""]
        for path, src, k2, gates in sorted(crate.external_reexports):
            g = f" (feature: {label_gates(gates, enabled)})" if gates else ""
            box = "x" if "::".join(path) in checked else " "
            kl = f"{k2} " if k2 else ""
            lines.append(f"- [{box}] {kl}`{'::'.join(path)}` re-exports `{src}`{g}")
            stats["reexport"] += 1
    current_area = None
    for mod_path in sorted(groups, key=lambda m: (len(m) > 1, m)):
        area = mod_path[1] if len(mod_path) > 1 else "(crate root)"
        if area != current_area:
            current_area = area
            lines += ["", f"## {area}"]
        mod_id, mod_gates = crate.modules.get(mod_path, (None, ()))
        title = "::".join(mod_path)
        g = f" (feature: {label_gates(mod_gates, enabled)})" if mod_gates else ""
        priv = "" if mod_path in crate.modules else " (private module; items are public through re-exports)"
        lines += ["", f"### `{title}`{g}{priv}", ""]
        entries = sorted(groups[mod_path], key=lambda e: (
            KIND_ORDER.index(kind(crate.idx[e[0]])) if kind(crate.idx[e[0]]) in KIND_ORDER else 99,
            e[1][-1].lower()))
        for iid, path, gates in entries:
            it = crate.idx[iid]
            k = kind(it)
            full = "::".join(path)
            extra = []
            item_gates = tuple(x for x in gates if x not in mod_gates)
            if item_gates:
                extra.append(f"feature: {label_gates(item_gates, enabled)}")
            if it.get("deprecation"):
                extra.append("deprecated")
            if default_paths is not None:
                in_default = full in default_paths
                expected = all(gate_on(x, enabled) for x in gates)
                if in_default and not expected:
                    extra.append("a stub with the same name exists when the feature is off")
                elif in_default != expected:
                    mismatches.append(f"{full}: gates {list(dict.fromkeys(gates))} predict "
                                      f"{'present' if expected else 'absent'} in default build, "
                                      f"rustdoc says {'present' if in_default else 'absent'}")
            others = sorted("::".join(p) for p, _ in crate.public_paths[iid] if p != path)
            if others:
                extra.append("also " + ", ".join(f"`{o}`" for o in others))
            box = "x" if full in checked else " "
            suffix = f" ({'; '.join(extra)})" if extra else ""
            lines.append(f"- [{box}] {KIND_LABEL[k]} `{full}` · {span(it)}{suffix}")
            stats[k] += 1
            members, notes = crate.members(iid)
            for n in notes:
                lines.append(f"  - {n}")
            seen = set()
            for mem in members:
                mk, name, sp, dep = mem[:4]
                req = mem[4] if len(mem) > 4 else ""
                if mk == "argument":
                    aid = f"{full} {name}"
                    box = "x" if aid in checked else " "
                    lines.append(f"  - [{box}] argument {name} · {sp}")
                    stats["member"] += 1
                    continue
                mpath = f"{full}::{name}"
                if (mpath, sp) in seen:
                    continue
                seen.add((mpath, sp))
                label = {"function": "fn", "assoc_const": "const", "assoc_type": "type"}[mk]
                bits = [b for b in (req, "deprecated" if dep else "") if b]
                box = "x" if mpath in checked else " "
                lines.append(f"  - [{box}] {label} `{mpath}` · {sp}" +
                             (f" ({'; '.join(bits)})" if bits else ""))
                stats["member"] += 1
    # Items rustdoc kept (so they are reachable from the public API, e.g. as a
    # return type or supertrait) but that no public path names.
    unnamed = []
    owned = set()
    for it in crate.idx.values():
        k = kind(it)
        if k in ("impl", "trait"):
            owned.update(str(m) for m in it["inner"][k]["items"])
    for iid, it in crate.idx.items():
        if (it["crate_id"] != 0 or kind(it) not in KIND_LABEL or iid in crate.public_paths
                or iid in owned):
            continue
        canon = "::".join(crate.paths.get(iid, {}).get("path") or [it["name"] or "?"])
        note = "sealed trait: a supertrait that stops implementations outside the crate" if (
            kind(it) == "trait" and it["name"] == "Sealed") else "public, but no public path names it"
        unnamed.append((canon, kind(it), span(it), note))
    if unnamed:
        lines += ["", "## Public but unnameable", "",
                  "Reachable from the public API (as a return type, field or supertrait) "
                  "but not importable by any public path.", ""]
        for canon, k, sp, note in sorted(unnamed):
            box = "x" if canon in checked else " "
            lines.append(f"- [{box}] {KIND_LABEL[k]} `{canon}` · {sp} ({note})")
            stats["unnameable"] += 1
    return lines, stats, mismatches


def load_checked(existing):
    """IDs of checked lines in an existing map, so regeneration keeps them."""
    checked = set()
    parent = None
    if existing and Path(existing).exists():
        for line in Path(existing).read_text().splitlines():
            top = re.match(r'- \[[ x]\] (?:[a-z ]+ )?`([^`]+)`', line)
            if top:
                parent = top.group(1)
            arg = re.match(r'\s+- \[x\] argument (.+) · ', line)
            if arg and parent:
                checked.add(f"{parent} {arg.group(1)}")
                continue
            m = re.match(r'\s*- \[x\] (?:[a-z ]+ )?`([^`]+)`', line)
            if m:
                checked.add(m.group(1))
    return checked


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("json")
    ap.add_argument("--default")
    ap.add_argument("--manifest")
    ap.add_argument("--sibling", action="append")
    ap.add_argument("--vocab")
    ap.add_argument("--out", required=True)
    ap.add_argument("--title", required=True)
    ap.add_argument("--source-note", required=True)
    a = ap.parse_args()
    if a.vocab:
        VOCAB.update(json.loads(Path(a.vocab).read_text())["arguments"])
    for sib in a.sibling or []:
        sc = Crate(sib)
        for iid in sc.public_paths:
            canon = sc.paths.get(iid, {}).get("path")
            if canon:
                SIBLING_PUBLIC["::".join(canon)] = "::".join(sc.best(iid)[0])
    crate = Crate(a.json)
    default_crate = Crate(a.default) if a.default else None
    checked = load_checked(a.out)
    header = [
        f"# {a.title}",
        "",
        a.source_note,
        "",
        "A checked box means the documentation for that item has been remediated",
        "against the source. Items are listed under their shortest public path;",
        "`also` names the other paths the same item is reachable by.",
    ]
    enabled = default_features(a.manifest) if a.manifest else None
    lines, stats, mismatches = render(crate, default_crate, checked, header, enabled)
    total_items = sum(v for k, v in stats.items() if k not in ("member",))
    summary = ["", "## Counts", "",
               f"- Top-level items (including re-exports): {total_items}",
               f"- Members (methods, associated consts and types): {stats['member']}",
               "- By kind: " + ", ".join(f"{k} {v}" for k, v in sorted(stats.items()) if k != "member")]
    lines[7:7] = summary
    Path(a.out).write_text("\n".join(lines) + "\n")
    report = {"stats": stats, "gate_mismatches": mismatches, "checked_preserved": len(checked)}
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
