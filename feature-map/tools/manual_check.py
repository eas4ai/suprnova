"""Check every code reference in the manual against the Suprnova surface.

Parses each English chapter with a CommonMark parser (markdown-it-py) and
each Rust code block with tree-sitter-rust, then resolves every reference to
a record in feature-map/surface.jsonl. Nothing is matched by regex over raw
markdown text.

Outcomes per reference:
  found          resolves to a Suprnova record (the matched id is reported)
  hidden         resolves to a `#[doc(hidden)]` record: it exists, but is not public API by intent
  missing        names a Suprnova API that does not exist
  laravel_only   not in Suprnova, but is a Laravel name (expected in parity/from-laravel)
  weak_found     a bare method call whose name exists on some Suprnova type
  weak_missing   a bare method call no Suprnova type has (may be user code)
  external       std or a third-party crate
  local          defined or imported by the snippet itself
  unresolved     a bare type or word the check cannot attribute (often example code)
  ambiguous      `Type::member` where Type is a Suprnova type without that member, but a Suprnova
                 trait has it: usually the reader's own model sharing a framework type's name
  external_unverified  a lowercase crate the check doesn't know, assumed third-party
  wrong_path     the item exists, but not at the path written (the match is its real public path)
  time_claim     a prose sentence that states the code's state in time ("today", "not yet", "v1", ...):
                 the ref is the sentence itself, so editing it invalidates its verdict (MAN-107)

Refuses to run when feature-map/meta.json's source_rev is not the last commit that touched
the source paths: findings against a stale surface would be wrong (MAN-002).

Usage: manual_check.py <repo> <out.jsonl>
Needs the venv from tools/manual_check_env.sh (markdown-it-py, tree-sitter, tree-sitter-rust).
"""
import json
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

from markdown_it import MarkdownIt
import tree_sitter_rust
from tree_sitter import Language, Parser

ROOT = Path(sys.argv[1])
OUT = Path(sys.argv[2])
RUST = Parser(Language(tree_sitter_rust.language()))

LOCAL_CRATES = {"suprnova", "suprnova_live", "magnetar", "suprnova_macros", "suprnova_web_push",
                "suprnova_payments_stripe", "suprnova_payments_paddle", "suprnova_payments_nowpayments"}
EXTERNAL_CRATES = {
    "std", "core", "alloc", "tokio", "serde", "serde_json", "sea_orm", "sea_query", "chrono", "chrono_tz",
    "inventory", "async_trait", "http", "hyper", "axum", "anyhow", "thiserror", "tracing", "futures", "bytes",
    "uuid", "validator", "askama", "reqwest", "rand", "once_cell", "regex", "url", "tower", "redis", "sqlx",
    "time", "dashmap", "parking_lot", "fake", "dummy", "indexmap", "opendal", "rust_decimal", "iso_currency",
    "webauthn_rs", "opentelemetry", "tracing_subscriber", "clap", "dotenvy", "sha2", "hmac", "base64", "hex",
    "mockito", "wiremock", "tempfile", "insta", "pretty_assertions", "serde_urlencoded", "mime", "log",
    "stripe", "paddle_rust_sdk", "web_push", "strum", "heck", "tap", "secrecy", "zeroize",
    "serial_test", "sea_orm_migration", "tokio_stream", "tower_http", "serde_with", "hyper_util",
    "featureflag", "lettre", "tracing_futures",
}
# std trait method -> the traits that provide it (None: a blanket impl, nothing to check).
STD_TRAIT_METHODS = {
    "default": {"Default"}, "clone": {"Clone"}, "from": {"From"}, "try_from": {"TryFrom"},
    "fmt": {"Debug", "Display"}, "eq": {"PartialEq"}, "ne": {"PartialEq"}, "to_string": {"Display", "ToString"},
    "from_str": {"FromStr"}, "hash": {"Hash"}, "cmp": {"Ord"}, "partial_cmp": {"PartialOrd"},
    "as_ref": {"AsRef"}, "deref": {"Deref"}, "borrow": {"Borrow"}, "drop": {"Drop"},
    "into": None, "try_into": None,
}
SUPRNOVA_COMMAND_NAMESPACES = ("make:", "migrate", "queue:", "schedule:", "live:", "db:", "docker:", "ssr:",
                               "workflow:", "key:", "dev:", "render-cache:", "model:", "web:", "generate-types")
STD_NAMES = {
    "Option", "Some", "None", "Result", "Ok", "Err", "Vec", "String", "Box", "Arc", "Rc", "HashMap", "HashSet",
    "BTreeMap", "BTreeSet", "VecDeque", "Default", "Clone", "Debug", "Into", "From", "TryFrom", "TryInto",
    "Iterator", "IntoIterator", "ToString", "Send", "Sync", "Sized", "Fn", "FnOnce", "FnMut", "Duration",
    "Instant", "SystemTime", "PhantomData", "Cow", "Mutex", "RwLock", "AsRef", "AsMut", "Display", "Error",
    "Self", "PartialEq", "Eq", "Hash", "Ord", "PartialOrd", "Copy", "Serialize", "Deserialize", "Future",
    "Pin", "Path", "PathBuf", "Value", "Utc", "DateTime", "NaiveDate", "NaiveDateTime", "Uuid", "Decimal",
    "Cell", "RefCell", "Ordering", "Infallible", "Formatter", "Json", "Bytes", "Local", "Tz", "Regex",
    "Borrow", "Deref", "DerefMut", "Drop", "Any", "TypeId", "Wrapping", "NonZeroU64", "NonZeroUsize",
    "OnceLock", "OnceCell", "LazyLock", "AtomicUsize", "AtomicBool", "AtomicU64", "Sender", "Receiver",
    "JoinHandle", "Stream", "StreamExt", "SinkExt", "FromStr", "Write", "Read", "BufRead", "Seek",
    "Validate", "Dummy", "EntityTrait", "ActiveModelTrait", "ColumnTrait", "QueryFilter", "DeriveEntityModel",
    "DeriveRelation", "EnumIter", "DeriveActiveEnum", "DeriveMigrationName", "MigrationTrait", "SchemaManager",
    "DbErr", "DatabaseConnection", "DatabaseTransaction", "ConnectionTrait", "ActiveModelBehavior", "Set",
    "NotSet", "Unchanged", "ActiveValue", "Condition", "Expr", "Alias", "Iden", "Table", "ColumnDef",
    "Index", "ForeignKey", "HeaderMap", "HeaderValue", "Method", "StatusCode", "Uri", "Incoming",
}
STD_DERIVES = {"Clone", "Debug", "Default", "PartialEq", "Eq", "Hash", "PartialOrd", "Ord", "Copy",
               "Serialize", "Deserialize", "Validate", "Dummy", "DeriveEntityModel", "DeriveRelation",
               "EnumIter", "DeriveActiveEnum", "DeriveMigrationName", "DeriveIden", "Iden", "Error",
               "EnumString", "Display", "thiserror::Error"}
STD_ATTRS = {"derive", "cfg", "test", "allow", "deny", "warn", "must_use", "inline", "doc", "serde",
             "sea_orm", "tokio::main", "tokio::test", "async_trait", "async_trait::async_trait",
             "validate", "error", "non_exhaustive", "repr", "dummy", "strum", "cfg_attr", "track_caller",
             "expect", "path", "ignore", "should_panic", "deprecated", "macro_export", "rustfmt::skip",
             "serial", "arg", "clap", "instrument"}
STD_MACROS = {"println", "print", "eprintln", "format", "vec", "assert", "assert_eq", "assert_ne", "panic",
              "todo", "unimplemented", "unreachable", "matches", "write", "writeln", "dbg", "json", "env",
              "include_str", "concat", "stringify", "debug_assert", "debug_assert_eq", "cfg", "select",
              "join", "try_join", "pin", "info", "warn", "error", "debug", "trace", "span", "info_span",
              "anyhow", "bail", "ensure", "lazy_static", "thread_local", "compile_error", "file", "line",
              "column", "module_path", "option_env", "include_bytes", "format_args", "submit"}


# ---- Refuse a stale surface ---------------------------------------------------------------
meta = json.loads((ROOT / "feature-map/meta.json").read_text())
head = subprocess.run(["git", "-C", str(ROOT), "log", "-1", "--format=%h", "--", *meta["source_paths"]],
                      capture_output=True, text=True, check=True).stdout.strip()
if head != meta["source_rev"]:
    sys.exit(f"surface built from {meta['source_rev']} but source is now at {head}; "
             "run feature-map/tools/generate.sh first")

# ---- The Suprnova (and Laravel) name index ------------------------------------------------
surface = [json.loads(l) for l in (ROOT / "feature-map/surface.jsonl").read_text().splitlines()]
by_path = {}                       # any full path -> record id
by_suffix = defaultdict(set)       # "Type::member" / "Name" suffixes -> record ids
member_names = defaultdict(set)    # bare member name -> record ids
proc_macros = {}                   # name -> record
helpers = defaultdict(set)         # helper attribute -> proc macro ids
arguments = defaultdict(set)       # proc macro id -> argument keywords
envs, commands, directives = set(), {}, set()
record = {r["id"]: r for r in surface}
for r in surface:
    rid = r["id"]
    if r["family"] == "rust-api":
        paths = [rid] + r.get("also", []) + r.get("aliases", [])
        for p in paths:
            by_path[p] = rid
            segs = p.split("::")
            for i in range(1, len(segs)):
                by_suffix["::".join(segs[i:])].add(rid)
        if r["parent"] and r["kind"] in ("fn", "const", "type"):
            member_names[rid.split("::")[-1].split("#")[0]].add(rid)
        if r["kind"] == "proc macro":
            for p in paths:  # a renamed re-export (`FormRequest as FormRequestDerive`) is usable by its alias
                proc_macros.setdefault(p.split("::")[-1], r)
            name = rid.split("::")[-1]
            proc_macros[name] = r
            for h in r["details"].get("helper_attributes", []):
                helpers[h.strip("#[]")].add(rid)
        if r["kind"] == "argument":
            for word in re.findall(r"\b([a-z_]+)\b", r["details"]["syntax"]):
                arguments[r["parent"]].add(word)
            for inner in re.findall(r"#\[([a-z_]+)", r["details"]["syntax"]):
                helpers[inner].add(r["parent"])
    elif r["kind"] == "env":
        envs.add(rid)
    elif r["kind"] == "command":
        binary, name = rid.split(" ", 1)
        commands.setdefault(name, []).append(r)
    elif r["kind"] == "directive":
        directives.add(rid)

# A member is reachable under every public path of its parent, not only the shortest one.
parent_paths = {r["id"]: [r["id"]] + r.get("also", []) + r.get("aliases", []) for r in surface
                if r["family"] == "rust-api" and r["parent"] is None}
for r in surface:
    if r["family"] == "rust-api" and r["parent"] in parent_paths and r["kind"] in ("fn", "const", "type"):
        name = r["id"].split("::")[-1].split("#")[0]
        for pp in parent_paths[r["parent"]]:
            by_path.setdefault(f"{pp}::{name}", r["id"])
            segs = f"{pp}::{name}".split("::")
            for i in range(1, len(segs)):
                by_suffix["::".join(segs[i:])].add(r["id"])

modules = set()                    # every public module path (prefixes of public item paths)
for p in list(by_path):
    segs = p.split("::")
    for i in range(1, len(segs)):
        modules.add("::".join(segs[:i]))
for r in surface:                  # Enum::Variant and Struct::field resolve to their type
    d = r.get("details") if isinstance(r.get("details"), dict) else {}
    for v in d.get("variants", []) + d.get("fields", []):
        for p in [r["id"]] + r.get("also", []) + r.get("aliases", []):
            segs = (p + "::" + v).split("::")
            for i in range(0, len(segs) - 1):
                by_suffix["::".join(segs[i:])].add(r["id"])
            by_path[p + "::" + v] = r["id"]
APP_CRATES = {"my_app", "app", "your_app", "my_project"}
reexports = {}                     # suprnova paths that re-export an external module, e.g. suprnova::sea_orm
reexport_names = defaultdict(set)  # last segment -> re-export ids, for a bare `Currency::USD`
for r in surface:
    if r["kind"] == "reexport":  # items under a re-exported external module or type are external
        reexports[r["id"]] = r
        reexport_names[r["id"].split("::")[-1]].add(r["id"])
build_time_env = set(json.loads((ROOT / "feature-map/exclusions.json").read_text()).get("build_time_env", {}))

laravel = []
lpath = ROOT / "feature-map/laravel/surface.jsonl"
if lpath.exists():
    laravel = [json.loads(l) for l in lpath.read_text().splitlines()]
laravel_envs = {r["id"].split(" ", 1)[1] for r in laravel if r["kind"] == "env var"}
laravel_names = set()
for r in laravel:
    rid = r["id"]
    if r["family"] == "laravel-api":
        cls = rid.split("::")[0].split("\\")[-1]
        laravel_names.add(cls)
        if "::" in rid:
            laravel_names.add(f"{cls}::{rid.split('::', 1)[1].lstrip('$')}")
    elif " " in rid:
        laravel_names.add(rid.split(" ", 1)[1])
    else:
        laravel_names.add(rid)


def type_member(segs):
    """`Type::member` where Type is one Suprnova type and member is a std trait method, or Type is an
    alias: check the trait impl, or resolve on the alias target. None when neither applies."""
    tpath = "::".join(segs[:-1])
    if tpath in by_path:
        tids = {by_path[tpath]}
    else:
        tids = by_suffix.get("::".join(segs[1:-1]) if segs[0] in LOCAL_CRATES else tpath, set())
    types = [record[t] for t in tids if t in record and record[t]["parent"] is None
             and record[t]["kind"] in ("struct", "enum", "union", "type", "trait")]
    if len(types) != 1:
        return None
    t, member = types[0], segs[-1]
    d = t.get("details") or {}
    if f'{t["id"]}::{member}' in by_path:
        return None
    if d.get("alias_of"):
        target = d["alias_of"].split("::")[-1]
        if target in by_suffix:
            return resolve_path(f"{target}::{member}")
        return "external", f'{t["id"]} = {d["alias_of"]}'  # a method of the aliased std or third-party type
    if member in STD_TRAIT_METHODS:
        needs = STD_TRAIT_METHODS[member]
        have = set(d.get("implements_external", [])) | {x.split("::")[-1] for x in d.get("implements", [])}
        if needs is None:
            return "external", None
        if needs & have:
            return "found", f'{t["id"]} (impl {sorted(needs & have)[0]})'
        return "missing", None
    return None


def resolve_path(path, locals_=frozenset()):
    """Resolve a Rust path like `suprnova::X`, `Auth::user`, `Router` to (outcome, match)."""
    if path.lstrip().startswith("<"):  # `<T as Trait>::item`: qualified trait syntax, resolved by the trait
        return "external", None
    path = re.sub(r"::<.*?>", "", path).strip(":").replace("r#", "")
    segs = [s for s in path.split("::") if s]
    if not segs:
        return "unresolved", None
    head = segs[0]
    if head in ("crate", "self", "super") or head in locals_ or head in APP_CRATES:
        return "local", None
    if head in EXTERNAL_CRATES or head in STD_NAMES:
        return "external", None
    if path in by_path:
        return "found", by_path[path]
    if len(segs) >= 2:
        hit = type_member(segs)
        if hit:
            return hit
    for i in range(len(segs) - 1, 0, -1):  # a path through a re-exported external crate
        rec = reexports.get("::".join(segs[:i]))
        if rec:
            return "external", None
    if (len(segs) >= 2 and len({reexports[i]["details"]["target"] for i in reexport_names.get(head, ())}) == 1
            and by_suffix.get(head, set()) <= set(reexports)):
        return "external", sorted(reexport_names[head])[0]  # `Currency::USD` via `suprnova::Currency`
    if path in modules:
        return "found", f"module {path}"
    if head in LOCAL_CRATES:
        # A crate-qualified path that isn't public: try the suffix after the crate name.
        cands = by_suffix.get("::".join(segs[1:]), set())
        if len(cands) == 1:
            return "found", sorted(cands)[0]
        # The item exists, but not at this path: report where it actually is.
        for k in (2, 1):
            if len(segs) > k:
                cands = by_suffix.get("::".join(segs[-k:]), set())
                if len(cands) == 1:
                    return "wrong_path", sorted(cands)[0]
        return "missing", None
    cands = by_suffix.get(path, set())
    if cands:
        return "found", sorted(cands)[0]
    if any(m.endswith("::" + path) for m in modules):
        return "found", f"module …::{path}"
    if path in laravel_names or segs[-1] in laravel_names and len(segs) == 1:
        return "laravel_only", None
    if len(segs) >= 2 and "::".join(segs[-2:]) in laravel_names:
        return "laravel_only", None
    if len(segs) == 1:
        return "unresolved", None
    if head[:1].isupper() and head in by_suffix and len(segs) == 2 and segs[-1] in member_names:
        # `User::query`: `User` names a Suprnova type that has no `query`, but a Suprnova trait does.
        # Almost always the reader's own model sharing a framework type's name.
        return "ambiguous", sorted(member_names[segs[-1]])[0]
    if head[:1].isupper() and head not in by_suffix and len(segs) == 2:
        # `User::query`: a method on the reader's own type, typically generated by a Suprnova macro
        # or provided by a Suprnova trait. The type can't be checked; the member name can.
        member = segs[-1]
        return ("weak_found", sorted(member_names[member])[0]) if member in member_names else ("weak_missing", None)
    if head[:1].islower() and not any(k.split("::")[0] == head for k in by_path) and head not in LOCAL_CRATES:
        return "external_unverified", None
    return "missing", None


def top_level_keywords(args):
    """Leading keyword of each top-level comma-separated argument; nested
    `{...}`, `[...]`, `(...)` and string contents are skipped."""
    parts, depth, buf, quote = [], 0, [], None
    for ch in args:
        if quote:
            if ch == quote:
                quote = None
            continue
        if ch == '"':
            quote = ch
        elif ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        elif ch == "," and depth == 0:
            parts.append("".join(buf))
            buf = []
            continue
        if depth == 0 and not quote:
            buf.append(ch)
    parts.append("".join(buf))
    out = []
    for part in parts:
        m = re.match(r"\s*([a-z_]+)\s*(=|$)", part)
        if m:
            out.append(m.group(1))
    return out


def check_attribute(text, locals_=frozenset()):
    """`#[path(args)]` or `#[derive(A, B)]` -> list of (ref, outcome, match)."""
    out = []
    m = re.match(r"#!?\[\s*([A-Za-z_][\w:]*)\s*(?:\((.*)\))?\s*\]$", text.strip(), re.S)
    if not m:
        return [(text, "unresolved", None)]
    path, args = m.group(1), m.group(2) or ""
    if path == "derive":
        for d in [x.strip() for x in args.split(",") if x.strip()]:
            name = d.split("::")[-1]
            if d in STD_DERIVES or name in STD_DERIVES or d.split("::")[0] in EXTERNAL_CRATES:
                out.append((f"derive({d})", "external", None))
            elif name in proc_macros:
                out.append((f"derive({d})", "found", proc_macros[name]["id"]))
            elif name in locals_:
                out.append((f"derive({d})", "local", None))
            else:
                out.append((f"derive({d})", "missing", None))
        return out
    name = path.split("::")[-1]
    if path in STD_ATTRS or name in STD_ATTRS and "::" not in path or path.split("::")[0] in EXTERNAL_CRATES:
        return [(f"#[{path}]", "external", None)]
    if path in by_path and by_path[path] not in (r["id"] for r in proc_macros.values()):
        return [(f"#[{path}]", "found", by_path[path])]
    if name in proc_macros and (path.startswith("suprnova") or "::" not in path):
        rec = proc_macros[name]
        out.append((f"#[{path}]", "found", rec["id"]))
        # A name can be both an attribute macro and a derive's helper (`#[live]`): accept either's arguments.
        known = set(arguments.get(rec["id"], set()))
        for owner in helpers.get(name, ()):
            known |= arguments.get(owner, set())
        for kw in top_level_keywords(args):
            if known and kw not in known:
                out.append((f"#[{path}({kw})]", "missing", None))
        return out
    if name in helpers:
        out = [(f"#[{path}]", "found", sorted(helpers[name])[0])]
        known = set().union(*(arguments.get(o, set()) for o in helpers[name]))
        for kw in top_level_keywords(args):
            if known and kw not in known:
                out.append((f"#[{path}({kw})]", "missing", None))
        return out
    if name in laravel_names:
        return [(f"#[{path}]", "laravel_only", None)]
    return [(f"#[{path}]", "missing", None)]


def check_macro(name, locals_=frozenset()):
    base = name.rstrip("!").split("::")[-1]
    head = name.rstrip("!").split("::")[0]
    if base in STD_MACROS or head in EXTERNAL_CRATES:
        return "external", None
    if base in locals_:
        return "local", None
    for p in (name.rstrip("!"), f"suprnova::{base}"):
        if p in by_path:
            return "found", by_path[p]
    if base in proc_macros and proc_macros[base]["details"].get("form", "").startswith("function-like"):
        return "found", proc_macros[base]["id"]
    return "missing", None


def check_command(line):
    """A shell line -> list of (ref, outcome, match) for Suprnova binaries."""
    parts = re.split(r"\s(?:&&|\|\||;|\|)\s", line)
    if len(parts) > 1:
        return [x for p in parts for x in check_command(p)]
    out = []
    toks = line.strip().lstrip("$ ").split()
    if not toks:
        return out
    binary, rest = None, []
    if toks[0] == "suprnova" and len(toks) > 1:
        binary, rest = "suprnova", toks[1:]
    elif toks[0] in ("./app", "app"):
        binary, rest = "app", toks[1:]
    elif toks[:2] == ["cargo", "run"] and "--" in toks:
        bin_ = toks[toks.index("--bin") + 1] if "--bin" in toks else "app"
        binary, rest = ("console" if bin_ == "console" else "app"), toks[toks.index("--") + 1:]
    elif toks[0] in ("./console", "console"):
        binary, rest = "console", toks[1:]
    if not binary or not rest or rest[0].startswith("-"):
        return out
    name = rest[0]
    recs = [r for r in commands.get(name, []) if r["details"]["binary"] == binary]
    if not recs:
        other = commands.get(name)
        if other:
            out.append((f"{binary} {name}", "missing",
                        f"exists as `{other[0]['id']}` instead"))
        elif name in laravel_names:
            out.append((f"{binary} {name}", "laravel_only", None))
        else:
            out.append((f"{binary} {name}", "missing", None))
        return out
    rec = recs[0]
    out.append((f"{binary} {name}", "found", rec["id"]))
    opts = " ".join(o["spec"] for o in rec["details"]["options"])
    for flag in [t.split("=")[0] for t in rest[1:] if t.startswith("--")]:
        if flag not in opts:
            out.append((f"{binary} {name} {flag}", "missing", None))
    return out


# ---- Rust code blocks ------------------------------------------------------------------------
def node_text(src, n):
    return src[n.start_byte:n.end_byte].decode()


def check_rust_block(code, start_line):
    src = code.encode()
    tree = RUST.parse(src)
    locals_, imported, refs = set(), {}, []

    def walk(n):
        yield n
        for c in n.children:
            yield from walk(c)

    nodes = list(walk(tree.root_node))
    for n in nodes:  # definitions the snippet makes itself
        if n.type in ("struct_item", "enum_item", "trait_item", "function_item", "type_item", "mod_item",
                      "const_item", "static_item", "macro_definition", "union_item", "function_signature_item"):
            name = n.child_by_field_name("name")
            if name:
                locals_.add(node_text(src, name))
        if n.type in ("type_parameter", "lifetime_parameter"):
            locals_.add(node_text(src, n).split(":")[0].strip())
    for n in nodes:  # imports
        if n.type == "use_declaration":
            text = node_text(src, n)[len("use "):].rstrip(";").strip()
            if text.startswith("pub "):
                text = text[4:]
            for path in expand_use(text):
                last = path.split("::")[-1]
                if " as " in path:
                    path, last = path.split(" as ")
                    path, last = path.strip(), last.strip()
                if last not in ("*", "self"):
                    imported[last] = path
                outcome, match = resolve_path(path.replace("::*", "").replace("::self", ""))
                refs.append((n.start_point[0], f"use {path}", outcome, match))
    names = locals_ | set(imported)
    for n in nodes:
        line = n.start_point[0]
        if n.type in ("scoped_identifier", "scoped_type_identifier") and n.parent.type not in (
                "scoped_identifier", "scoped_type_identifier", "use_declaration", "scoped_use_list", "use_list",
                "use_as_clause", "use_wildcard"):
            if any(a.type == "use_declaration" for a in ancestors(n)):
                continue
            path = node_text(src, n)
            head = path.split("::")[0]
            if head in imported:
                path = imported[head] + path[len(head):]
            elif head in locals_:
                refs.append((line, path, "local", None))
                continue
            outcome, match = resolve_path(path, frozenset())
            refs.append((line, path, outcome, match))
        elif n.type == "macro_invocation":
            name = node_text(src, n.child_by_field_name("macro"))
            head = name.split("::")[0]
            if head in imported:
                name = imported[head] + name[len(head):]
            outcome, match = check_macro(name + "!", frozenset(locals_))
            refs.append((line, name + "!", outcome, match))
        elif n.type == "attribute_item":
            for ref, outcome, match in check_attribute(node_text(src, n), frozenset(names)):
                refs.append((line, ref, outcome, match))
        elif n.type == "type_identifier" and n.parent.type not in ("scoped_type_identifier",) and \
                not any(a.type in ("struct_item", "enum_item", "trait_item", "type_item") and
                        a.child_by_field_name("name") == n for a in ancestors(n)):
            t = node_text(src, n)
            if t in names or t in STD_NAMES:
                continue
            outcome, match = resolve_path(t)
            refs.append((line, t, outcome, match))
        elif n.type == "call_expression":
            fn = n.child_by_field_name("function")
            if fn is not None and fn.type == "field_expression":
                method = node_text(src, fn.child_by_field_name("field"))
                refs.append((line, f".{method}()", "weak_found" if method in member_names else "weak_missing",
                             sorted(member_names[method])[0] if method in member_names else None))
    return [(start_line + l, r, o, m) for l, r, o, m in refs]


def ancestors(n):
    p = n.parent
    while p is not None:
        yield p
        p = p.parent


def expand_use(text):
    """`a::{b, c::{d, e}}` -> [a::b, a::c::d, a::c::e]."""
    text = " ".join(text.split())
    m = re.match(r"^(.*?)\{(.*)\}$", text)
    if not m:
        return [text]
    prefix, inner = m.group(1), m.group(2)
    parts, depth, cur = [], 0, ""
    for ch in inner:
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
            continue
        depth += ch == "{"
        depth -= ch == "}"
        cur += ch
    parts.append(cur)
    out = []
    for p in (x.strip() for x in parts if x.strip()):
        for sub in expand_use(p):
            out.append(prefix + sub if sub != "self" else prefix.rstrip(":"))
    return out


# ---- Inline code spans --------------------------------------------------------------------------
ENV = re.compile(r"[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+")
COLON_CMD = re.compile(r"[a-z][a-z-]*(?::[a-z][a-z-]*)+")


def check_inline(s):
    s = s.strip()
    if s.startswith("#["):
        return check_attribute(s)
    if re.match(r"^[a-z_][\w:]*!(\(.*\))?$", s):
        return [(s, *check_macro(s.split("(")[0]))]
    if ENV.fullmatch(s):
        if s in envs:
            return [(s, "found", s)]
        if s.startswith("OTEL_"):  # OpenTelemetry's standard variables, read by the SDK itself
            return [(s, "external", None)]
        if s in by_suffix:  # a public constant, not an env var
            return [(s, "found", sorted(by_suffix[s])[0])]
        if s in build_time_env:
            return [(s, "external", None)]
        return [(s, "laravel_only" if s in laravel_envs else "missing", None)]
    if s in commands:
        return [(s, "found", commands[s][0]["id"])]
    if s.startswith("live:") and re.fullmatch(r"live:[a-z.-]+(\.[a-z.-]+)*", s):
        base = s.split(".")[0]
        return [(s, "found" if base in directives else "missing", base if base in directives else None)]
    if COLON_CMD.fullmatch(s):
        if s in commands:
            return [(s, "found", commands[s][0]["id"])]
        if s in laravel_names:
            return [(s, "laravel_only", None)]
        # Outside Suprnova's own command namespaces a `x:y` name is usually an example user command.
        return [(s, "missing" if s.startswith(SUPRNOVA_COMMAND_NAMESPACES) else "unresolved", None)]
    if s.startswith(("suprnova ", "./app ", "cargo run")):
        return [c for c in check_command(s)]
    if "::" in s:
        path = re.sub(r"\(.*$", "", s)
        path = re.sub(r"[?;]+$", "", path)
        if re.fullmatch(r"[A-Za-z_][\w]*(::<[^>]*>)?(::[A-Za-z_][\w]*(::<[^>]*>)?)*", path):
            return [(s, *resolve_path(path))]
        return []
    if re.fullmatch(r"\.[a-z_][a-z0-9_]*\(.*\)", s):
        method = s[1:].split("(")[0]
        return [(s, "weak_found" if method in member_names else "weak_missing",
                 sorted(member_names[method])[0] if method in member_names else None)]
    if re.fullmatch(r"[A-Z][A-Za-z0-9]+", s):
        if s in STD_NAMES:
            return [(s, "external", None)]
        return [(s, *resolve_path(s))]
    return []


# ---- Time claims (MAN-107) ----------------------------------------------------------------------
# Matched against prose only: code spans are left out, so `/v1/...` in a span is not a claim.
TIME_MARKERS = re.compile(r"\b(today|for now|currently|not yet|at the moment|follow-up|planned|known seam|"
                          r"will land|lands in|in a future|v1)\b", re.I)
SENTENCE_END = re.compile(r"(?<=[.!?])\s+(?=[A-Z`*(\[])")


def time_claims(children):
    """Sentences of one inline token whose prose carries a time marker. Each sentence comes back
    whole, code spans included, with whitespace collapsed: that text is the verdict key."""
    full, prose = [], []
    for c in children:
        if c.type == "code_inline":
            full.append(f"`{c.content}`")
            prose.append("\0" * (len(c.content) + 2))
        elif c.type in ("softbreak", "hardbreak"):
            full.append(" ")
            prose.append(" ")
        elif c.type == "text":
            full.append(c.content)
            prose.append(c.content)
    full, prose = "".join(full), "".join(prose)
    out, start = [], 0
    for m in list(SENTENCE_END.finditer(full)) + [None]:
        end = m.start() if m else len(full)
        if TIME_MARKERS.search(prose[start:end]):
            out.append(" ".join(full[start:end].split()))
        start = m.end() if m else end
    return out


# ---- Walk the manual ----------------------------------------------------------------------------
md = MarkdownIt("commonmark").enable("table")
findings = []
for f in sorted((ROOT / "manual").glob("*.md")):
    chapter = f.stem
    tokens = md.parse(f.read_text())
    heading = []
    row_subject = None  # a table row's first cell: what a claim in its other cells is about
    for i, t in enumerate(tokens):
        if t.type == "tr_open":
            row_subject = None
        if t.type == "heading_open":
            level = int(t.tag[1])
            heading = heading[:level - 1] + [tokens[i + 1].content]
        section = " > ".join(heading)
        if t.type == "fence":
            lang = (t.info.split() or [""])[0].split(",")[0]
            start = t.map[0] + 2
            if lang == "rust":
                results = check_rust_block(t.content, start)
            elif lang in ("bash", "sh", "shell", "console"):
                results = [(start + n, *c) for n, line in enumerate(t.content.splitlines())
                           for c in check_command(line)]
            elif lang in ("env", "dotenv"):
                results = []
                for n, line in enumerate(t.content.splitlines()):
                    m = re.match(r"\s*#?\s*([A-Z][A-Z0-9_]+)=", line)
                    if m and "_" in m.group(1):
                        name = m.group(1)
                        results.append((start + n, name, *((("found", name) if name in envs else
                                        ("laravel_only", None) if name in laravel_envs
                                        else ("missing", None)))))
            elif lang in ("html", "askama", "jinja", "svelte"):
                results = []
                for n, line in enumerate(t.content.splitlines()):
                    for d in re.findall(r"\b(live:[a-z-]+)", line):
                        results.append((start + n, d, *(("found", d) if d in directives else ("missing", None))))
            else:
                results = []
            for line, ref, outcome, match in results:
                findings.append({"chapter": chapter, "line": line, "section": section, "context": f"```{lang}",
                                 "ref": ref, "outcome": outcome, "match": match})
        elif t.type == "inline" and t.children:
            line = (t.map[0] + 1) if t.map else None
            in_table = any(tokens[j].type == "td_open" or tokens[j].type == "th_open" for j in range(max(0, i - 1), i))
            for c in t.children:
                if c.type == "code_inline":
                    for ref, outcome, match in check_inline(c.content):
                        findings.append({"chapter": chapter, "line": line, "section": section,
                                         "context": "table cell" if in_table else "prose",
                                         "ref": ref, "outcome": outcome, "match": match})
            if in_table and row_subject is None:
                row_subject = " ".join(t.content.split())
            for sentence in time_claims(t.children):
                if in_table and sentence != row_subject:
                    sentence = f"{row_subject} :: {sentence}"
                findings.append({"chapter": chapter, "line": line, "section": section,
                                 "context": "table cell" if in_table else "prose",
                                 "ref": sentence, "outcome": "time_claim", "match": None})

# A reference that resolves to a `#[doc(hidden)]` record exists, but is not public API by intent.
hidden_ids = {r["id"] for r in surface if r.get("hidden")}
for x in findings:
    if x["outcome"] == "found" and x["match"] in hidden_ids:
        x["outcome"] = "hidden"

OUT.write_text("".join(json.dumps(x, sort_keys=True) + "\n" for x in findings))
by_outcome = Counter(x["outcome"] for x in findings)
worst = Counter(x["chapter"] for x in findings if x["outcome"] == "missing").most_common(12)
print(json.dumps({"references": len(findings), "by_outcome": dict(by_outcome), "most_missing": worst}))
