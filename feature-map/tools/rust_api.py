"""Walk a crate's rustdoc JSON and emit its public API as JSON records.

The walk starts at the crate root and follows every public module and
`pub use`, so an item appears at each public path a user can name it by.
It is listed once, under its shortest public path, with the other paths
noted. rustdoc has already removed private items. The JSON is built with
`--document-hidden-items`, so `#[doc(hidden)]` items are kept and flagged
`hidden`: a path is hidden when the item, or any module or `use` on the way
to it, carries the attribute, and an item is hidden only when every public
path to it is.

Per type: inherent methods, associated constants and associated types each
get their own record. Implemented traits defined in the same crate family are
listed by name, and std or third-party traits by their last segment
(`Default`, `From`), so `Type::default()` can be checked. Traits list their
required and provided items, then their implementors. Enum variants and
public struct fields are listed inline.

Feature gates come from rustdoc's CfgTrace attributes, inherited down the
module and re-export chain; `--default` names the default-features JSON so
the gate labels can be cross-checked against what a default build exposes.
"""
import argparse
import hashlib
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
HIDDEN = re.compile(r'^#\[doc\((?:[^)]*,\s*)?hidden\b')


def is_hidden(item):
    return any(isinstance(a, dict) and HIDDEN.match(a.get("other", "")) for a in item.get("attrs") or [])


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
        self.hidden_paths = set()              # path tuples reached through a #[doc(hidden)] item
        self.external_reexports = []           # (path, source, kind, gates, hidden)
        self.modules = {}                      # path tuple -> (id, gates)
        self._walk(self.root, (self.name,), (), False, set())

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

    def _walk(self, mod_id, path, gates, hidden, seen):
        key = (mod_id, path)
        if key in seen:
            return
        seen.add(key)
        mod = self.idx[mod_id]
        self.modules.setdefault(path, (mod_id, gates))
        if hidden:
            self.hidden_paths.add(path)
        for iid in mod["inner"]["module"]["items"]:
            it = self.item(iid)
            if it is None:
                continue
            g = gates + tuple(cfg_of(it))
            h = hidden or is_hidden(it)
            k = kind(it)
            if k == "use":
                u = it["inner"]["use"]
                target = self.item(u["id"]) if u["id"] is not None else None
                if target is None:
                    src, k2 = self.resolve_external(u)
                    name = u["name"] if not u["is_glob"] else "*"
                    self.external_reexports.append((path + (name,), src, k2, g, h))
                    continue
                g2 = g + tuple(cfg_of(target))
                h2 = h or is_hidden(target)
                if u["is_glob"]:
                    if kind(target) == "module":
                        self._walk(str(u["id"]), path, g2, h2, seen)
                    continue
                if kind(target) == "module":
                    self._walk(str(u["id"]), path + (u["name"],), g2, h2, seen)
                else:
                    self._add(str(u["id"]), path + (u["name"],), g2, h2)
            elif k == "module":
                self._walk(str(iid), path + (it["name"],), g, h, seen)
            elif k in KIND_LABEL:
                self._add(str(iid), path + (it["name"],), g, h)

    def _add(self, iid, path, gates, hidden):
        self.public_paths[iid].add((path, gates))
        if hidden:
            self.hidden_paths.add(path)

    def hidden(self, iid):
        return all(p in self.hidden_paths for p, _ in self.public_paths[iid])

    # ---- presentation -------------------------------------------------
    def best(self, iid):
        """Shortest visible public path (hidden ones only as a last resort), and its gate set."""
        cands = sorted(self.public_paths[iid], key=lambda pg: (pg[0] in self.hidden_paths, len(pg[0]), pg[0]))
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
            local_traits, other_traits = set(), set()
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
                            out.append((mk, m["name"], span(m), m.get("deprecation"), "",
                                        is_hidden(m) or is_hidden(imp)))
                else:
                    crate, tname = self.trait_name(im["trait"])
                    if tname and tname in SIBLING_PUBLIC:
                        tname = SIBLING_PUBLIC[tname]
                    if crate in LOCAL_CRATES and tname:
                        local_traits.add(tname if tname.split("::")[0] in LOCAL_CRATES else f"{crate}::{tname}")
                    elif tname:
                        other_traits.add(tname.split("::")[-1])
            if local_traits:
                notes.append("Implements: " + ", ".join(f"`{t}`" for t in sorted(local_traits)))
            if other_traits:
                notes.append("Implements (std and third-party): " + ", ".join(f"`{t}`" for t in sorted(other_traits)))
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
                out.append((mk, m["name"], span(m), m.get("deprecation"), req, is_hidden(m)))
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


def _source_text(item, mode):
    """Declaration ("sig") or full ("body") source text of an item, whitespace-normalised.

    Comment lines are dropped so a doc edit is not an API change. For
    functions, traits and macros the declaration stops at the body; for data
    types, constants and aliases the whole definition is the declaration.
    """
    sp = item.get("span") if item else None
    if not sp:
        return ""
    path = Path(sp["filename"])
    if not path.exists():
        return ""
    lines = path.read_text().splitlines()[sp["begin"][0] - 1: sp["end"][0]]
    text = "\n".join(l for l in lines if not l.strip().startswith("//"))
    if mode == "sig" and kind(item) in ("function", "trait", "macro"):
        depth = 0
        for i, ch in enumerate(text):
            if ch in "([<":
                depth += 1
            elif ch in ")]>":
                depth -= 1
            elif depth <= 0 and ch in "{;":
                text = text[:i]
                break
    return " ".join(text.split())


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()[:16] if text else None


def member_ids(crate, iid):
    """Ids of a type's inherent-impl items, or a trait's items."""
    it = crate.idx[iid]
    k = kind(it)
    if k == "trait":
        return [str(m) for m in it["inner"]["trait"]["items"] if crate.item(m)]
    if k not in ("struct", "enum", "union"):
        return []
    ids = []
    for imp_id in it["inner"][k]["impls"]:
        imp = crate.item(imp_id)
        if imp and imp["inner"]["impl"]["trait"] is None:
            ids += [str(m) for m in imp["inner"]["impl"]["items"] if crate.item(m)]
    return ids


def notes_to_details(notes):
    """Structured form of the per-item notes members() produces."""
    fields = {"Variants": "variants", "Public fields": "fields", "Implements": "implements",
              "Implemented here by": "implemented_by", "Implements (std and third-party)": "implements_external", "Helper attributes": "helper_attributes",
              "Form": "form", "Public tuple fields": "tuple_fields"}
    d = {}
    for n in notes:
        key, _, rest = n.partition(": ")
        field = fields[key]
        if field == "tuple_fields":
            d[field] = int(rest)
        elif field == "form":
            d[field] = rest.replace("`", "")
        else:
            d[field] = re.findall(r"`([^`]+)`", rest)
    return d


def records(crate, default_crate, enabled=None):
    """One JSON record per public item and member. Returns (records, gate mismatches)."""
    default_paths = None
    if default_crate is not None:
        default_paths = {"::".join(p) for i in default_crate.public_paths
                         for p, _ in default_crate.public_paths[i]}
    out, mismatches, ids = [], [], set()

    def unique(rid):
        base, n = rid, 2
        while rid in ids:
            rid, n = f"{base}#{n}", n + 1
        ids.add(rid)
        return rid

    for path, src, k2, gates, hidden in sorted(crate.external_reexports):
        full = "::".join(path)
        out.append({"id": unique(full), "kind": "reexport", "parent": None, "family": "rust-api", "hidden": hidden,
                    "crate": crate.name, "module": "::".join(path[:-1]), "file": None, "line": None,
                    "feature": label_gates(gates, enabled) or None,
                    "details": {"target": src, "target_kind": k2 or None},
                    "sig_hash": digest(src + "|" + "|".join(gates)), "body_hash": None})

    for iid in sorted(crate.public_paths, key=lambda i: crate.best(i)[0]):
        it = crate.idx[iid]
        k = kind(it)
        path, gates = crate.best(iid)
        full = "::".join(path)
        canon = crate.paths.get(iid, {}).get("path") or list(path)
        mod_path = tuple(canon[:-1])
        details = {}
        if default_paths is not None:
            in_default = full in default_paths
            expected = all(gate_on(x, enabled) for x in gates)
            if in_default and not expected:
                details["stub_when_feature_off"] = True
            elif in_default != expected:
                mismatches.append(f"{full}: gates {list(dict.fromkeys(gates))} predict "
                                  f"{'present' if expected else 'absent'} in default build, "
                                  f"rustdoc says {'present' if in_default else 'absent'}")
        members, notes = crate.members(iid)
        details.update(notes_to_details(notes))
        if k == "type_alias":
            target = it["inner"]["type_alias"]["type"].get("resolved_path")
            if target:
                details["alias_of"] = target["path"]
        sp = it.get("span") or {}
        rid = unique(full)
        hidden = crate.hidden(iid)
        out.append({"id": rid, "kind": KIND_LABEL[k], "parent": None, "family": "rust-api", "hidden": hidden,
                    "crate": crate.name, "module": "::".join(mod_path),
                    "module_public": mod_path in crate.modules,
                    "file": sp.get("filename"), "line": (sp.get("begin") or [None])[0],
                    "feature": label_gates(gates, enabled) or None,
                    "deprecated": bool(it.get("deprecation")),
                    "also": sorted("::".join(p) for p, _ in crate.public_paths[iid] if p != path),
                    "details": details,
                    "sig_hash": digest(_source_text(it, "sig")),
                    "body_hash": digest(_source_text(it, "body"))})
        by_name_span = {(crate.idx[m]["name"], span(crate.idx[m])): crate.idx[m] for m in member_ids(crate, iid)}
        seen = set()
        for mem in members:
            mk, name, at, dep = mem[:4]
            req = mem[4] if len(mem) > 4 else ""
            mem_hidden = hidden or (mem[5] if len(mem) > 5 else False)
            if mk == "argument":
                file, line = at.rsplit(":", 1)
                src = Path(file).read_text().splitlines()[int(line) - 1].strip() if Path(file).exists() else ""
                out.append({"id": unique(f"{rid}#arg:{name.replace('`', '')}"), "kind": "argument",
                            "parent": rid, "family": "rust-api", "hidden": hidden, "crate": crate.name,
                            "module": "::".join(mod_path), "file": file, "line": int(line),
                            "details": {"syntax": name.replace("`", "")},
                            "sig_hash": digest(name + "|" + " ".join(src.split())), "body_hash": None})
                continue
            if (name, at) in seen:
                continue
            seen.add((name, at))
            mitem = by_name_span.get((name, at))
            file, _, line = at.rpartition(":") if at else (None, None, None)
            label = {"function": "fn", "assoc_const": "const", "assoc_type": "type"}[mk]
            out.append({"id": unique(f"{rid}::{name}"), "kind": label, "parent": rid, "family": "rust-api",
                        "hidden": mem_hidden,
                        "crate": crate.name, "module": "::".join(mod_path), "file": file or None,
                        "line": int(line) if line else None, "deprecated": bool(dep),
                        "details": {"trait_item": req} if req else {},
                        "sig_hash": digest(_source_text(mitem, "sig")),
                        "body_hash": digest(_source_text(mitem, "body"))})

    # Items rustdoc kept (reachable as a return type, field or supertrait) that no public path names.
    owned = set()
    for it in crate.idx.values():
        k = kind(it)
        if k in ("impl", "trait"):
            owned.update(str(m) for m in it["inner"][k]["items"])
    for iid, it in crate.idx.items():
        if it["crate_id"] != 0 or kind(it) not in KIND_LABEL or iid in crate.public_paths or iid in owned:
            continue
        canon = crate.paths.get(iid, {}).get("path") or [it["name"] or "?"]
        note = ("sealed trait: a supertrait that stops implementations outside the crate"
                if kind(it) == "trait" and it["name"] == "Sealed" else "public, but no public path names it")
        sp = it.get("span") or {}
        out.append({"id": unique("::".join(canon)), "kind": KIND_LABEL[kind(it)], "parent": None,
                    "family": "rust-api", "hidden": is_hidden(it), "crate": crate.name, "module": "::".join(canon[:-1]),
                    "module_public": False, "file": sp.get("filename"),
                    "line": (sp.get("begin") or [None])[0], "details": {"unnameable": note},
                    "sig_hash": digest(_source_text(it, "sig")),
                    "body_hash": digest(_source_text(it, "body"))})
    return out, mismatches


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("json")
    ap.add_argument("--default")
    ap.add_argument("--manifest")
    ap.add_argument("--sibling", action="append")
    ap.add_argument("--vocab")
    ap.add_argument("--out", required=True)
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
    enabled = default_features(a.manifest) if a.manifest else None
    recs, mismatches = records(crate, default_crate, enabled)
    with open(a.out, "w") as f:
        for r in recs:
            f.write(json.dumps(r, sort_keys=True) + "\n")
    print(json.dumps({"records": len(recs), "gate_mismatches": mismatches}))


if __name__ == "__main__":
    main()
