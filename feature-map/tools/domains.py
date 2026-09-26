"""File every map entry under the manual chapter that owns its domain.

Reads the per-source maps the extractors produced (the raw directory), and
writes one checklist per manual chapter to `feature-map/domains/`. The
chapter only decides where an entry is filed; everything the entry says
still comes from the source.

Rules are explicit and ordered: Rust items are filed by the source file
that defines them (longest prefix wins), everything else by the rule for
its family. Entries no rule claims go to a named "no chapter" group so the
gap stays visible. A rule naming a chapter that does not exist is a hard
error.
"""
import re
import sys
from collections import defaultdict
from pathlib import Path

RAW = Path(sys.argv[1])
MANUAL = Path(sys.argv[2])
OUT = Path(sys.argv[3])
REV = sys.argv[4]

# Source-file prefix -> chapter (longest prefix wins).
PATH_RULES = {
    "framework/src/app/maintenance": "deployment",
    "framework/src/app/paths": "structure",
    "framework/src/app/": "bootstrap",
    "framework/src/boot": "bootstrap",
    "framework/src/auth/": "authentication",
    "framework/src/auth_flows/": "auth-flows",
    "framework/src/authorization/": "authorization",
    "framework/src/rbac/": "authorization",
    "framework/src/broadcasting/": "broadcasting",
    "framework/src/bus/": "bus",
    "framework/src/cache/": "cache",
    "framework/src/redis_retry": "cache",
    "framework/src/config/": "configuration",
    "framework/src/console/": "console",
    "framework/src/container/": "container",
    "framework/src/context/": "context",
    "framework/src/cors/": "cors",
    "framework/src/crypto/": "encryption",
    "framework/src/csrf/": "csrf",
    "framework/src/data/": "data",
    "framework/src/database/testing": "database-testing",
    "framework/src/database/route_binding": "routing",
    "framework/src/database/": "database",
    "framework/src/eloquent/builder": "queries",
    "framework/src/eloquent/scopes": "queries",
    "framework/src/eloquent/lazy": "queries",
    "framework/src/eloquent/relations/": "eloquent-relationships",
    "framework/src/eloquent/collection": "eloquent-collections",
    "framework/src/eloquent/casts/": "eloquent-mutators",
    "framework/src/eloquent/attrs": "eloquent",
    "framework/src/eloquent/": "eloquent",
    "framework/src/error.rs": "errors",
    "framework/src/http/abort": "errors",
    "framework/src/events/": "events",
    "framework/src/factory/": "eloquent-factories",
    "framework/src/features/": "feature-flags",
    "framework/src/filesystem/": "filesystem",
    "framework/src/hashing/": "hashing",
    "framework/src/http/body": "responses",
    "framework/src/http/response": "responses",
    "framework/src/http/cookie": "responses",
    "framework/src/http/form_request": "validation",
    "framework/src/http/": "requests",
    "framework/src/http_client/": "http-client",
    "framework/src/idempotency/": "idempotency",
    "framework/src/inertia/": "frontend-inertia-responses",
    "framework/src/live/": "live",
    "framework/src/localization/": "localization",
    "framework/src/logging/": "logging",
    "framework/src/magnetar_integration/oauth": "oauth",
    "framework/src/magnetar_integration/passkey": "oauth",
    "framework/src/magnetar_integration/magic_link": "oauth",
    "framework/src/magnetar_integration/": "authentication",
    "framework/src/mail/": "mail",
    "framework/src/media/": "images",
    "framework/src/middleware/": "middleware",
    "framework/src/notifications/channels/webpush": "web-push",
    "framework/src/notifications/": "notifications",
    "framework/src/pagination/": "pagination",
    "framework/src/payments/traits/": "payments-provider-guide",
    "framework/src/payments/mock": "payments-provider-guide",
    "framework/src/payments/": "payments",
    "framework/src/queue/": "queues",
    "framework/src/rate_limit/": "rate-limiting",
    "framework/src/render_cache/providers/": "render-cache-deployment",
    "framework/src/render_cache/ledger": "render-cache-deployment",
    "framework/src/render_cache/migration": "render-cache-deployment",
    "framework/src/render_cache/file_store": "render-cache-deployment",
    "framework/src/render_cache/telemetry": "render-cache-operations",
    "framework/src/render_cache/console": "render-cache-operations",
    "framework/src/render_cache/collector": "render-cache-generations",
    "framework/src/render_cache/orm": "render-cache-generations",
    "framework/src/render_cache/write_side": "render-cache-generations",
    "framework/src/render_cache/": "render-cache",
    "framework/src/resources/": "eloquent-resources",
    "framework/src/routing/signed": "urls",
    "framework/src/routing/url": "urls",
    "framework/src/routing/resource": "controllers",
    "framework/src/routing/": "routing",
    "framework/src/static_files": "routing",
    "framework/src/schedule/": "scheduling",
    "framework/src/seed/": "seeding",
    "framework/src/server.rs": "lifecycle",
    "framework/src/session/": "session",
    "framework/src/sse/": "sse",
    "framework/src/supervisor/": "supervisors",
    "framework/src/telemetry/": "observability",
    "framework/src/testing/response": "http-tests",
    "framework/src/testing/inertia": "http-tests",
    "framework/src/testing/": "testing",
    "framework/src/timeout/": "timeout",
    "framework/src/validation/": "validation",
    "framework/src/vector/": "vector",
    "framework/src/web_push": "web-push",
    "framework/src/workflow/": "workflows",
    "framework/src/ws/": "websockets",
    "framework/src/view/": "(no chapter) server-rendered views",
    "framework/src/content/": "(no chapter) Markdown content and docs pipeline",
    "crates/suprnova-live/src/render_cache/": "render-cache",
    "crates/suprnova-live/": "live",
    "crates/suprnova-magnetar/src/oauth": "oauth",
    "crates/suprnova-magnetar/src/plugins/oauth": "oauth",
    "crates/suprnova-magnetar/src/passkey": "oauth",
    "crates/suprnova-magnetar/": "(no chapter) Magnetar engine API",
    "crates/suprnova-payments-stripe/": "payments-stripe",
    "crates/suprnova-payments-paddle/": "payments-paddle",
    "crates/suprnova-payments-nowpayments/": "payments-nowpayments",
    "crates/suprnova-web-push/": "web-push",
}
# Items defined in framework/src/lib.rs itself, by name.
LIB_RS = {
    "json_response": "responses", "text_response": "responses", "expect": "testing",
    "global_middleware": "middleware", "VERSION": "(no chapter) crate metadata",
}
# Test fakes live beside their subsystem; the manual documents fakes in mocking.md.
FAKE_FILES = re.compile(r'framework/src/(queue|events|bus|notifications|filesystem|container|broadcasting)/testing\.rs|framework/src/http_client/fake')
# Proc-macros by name.
MACROS = {
    "model": "eloquent", "observer": "eloquent", "scopes": "queries", "accessor": "eloquent-mutators",
    "mutator": "eloquent-mutators", "prunable": "eloquent", "LiveComponent": "live", "live": "live",
    "Data": "data", "FormRequest": "validation", "request": "validation", "MultipartRequest": "requests",
    "NotificationMailable": "notifications", "Command": "console", "command": "console",
    "Factory": "eloquent-factories", "domain_error": "error-model", "handler": "controllers",
    "injectable": "container", "service": "container", "main": "bootstrap", "policy": "authorization",
    "suprnova_test": "testing", "describe": "testing", "test": "testing",
    "view": "(no chapter) server-rendered views", "view_filter": "(no chapter) server-rendered views",
    "workflow": "workflows", "workflow_step": "workflows", "inertia_response": "frontend-inertia-responses",
    "InertiaProps": "frontend-typescript-types", "redirect": "responses",
}
# External crates re-exported by `suprnova`.
EXTERNAL = {
    "sea_orm": "eloquent", "sea_orm_macros": "eloquent", "sea_query": "queries", "sea_query_derive": "queries",
    "validator": "validation", "validator_derive": "validation", "serde": "data", "http": "requests",
    "hyper": "lifecycle", "tokio": "lifecycle", "opendal": "filesystem", "iso_currency": "payments",
    "fake": "eloquent-factories", "dummy": "eloquent-factories", "async_trait": "(no chapter) re-exported utility crates", "webauthn_rs": "oauth",
    "webauthn_rs_core": "oauth", "webauthn_rs_proto": "oauth", "opentelemetry_http": "observability",
    "featureflag": "feature-flags", "secrecy": "encryption", "askama": "(no chapter) server-rendered views",
    "chrono": "(no chapter) re-exported utility crates", "chrono_tz": "(no chapter) re-exported utility crates",
    "indexmap": "(no chapter) re-exported utility crates", "async_trait": "(no chapter) re-exported utility crates",
    "strum": "(no chapter) re-exported utility crates", "magnetar": "(no chapter) Magnetar engine API",
}
SIBLINGS = ("suprnova_live", "suprnova_macros", "suprnova_web_push", "magnetar")
CLI_RULES = [  # (binary, command prefix, chapter)
    ("suprnova", "new", "cli-new"), ("suprnova", "serve", "cli-serve"), ("suprnova", "web:run", "cli-serve"),
    ("suprnova", "dev:tls", "dev-tls"), ("suprnova", "generate-types", "frontend-typescript-types"),
    ("suprnova", "make:", "cli-generators"), ("suprnova", "live:", "live"), ("suprnova", "migrate", "cli-migrations"),
    ("suprnova", "db:sync", "cli-migrations"), ("suprnova", "docker:", "cli-docker"),
    ("suprnova", "schedule:", "cli-scheduling"), ("suprnova", "workflow:", "workflows"),
    ("suprnova", "ssr:", "frontend-inertia-responses"), ("suprnova", "key:generate", "encryption"),
    ("app", "serve", "deployment"), ("app", "web:run", "deployment"), ("app", "migrate", "cli-migrations"),
    ("app", "schedule:", "cli-scheduling"), ("app", "workflow:", "workflows"), ("app", "queue:", "queues"),
    ("app", "down", "deployment"), ("app", "up", "deployment"),
    ("console", "db:seed", "seeding"), ("console", "model:prune", "eloquent"),
    ("console", "render-cache:", "render-cache-operations"), ("console", "__suprnova:live-tool", "live"),
]
ENDPOINT_RULES = [("/_suprnova/health", "deployment"), ("/_suprnova/lang", "localization"),
                  ("/__live", "live"), ("/suprnova-ui", "live"), ("/webhooks/payments", "payments")]
TABLE_RULES = [  # (source-path prefix, chapter)
    ("framework/src/rbac/", "authorization"), ("framework/src/features/", "feature-flags"),
    ("framework/src/workflow/", "workflows"), ("framework/src/auth_flows/", "auth-flows"),
    ("framework/src/payments/", "payments"), ("framework/src/render_cache/", "render-cache-deployment"),
    ("crates/suprnova-magnetar/", "auth-flows"), ("suprnova-cli/src/templates/", "cli-new"),
    ("framework/src/queue/", "queues"),
]

chapters = {p.stem for p in MANUAL.glob("*.md")} - {"README", "documentation"}
for rules in (PATH_RULES.values(), LIB_RS.values(), MACROS.values(), EXTERNAL.values(),
              [c for *_, c in CLI_RULES], [c for _, c in ENDPOINT_RULES], [c for _, c in TABLE_RULES]):
    for c in rules:
        if not c.startswith("(no chapter)") and c not in chapters:
            raise SystemExit(f"rule names a chapter that does not exist: {c}")

TOP = re.compile(r'^- \[([ x])\] (?:([a-z ]+?) )?`([^`]+)`(.*)$')
AT = re.compile(r' · ([^ ]+?):(\d+)')


def entries(md):
    """(section heading, [lines]) for each top-level entry in a raw map file."""
    section, cur, out = "", None, []
    for line in md.read_text().splitlines():
        if line.startswith("### ") or line.startswith("## "):
            section = line.lstrip("#").strip()
            cur = None
            continue
        if line.startswith("- ["):
            cur = [line]
            out.append((section, cur))
        elif cur is not None and line.startswith("  "):
            cur.append(line)
        else:
            cur = None
    return out


def by_path(path):
    best = max((p for p in PATH_RULES if path.startswith(p)), key=len, default=None)
    return PATH_RULES[best] if best else None


def assign(source, section, lines):
    m = TOP.match(lines[0])
    kind, ident, rest = (m.group(2) or ""), m.group(3), m.group(4)
    at = AT.search(rest)
    path = at.group(1) if at else ""
    if source == "cli.md":
        binary, name = ident.split(" ", 1)
        for b, pre, ch in CLI_RULES:
            if b == binary and name.startswith(pre):
                return ch
    elif source == "configuration.md":
        return "env-vars" if kind == "env" else "installation"
    elif source == "live-templates.md":
        return "live"
    elif source == "endpoints-and-tables.md":
        if kind == "endpoint":
            return next((c for p, c in ENDPOINT_RULES if ident.startswith(p)), None)
        return next((c for p, c in TABLE_RULES if path.startswith(p)), None)
    elif "re-exports" in rest:
        target = re.search(r're-exports `([a-z_0-9]+)', rest).group(1)
        return EXTERNAL.get(target)
    elif kind.startswith("proc"):
        return MACROS.get(ident.split("::")[-1])
    if path == "framework/src/lib.rs":
        return LIB_RS.get(ident.split("::")[-1])
    if FAKE_FILES.search(path):
        return "mocking"
    return by_path(path)


# Aliases: `suprnova::X` re-exporting a sibling crate's item is noted on that item.
aliases = defaultdict(list)
raw_files = sorted(RAW.glob("*.md"))
collected = []
for f in raw_files:
    for section, lines in entries(f):
        m = TOP.match(lines[0])
        if not m:
            continue
        rest = m.group(4)
        sib = re.search(r're-exports `((?:' + "|".join(SIBLINGS) + r')::[^`]+)`', rest)
        if sib:
            aliases[sib.group(1)].append(m.group(3))
            continue
        collected.append((f.name, section, lines))

checked = set()
parent = None
for f in OUT.glob("*.md"):
    for line in f.read_text().splitlines():
        top = re.match(r'- \[[ x]\] (?:[a-z ]+? )?`([^`]+)`', line)
        if top:
            parent = top.group(1)
        arg = re.match(r'\s+- \[x\] argument (.+) · ', line)
        if arg and parent:
            checked.add(f"{parent} {arg.group(1)}")
            continue
        m = re.match(r'\s*- \[x\] (?:[a-z ]+? )?`([^`]+)`', line)
        if m:
            checked.add(m.group(1))

# Resolve each re-export target through every path its item is reachable by.
by_any_path = {}
for _, section, lines in collected:
    m = TOP.match(lines[0])
    by_any_path[m.group(3)] = m.group(3)
    home = re.match(r'`([^`]+)`', section)
    if home:  # the defining module, which is the path a re-export names
        by_any_path.setdefault(f"{home.group(1)}::{m.group(3).split('::')[-1]}", m.group(3))
    also = re.search(r'also ((?:`[^`]+`(?:, )?)+)', m.group(4))
    if also:
        for p in re.findall(r'`([^`]+)`', also.group(1)):
            by_any_path.setdefault(p, m.group(3))
resolved = defaultdict(list)
for target, names in aliases.items():
    if target not in by_any_path:
        raise SystemExit(f"re-export target not found in any map: {target}")
    resolved[by_any_path[target]].extend(names)
aliases = resolved

domains = defaultdict(lambda: defaultdict(list))
unfiled = []
for source, section, lines in collected:
    ch = assign(source, section, lines)
    if ch is None:
        unfiled.append((source, lines[0]))
        ch = "(no chapter) not yet classified"
    m = TOP.match(lines[0])
    ident = m.group(3)
    first = lines[0]
    if ident in aliases:
        alias = ", ".join(f"`{a}`" for a in sorted(aliases[ident]))
        first = first + f" (re-exported as {alias})"
    out_lines = [first] + lines[1:]
    # Reapply checked state by identity: arguments by "parent syntax", others by their path.
    fixed = []
    for line in out_lines:
        arg = re.match(r'(\s+- \[)[ x](\] argument (.+) · .*)', line)
        item = re.match(r'(\s*- \[)[ x](\] (?:[a-z ]+? )?`([^`]+)`.*)', line)
        if arg:
            line = arg.group(1) + ("x" if f"{ident} {arg.group(3)}" in checked else " ") + arg.group(2)
        elif item:
            line = item.group(1) + ("x" if item.group(3) in checked else " ") + item.group(2)
        fixed.append(line)
    label = {"cli.md": "Command line", "configuration.md": "Configuration",
             "live-templates.md": "Live templates", "endpoints-and-tables.md": "Endpoints and tables"}.get(
        source, f"Rust API: {source[:-3]}")
    domains[ch][(label, section)].append(fixed)

OUT.mkdir(parents=True, exist_ok=True)
for old in OUT.glob("*.md"):
    old.unlink()


def slug(ch):
    return re.sub(r'[^a-z0-9]+', '-', ch.lower()).strip('-')


rows = []
for ch in sorted(domains, key=lambda c: (c.startswith("(no chapter)"), c)):
    groups = domains[ch]
    boxes = sum(sum(1 for e in es for l in e if re.match(r'\s*- \[[ x]\]', l)) for es in groups.values())
    done = sum(sum(1 for e in es for l in e if re.match(r'\s*- \[x\]', l)) for es in groups.values())
    fname = f"{ch}.md" if not ch.startswith("(no chapter)") else f"_{slug(ch)}.md"
    title = f"`manual/{ch}.md`" if not ch.startswith("(no chapter)") else ch
    L = [f"# Feature map: {title}", "",
         f"Source at {REV}. Every entry below is extracted from the code; this file only groups "
         "them under the chapter that owns their domain. A checked box means the chapter's "
         "documentation of that item has been remediated against the source.", "",
         f"{done} of {boxes} checked.", ""]
    cur_label = None
    for (label, section) in sorted(groups):
        if label != cur_label:
            L += [f"## {label}", ""]
            cur_label = label
        if section:
            L += [f"### {section}", ""]
        for e in groups[(label, section)]:
            L += e
        L.append("")
    (OUT / fname).write_text("\n".join(L).rstrip() + "\n")
    rows.append((ch, fname, boxes, done))

empty = sorted(chapters - set(domains))
L = ["# Feature map by manual chapter", "",
     f"Source at {REV}. {sum(r[2] for r in rows)} items across {sum(1 for r in rows if not r[0].startswith('('))} "
     f"chapters, plus the groups no chapter covers yet.", "",
     "| Chapter | Items | Checked |", "|---|---|---|"]
for ch, fname, boxes, done in rows:
    name = f"[`{ch}`](domains/{fname})" if not ch.startswith("(") else f"[{ch}](domains/{fname})"
    L.append(f"| {name} | {boxes} | {done} |")
L += ["", f"Chapters with no extracted surface ({len(empty)}), narrative or reference pages whose claims "
      "are checked against the items filed elsewhere: " + ", ".join(f"`{c}`" for c in empty), ""]
(OUT.parent / "INDEX.md").write_text("\n".join(L) + "\n")
print(f"{sum(r[2] for r in rows)} items, {len(rows)} domain files, {len(unfiled)} unclassified, "
      f"{len(checked)} checks preserved")
for src, line in unfiled[:20]:
    print("  unclassified:", src, line[:120])

# Exclusions stay visible: carry every "not listed, and why" note forward.
X = ["# Feature map exclusions", "",
     f"Source at {REV}. What the extractors saw and deliberately did not list, with the reason.", ""]
conf = (RAW / "configuration.md").read_text()
if "### Not runtime configuration" in conf:
    block = conf.split("### Not runtime configuration", 1)[1].split("\n## ", 1)[0]
    X += ["## Environment-like names that are not runtime configuration", "", block.strip(), ""]
cli = (RAW / "cli.md").read_text()
notes = [l for l in cli.splitlines() if l.startswith(("Excluded as", "Declared in source"))]
if notes:
    X += ["## Commands", ""] + notes + [""]
vocab_path = RAW.parent / "vocab.json"
if vocab_path.exists():
    import json
    internal = json.loads(vocab_path.read_text())["internal"]
    X += ["## Macro parser keywords that are internal", ""]
    X += [f"- `{k}`: {v}" for k, v in internal.items()] + [""]
X += ["## Rust items", "",
      "- Private, `pub(crate)` and `#[doc(hidden)]` items: rustdoc removes them before extraction.",
      "- Implementations of external traits (`Debug`, `Clone`, `Serialize`, ...).",
      "- Demo application code (`app/`), test-support crates and fixtures.",
      f"- Re-exports of sibling Suprnova crates ({sum(len(v) for v in aliases.values())}): each is noted as "
      "\"re-exported as\" on the item it names instead of being listed twice.", ""]
(OUT.parent / "EXCLUSIONS.md").write_text("\n".join(X) + "\n")

