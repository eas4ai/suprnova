"""Assemble surface.jsonl: every extracted record, filed under the manual chapter that owns it.

Reads the extractors' raw JSONL, removes `suprnova` re-exports of sibling
crates (noting each as an alias on the item it names), assigns each record
a chapter, and writes one sorted record per line. The chapter only decides
where a record is tracked; everything else in it comes from the source.

Rules are explicit and ordered: Rust items by the source file that defines
them (longest prefix wins), members by their parent, everything else by
the rule for its family. A record no rule claims is filed under a named
"no chapter" group so the gap stays visible. A rule naming a chapter that
does not exist, a duplicate id, or an unresolvable re-export stops the run.
"""
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

RAW = Path(sys.argv[1])
MANUAL = Path(sys.argv[2])
OUT = Path(sys.argv[3])
EXCL = Path(sys.argv[4])
REV = sys.argv[5]

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
    "framework/src/clock": "testing",
    "framework/src/schema/": "migrations",
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
    "framework/src/database/schema_dump": "migrations",
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
    "framework/src/error/report": "error-model",
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
    "framework/src/process/": "processes",
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
    "framework/src/redis_facade/": "redis",
    "framework/src/strings/": "strings",
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
    "InputNames": "data", "authorize": "authorization",
}
# External crates re-exported by `suprnova`.
EXTERNAL = {
    "sea_orm": "eloquent", "sea_orm_macros": "eloquent", "sea_query": "queries", "sea_query_derive": "queries",
    "validator": "validation", "validator_derive": "validation", "serde": "data", "http": "requests",
    "hyper": "lifecycle", "tokio": "lifecycle", "opendal": "filesystem", "iso_currency": "payments",
    "fake": "eloquent-factories", "dummy": "eloquent-factories", "webauthn_rs": "oauth",
    "webauthn_rs_core": "oauth", "webauthn_rs_proto": "oauth", "opentelemetry_http": "observability",
    "featureflag": "feature-flags", "secrecy": "encryption", "redis": "redis", "askama": "(no chapter) server-rendered views",
    "chrono": "(no chapter) re-exported utility crates", "chrono_tz": "(no chapter) re-exported utility crates",
    "indexmap": "(no chapter) re-exported utility crates", "async_trait": "(no chapter) re-exported utility crates",
    "strum": "(no chapter) re-exported utility crates", "magnetar": "(no chapter) Magnetar engine API",
    # Doc-hidden re-exports that generated code names, so user crates need not depend on them.
    "clap": "console", "tera": "notifications",
    "inventory": "(no chapter) doc-hidden macro support", "serde_json": "(no chapter) doc-hidden macro support",
}
SIBLINGS = ("suprnova_live", "suprnova_macros", "suprnova_web_push", "magnetar")
CLI_RULES = [  # (binary, command prefix, chapter)
    ("suprnova", "new", "cli-new"), ("suprnova", "serve", "cli-serve"), ("suprnova", "web:run", "cli-serve"),
    ("suprnova", "dev:tls", "dev-tls"), ("suprnova", "generate-types", "frontend-typescript-types"),
    ("suprnova", "make:", "cli-generators"), ("suprnova", "live:", "live"), ("suprnova", "migrate", "cli-migrations"),
    ("suprnova", "db:sync", "cli-migrations"), ("suprnova", "schema:dump", "cli-migrations"), ("suprnova", "docker:", "cli-docker"),
    ("suprnova", "schedule:", "cli-scheduling"), ("suprnova", "workflow:", "workflows"),
    ("suprnova", "ssr:", "frontend-inertia-responses"), ("suprnova", "key:generate", "encryption"),
    ("app", "serve", "deployment"), ("app", "web:run", "deployment"), ("app", "migrate", "cli-migrations"), ("app", "schema:dump", "cli-migrations"),
    ("app", "schedule:", "cli-scheduling"), ("app", "workflow:", "workflows"), ("app", "queue:", "queues"),
    ("app", "down", "deployment"), ("app", "up", "deployment"),
    ("console", "db:seed", "seeding"), ("console", "model:prune", "eloquent"),
    ("console", "db:monitor", "database"),
    ("suprnova", "db:seed", "seeding"), ("suprnova", "model:prune", "eloquent"),
    ("suprnova", "queue:", "queues"),
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


def by_path(path):
    best = max((p for p in PATH_RULES if path.startswith(p)), key=len, default=None)
    return PATH_RULES[best] if best else None


def assign(r):
    fam, kind, rid, path = r["family"], r["kind"], r["id"], r.get("file") or ""
    if fam == "cli":
        binary, name = rid.split(" ", 1)
        return next((c for b, pre, c in CLI_RULES if b == binary and name.startswith(pre)), None)
    if fam == "config":
        return "env-vars" if kind == "env" else "installation"
    if fam == "live-templates":
        return "live"
    if fam == "endpoints-tables":
        if kind == "endpoint":
            return next((c for p, c in ENDPOINT_RULES if rid.startswith(p)), None)
        return next((c for p, c in TABLE_RULES if path.startswith(p)), None)
    if kind == "reexport":
        return EXTERNAL.get(r["details"]["target"].split("::")[0])
    if kind == "proc macro":
        return MACROS.get(rid.split("::")[-1])
    if path == "framework/src/lib.rs":
        return LIB_RS.get(rid.split("::")[-1])
    if FAKE_FILES.search(path):
        return "mocking"
    return by_path(path)


records = []
for f in sorted(RAW.glob("*.jsonl")):
    records += [json.loads(l) for l in f.read_text().splitlines() if l.strip()]
raw_count = len(records)

# Every path a record is reachable by, including its defining module path,
# so a sibling re-export (which names the defining path) resolves to it.
by_any_path = {}
for r in records:
    if r["kind"] in ("reexport", "argument") or r["parent"] is not None:
        continue
    for p in [r["id"], *r.get("also", []), f'{r.get("module")}::{r["id"].split("::")[-1]}']:
        by_any_path.setdefault(p, r["id"])

aliases = defaultdict(list)
kept = []
for r in records:
    target = r["details"].get("target", "") if r["kind"] == "reexport" else ""
    if target.split("::")[0] in SIBLINGS:
        if target not in by_any_path:
            raise SystemExit(f"re-export target not found in any record: {target}")
        aliases[by_any_path[target]].append(r["id"])
        continue
    kept.append(r)

ids = defaultdict(int)
for r in kept:
    ids[r["id"]] += 1
dupes = [i for i, n in ids.items() if n > 1]
if dupes:
    raise SystemExit(f"duplicate ids across extractors: {dupes[:10]}")

# A rule that claims nothing is misleading; every path rule must match a record's file.
files = {r.get("file") or "" for r in kept}
unused = [p for p in PATH_RULES if not any(f.startswith(p) for f in files)]
if unused:
    raise SystemExit(f"path rules that match no record: {unused}")

chapter_of = {}
for r in kept:
    if r["parent"] is None:
        chapter_of[r["id"]] = assign(r) or "(no chapter) not yet classified"
out = []
for r in kept:
    r = dict(r)
    r["chapter"] = chapter_of[r["parent"]] if r["parent"] is not None else chapter_of[r["id"]]
    if r["id"] in aliases:
        r["aliases"] = sorted(aliases[r["id"]])
    out.append(r)
out.sort(key=lambda r: r["id"])
OUT.write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in out))

unfiled = [r["id"] for r in out if r["chapter"] == "(no chapter) not yet classified" and r["parent"] is None]
excluded = {"source_rev": REV, "sibling_reexports_folded_into_aliases": sum(len(v) for v in aliases.values())}
for f in sorted(RAW.glob("exclusions-*.json")):
    excluded.update(json.loads(f.read_text()))
vocab = RAW.parent / "vocab.json"
if vocab.exists():
    excluded["internal_macro_keywords"] = json.loads(vocab.read_text())["internal"]
excluded["rust_items_not_extracted"] = [
    "private, pub(crate) and #[doc(hidden)] items (rustdoc removes them before extraction)",
    "methods of external trait impls (Debug, Clone, Serialize, ...); the traits themselves are listed in each type's implements_external",
    "demo application code (app/), test-support crates and fixtures",
]
EXCL.write_text(json.dumps(excluded, indent=1, sort_keys=True) + "\n")
(OUT.parent / "meta.json").write_text(json.dumps({
    "source_rev": REV, "records": len(out),
    "source_paths": ["framework", "crates", "suprnova-cli", "suprnova-macros", "Cargo.toml"],
}, indent=1, sort_keys=True) + "\n")
print(json.dumps({"raw": raw_count, "folded_reexports": raw_count - len(out), "records": len(out),
                  "chapters": len({r["chapter"] for r in out if not r["chapter"].startswith("(")}),
                  "unclassified": unfiled[:10]}))
