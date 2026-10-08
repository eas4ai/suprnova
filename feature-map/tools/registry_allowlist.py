"""Generate the component scan's allowlist from the feature map.

A third-party Live component may name Suprnova's documented public API and
nothing else (REG-006, REG-030). This tool reads `feature-map/surface.jsonl`
and writes one JSON line per admitted path to
`suprnova-cli/src/registry/scan/allowlist.jsonl`, which the CLI embeds.

Every `rust-api` record of the `suprnova` crate gives one path (`id`) and
its aliases (`also`). A record of a sibling crate counts only through the
`suprnova::` paths it lists under `aliases`: a re-export is Suprnova API
through `suprnova::` only. A method takes its parent's paths. Each item
carries the capability its chapter and module decide, from the tables
below; an item those tables cannot decide is left out, so the scan refuses
it: the file still lists it, marked `refused`, so the scan can tell a
refused method from a name Suprnova does not define. A re-exported crate is
admitted as a prefix: every path under it carries the capability of the
longest rule that covers it, and a path no rule covers is refused.

Usage:
  registry_allowlist.py            write the data file
  registry_allowlist.py --stdout   print it instead
  registry_allowlist.py --check    exit 1 when the data file is out of date
"""
import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SURFACE = ROOT / "feature-map" / "surface.jsonl"
OUTPUT = ROOT / "suprnova-cli" / "src" / "registry" / "scan" / "allowlist.jsonl"

# The capability a chapter carries. `None` admits the chapter with no
# capability; a chapter missing from this table is refused, as is one
# mapped to REFUSE, so an undecided chapter can never be admitted.
REFUSE = "refuse"
CHAPTERS = {
    "(no chapter) Markdown content and docs pipeline": None,
    "(no chapter) crate metadata": None,
    "(no chapter) doc-hidden macro support": REFUSE,
    "(no chapter) re-exported utility crates": None,
    "(no chapter) server-rendered views": None,
    "auth-flows": REFUSE,
    "authentication": "session",
    "authorization": None,
    "bootstrap": REFUSE,
    "broadcasting": "network",
    "bus": "queue",
    "cache": "cache",
    "configuration": "environment",
    "console": REFUSE,
    "container": None,
    "context": None,
    "controllers": None,
    "cors": None,
    "csrf": "session",
    "data": None,
    "database": "database",
    "database-testing": REFUSE,
    "deployment": "files",
    "eloquent": "database",
    "eloquent-collections": "database",
    "eloquent-factories": "database",
    "eloquent-mutators": "database",
    "eloquent-relationships": "database",
    "eloquent-resources": "database",
    "encryption": "environment",
    "error-model": None,
    "errors": None,
    "events": None,
    "feature-flags": "database",
    "filesystem": "files",
    "frontend-inertia-responses": None,
    "hashing": None,
    "http-client": "network",
    "http-tests": REFUSE,
    "idempotency": "cache",
    "images": "files",
    "lifecycle": "network",
    "live": None,
    "localization": None,
    "logging": "files",
    "mail": "mail",
    "middleware": None,
    "migrations": "database",
    "mocking": REFUSE,
    "notifications": REFUSE,
    "oauth": "network",
    "observability": "network",
    "pagination": "database",
    "payments": "network",
    "payments-provider-guide": "network",
    "processes": "process",
    "queries": "database",
    "queues": "queue",
    "rate-limiting": "cache",
    "redis": "database",
    "render-cache": "cache",
    "render-cache-deployment": "cache",
    "render-cache-generations": "cache",
    "render-cache-operations": "cache",
    "requests": None,
    "responses": None,
    "routing": None,
    "scheduling": "process",
    "seeding": "database",
    "session": "session",
    "sse": "network",
    "strings": None,
    "structure": "environment",
    "supervisors": "process",
    "testing": None,
    "timeout": None,
    "urls": None,
    "validation": None,
    "vector": "database",
    "web-push": "network",
    "websockets": "network",
    "workflows": "queue",
}

# Module rules, by the item's module path: the longest prefix that covers
# the module wins over the chapter.
MODULES = {
    "suprnova::auth::database_provider": "database",
    "suprnova::auth::eloquent_provider": "database",
    "suprnova::auth::remember": "database",
    "suprnova::auth::token_guard": "database",
    "suprnova::magnetar_integration": REFUSE,
    "suprnova::magnetar_integration::oauth": "network",
    "suprnova::magnetar_integration::oauth_transport": "network",
    "suprnova::rbac": "database",
    "suprnova::content::docs": "files",
    "suprnova::profiling": None,
    "suprnova::events::queued_listener": "queue",
    "suprnova::features::evaluators::cached": "cache",
    "suprnova::inertia::ssr": "network",
    "suprnova::inertia::manifest": "files",
    "suprnova::inertia::encrypt_middleware": "environment",
    "suprnova::hashing::config": "environment",
    "suprnova::media::magick": "process",
    "suprnova::live::session_state": "session",
    "suprnova::live::streams": "network",
    "suprnova::live::upload_host": "files",
    "suprnova::live::assets": "files",
    "suprnova::live::ui_assets": "files",
    "suprnova::live::tooling": REFUSE,
    "suprnova::live::__private": REFUSE,
    "suprnova::localization::fluent": "files",
    "suprnova::localization::config": "environment",
    "suprnova::payments::dto": None,
    "suprnova::payments::money": None,
    "suprnova::payments::error": None,
    "suprnova::payments::entities": "database",
    "suprnova::payments::migrations": "database",
    "suprnova::payments::mock": REFUSE,
    "suprnova::http::upload": "files",
    "suprnova::http::file_response": "files",
    "suprnova::http::cookie": "session",
    "suprnova::database::route_binding": "database",
    "suprnova::static_files": "files",
    "suprnova::routing::signed": "environment",
    "suprnova::validation::rule::async_rules": "database",
    "suprnova::vector::pinecone": "network",
    "suprnova::vector::qdrant": "network",
    "suprnova::workflow::entities": "database",
    "suprnova::workflow::migrations": "database",
    "suprnova::workflow::store": "database",
    "suprnova::server": "network",
    # Test infrastructure swaps the application's own services or clock;
    # a component never ships it.
    "suprnova::live::testing": REFUSE,
    "suprnova::render_cache::testing": REFUSE,
    "suprnova::crypto::testing": REFUSE,
    "suprnova::database::testing": REFUSE,
}

# Item rules, by full path, for items whose own effect differs from their
# module's or their type's: writes into the container or the application's
# global middleware and shared props, which are refused, and methods that
# reach an effect on a type whose other members do not (a redirect that
# writes the session, a response that opens a file, a URL helper that signs
# with the application key). A method takes its module from its type, not
# from the file that holds its impl block, so such a method needs a rule
# here; a rule on a type covers every member under it.
ITEMS = {
    "suprnova::App::bind": REFUSE,
    "suprnova::App::bind_factory": REFUSE,
    "suprnova::App::bind_if_absent": REFUSE,
    "suprnova::App::bind_scoped": REFUSE,
    "suprnova::App::boot_services": REFUSE,
    "suprnova::App::clear_history": REFUSE,
    "suprnova::App::disable_ssr_for_request": REFUSE,
    "suprnova::App::factory": REFUSE,
    "suprnova::App::flash": "session",
    "suprnova::App::flush_inertia_shared": REFUSE,
    "suprnova::App::inertia_share": REFUSE,
    "suprnova::App::inertia_share_lazy": REFUSE,
    "suprnova::App::inertia_share_once": REFUSE,
    "suprnova::App::init": REFUSE,
    "suprnova::App::instance": REFUSE,
    "suprnova::App::register_inertia_shared": REFUSE,
    "suprnova::App::run_scoped": REFUSE,
    "suprnova::App::scoped": REFUSE,
    "suprnova::App::singleton": REFUSE,
    "suprnova::App::singleton_if_absent": REFUSE,
    "suprnova::App::singleton_if_absent_with": REFUSE,
    "suprnova::App::spawn_scoped": REFUSE,
    "suprnova::Container::bind": REFUSE,
    "suprnova::Container::bind_factory": REFUSE,
    "suprnova::Container::bind_if_absent": REFUSE,
    "suprnova::Container::bind_scoped": REFUSE,
    "suprnova::Container::factory": REFUSE,
    "suprnova::Container::scoped": REFUSE,
    "suprnova::Container::singleton": REFUSE,
    "suprnova::Container::singleton_if_absent": REFUSE,
    "suprnova::container::provider::bootstrap": REFUSE,
    "suprnova::container::provider::register_service_bindings": REFUSE,
    "suprnova::container::provider::register_singletons": REFUSE,
    "suprnova::Request::auth_user_id": "session",
    "suprnova::Request::with_auth_user_id": "session",
    "suprnova::Request::cookie": "session",
    "suprnova::Request::cookies": "session",
    "suprnova::Request::live_tenant": "session",
    "suprnova::clock::TestClock": REFUSE,
    "suprnova::clock::TestClockGuard": REFUSE,
    "suprnova::clock::TestClockHandle": REFUSE,
    # Global registration: rewrites the application for every request,
    # as a container binding does.
    "suprnova::register_global_middleware": REFUSE,
    "suprnova::prepend_global_middleware": REFUSE,
    "suprnova::register_middleware_group": REFUSE,
    "suprnova::register_middleware_alias": REFUSE,
    "suprnova::middleware::register_middleware_alias_with_args": REFUSE,
    "suprnova::clear_middleware_alias": REFUSE,
    "suprnova::clear_middleware_group": REFUSE,
    "suprnova::middleware::clear_middleware_priority_for_test": REFUSE,
    "suprnova::append_middleware_priority": REFUSE,
    "suprnova::prepend_middleware_priority": REFUSE,
    "suprnova::register_terminable": REFUSE,
    "suprnova::data::registry::register": REFUSE,
    "suprnova::InertiaRegistry::share_lazy": REFUSE,
    "suprnova::InertiaRegistry::share_once": REFUSE,
    "suprnova::InertiaRegistry::share_value": REFUSE,
    # Reads its settings from the environment when built by `default`, which
    # has no path of its own.
    "suprnova::InertiaConfig": "environment",
    # Reaches session on its own path.
    "suprnova::Redirect::back": "session",
    "suprnova::Redirect::refresh": "session",
    "suprnova::Redirect::guest": "session",
    "suprnova::Redirect::intended": "session",
    "suprnova::Redirect::set_intended_url": "session",
    "suprnova::Redirect::with": "session",
    "suprnova::Redirect::with_input": "session",
    "suprnova::Redirect::with_errors": "session",
    "suprnova::Redirect::with_errors_bag": "session",
    "suprnova::Redirect::preserve_fragment": "session",
    "suprnova::Redirect::without_cookie": "session",
    "suprnova::Redirect::without_cookies": "session",
    "suprnova::RedirectRouteBuilder::flash": "session",
    "suprnova::RedirectRouteBuilder::with_input": "session",
    "suprnova::RedirectRouteBuilder::with_errors": "session",
    "suprnova::RedirectRouteBuilder::with_errors_bag": "session",
    "suprnova::RedirectRouteBuilder::preserve_fragment": "session",
    "suprnova::RedirectRouteBuilder::without_cookie": "session",
    "suprnova::RedirectRouteBuilder::without_cookies": "session",
    "suprnova::HttpResponse::without_cookie": "session",
    "suprnova::HttpResponse::without_cookies": "session",
    "suprnova::ResponseExt::without_cookie": "session",
    "suprnova::ResponseExt::without_cookies": "session",
    "suprnova::url::previous": "session",
    "suprnova::InertiaResponse::resolve": "session",
    "suprnova::InertiaResponse::try_with": "session",
    "suprnova::InertiaResponse::try_always": "session",
    "suprnova::InertiaResponse::try_flash": "session",
    "suprnova::InertiaResponse::try_merge_with": "session",
    "suprnova::InertiaResponse::try_scroll": "session",
    "suprnova::InertiaResponse::try_scroll_wrapped": "session",
    "suprnova::InertiaVersionMiddleware": "session",
    "suprnova::LocaleMiddleware": "session",
    # Reaches environment on its own path.
    "suprnova::LocaleMiddleware::from_env": "environment",
    # Reaches session on its own path.
    "suprnova::live::action::flash_intent": "session",
    "suprnova::live::action::FlashIntent": "session",
    "suprnova::live::LiveDocument::mount": "session",
    "suprnova::live::current_tenant": "session",
    "suprnova::authorization::__authorize_handler": "session",
    "suprnova::authorization::__authorize_handler_type": "session",
    # Reaches environment on its own path.
    "suprnova::EventDispatcher::new": "environment",
    "suprnova::InertiaConfig::new": "environment",
    "suprnova::hashing::default_driver": "environment",
    "suprnova::hash": "environment",
    "suprnova::verify": "environment",
    "suprnova::needs_rehash": "environment",
    "suprnova::hashing::hash_async": "environment",
    "suprnova::hashing::verify_async": "environment",
    "suprnova::url::signed_route": "environment",
    "suprnova::url::temporary_signed_route": "environment",
    "suprnova::url::signed_url": "environment",
    "suprnova::url::has_valid_signature": "environment",
    "suprnova::url::signature_has_not_expired": "environment",
    "suprnova::url::signature_verdict": "environment",
    "suprnova::Redirect::signed_route": "environment",
    "suprnova::Redirect::temporary_signed_route": "environment",
    "suprnova::FrameworkError::into_json_api_response": "environment",
    "suprnova::live::production_ledger_limits": "environment",
    # Reaches files on its own path.
    "suprnova::HttpResponse::file": "files",
    "suprnova::HttpResponse::download": "files",
    "suprnova::InertiaConfig::vite_manifest": "files",
    "suprnova::VersionResolver::resolve": "files",
    "suprnova::Inertia::install": "files",
    "suprnova::Router::try_live_ui_assets": "files",
    "suprnova::Router::try_live_ui_assets_from": "files",
    "suprnova::Router::try_live_ui_assets_for": "files",
    "suprnova::Router::try_live_ui_assets_for_from": "files",
    "suprnova::Lang::reload": "files",
    "suprnova::Translator::reload": "files",
    "suprnova::Translator::reload_if_stale": "files",
    "suprnova::profiling::start": "files",
    "suprnova::profiling::HeapProfile": "files",
    # Reaches network on its own path.
    "suprnova::Password::uncompromised": "network",
    "suprnova::Password::uncompromised_with_threshold": "network",
    "suprnova::HibpVerifier": "network",
    # Reaches database on its own path.
    "suprnova::live::verify_ledger_backend": "database",
    "suprnova::live::verify_ledger_driver_for_test": "database",
}

# Item names whose effect is the same wherever they sit: a constructor that
# reads its settings from the environment reaches the environment, whatever
# its chapter's capability.
NAMES = {
    "from_env": "environment",
    "try_from_env": "environment",
    "from_env_prefix": "environment",
    "detect_from_env": "environment",
}

# The crates Suprnova re-exports, by the `suprnova::` path that re-exports
# them. Each rule covers the path and everything under it; the longest rule
# wins, and a path under a crate no rule covers is refused. A crate whose
# own path maps to REFUSE admits only the sub-paths listed.
CRATES = {
    "suprnova::tokio": REFUSE,
    "suprnova::tokio::fs": "files",
    "suprnova::tokio::io": "files",
    "suprnova::tokio::net": "network",
    "suprnova::tokio::process": "process",
    "suprnova::tokio::signal": "process",
    "suprnova::tokio::time": None,
    "suprnova::tokio::sync": None,
    "suprnova::tokio::task": None,
    "suprnova::tokio::spawn": None,
    "suprnova::hyper": "network",
    "suprnova::opendal": "files",
    "suprnova::redis": "database",
    "suprnova::sea_orm": "database",
    "suprnova::database::sea_orm": "database",
    "suprnova::sea_query": "database",
    "suprnova::serde": None,
    "suprnova::validator": None,
    "suprnova::fake": None,
    "suprnova::chrono": None,
    "suprnova::chrono::Local": "environment",
    "suprnova::chrono::offset::Local": "environment",
    "suprnova::chrono_tz": None,
    "suprnova::indexmap": None,
}

# Capabilities the CLI knows (registry::Capability); anything else is a bug
# in the tables above.
CAPABILITIES = {"database", "network", "files", "mail", "queue", "cache", "session", "environment", "process"}


def strip_suffix(path):
    """`suprnova::Iden#2` names the second item at `suprnova::Iden`."""
    return path.split("#", 1)[0]


def longest_rule(table, path):
    """The rule of the longest key that is `path` or one of its ancestors."""
    best = None
    for key, value in table.items():
        if path == key or path.startswith(key + "::"):
            if best is None or len(key) > len(best[0]):
                best = (key, value)
    return best


def decide(path, module, chapter):
    """The capability of one item, or REFUSE when the tables cannot decide."""
    rule = longest_rule(ITEMS, path)
    if rule is not None:
        return rule[1]
    name = path.rsplit("::", 1)[-1]
    if name in NAMES:
        return NAMES[name]
    rule = longest_rule(MODULES, module)
    if rule is not None:
        return rule[1]
    return CHAPTERS.get(chapter, REFUSE)


def suprnova_paths(record, by_id):
    """Every `suprnova::` path that names this record, canonical first."""
    if record["crate"] == "suprnova":
        paths = [strip_suffix(record["id"])] + [strip_suffix(p) for p in record.get("also") or []]
    else:
        paths = [strip_suffix(p) for p in record.get("aliases") or [] if p.startswith("suprnova::")]
    parent = record.get("parent")
    if parent and parent in by_id and not paths:
        name = strip_suffix(record["id"]).rsplit("::", 1)[-1]
        paths = [p + "::" + name for p in suprnova_paths(by_id[parent], by_id)]
    elif parent and parent in by_id:
        name = strip_suffix(record["id"]).rsplit("::", 1)[-1]
        paths += [p + "::" + name for p in suprnova_paths(by_id[parent], by_id)]
    seen, out = set(), []
    for path in paths:
        if path.startswith("suprnova::") and path not in seen:
            seen.add(path)
            out.append(path)
    return out


def module_of(record, path):
    """The `suprnova::` module an item sits in, for the module rules."""
    module = record.get("module") or ""
    if module.startswith("suprnova::") or module == "suprnova":
        return module
    return path.rsplit("::", 1)[0]


PATH_TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+")


def suprnova_names(rust, by_id):
    """Every path a sibling crate's item is reachable by, mapped to the
    `suprnova::` path that re-exports it, so a return type a sibling
    crate's path names reads as the Suprnova item the scan admits."""
    names = {}
    for record in rust:
        if record["crate"] == "suprnova" or record.get("parent"):
            continue
        paths = suprnova_paths(record, by_id)
        if not paths:
            continue
        own = [strip_suffix(record["id"]), *(record.get("also") or [])]
        own.append(f'{record.get("module")}::{strip_suffix(record["id"]).rsplit("::", 1)[-1]}')
        for path in own:
            names.setdefault(path, paths[0])
    return names


def returned(record, names):
    """A function's return type with every sibling-crate path rewritten to
    its `suprnova::` path, or None when the record carries none."""
    text = (record.get("details") or {}).get("returns")
    if text is None:
        return None
    return PATH_TOKEN.sub(lambda match: names.get(match.group(0), match.group(0)), text)


def build():
    records = [json.loads(line) for line in SURFACE.read_text().splitlines() if line.strip()]
    rust = [r for r in records if r.get("family") == "rust-api" and r.get("kind") != "argument"]
    by_id = {r["id"]: r for r in rust}
    names = suprnova_names(rust, by_id)
    items = {}
    for record in rust:
        paths = suprnova_paths(record, by_id)
        if not paths:
            continue
        canonical = paths[0]
        target_kind = (record.get("details") or {}).get("target_kind")
        if record["kind"] == "reexport" and target_kind == "module":
            continue  # a re-exported crate: the CRATES table decides it
        capability = decide(canonical, module_of(record, canonical), record.get("chapter"))
        refused = capability == REFUSE
        if refused:
            capability = None
        if capability is not None and capability not in CAPABILITIES:
            raise SystemExit(f"unknown capability {capability!r} for {canonical}")
        implements = sorted(
            {strip_suffix(t) for t in (record.get("details") or {}).get("implements") or [] if t.startswith("suprnova::")}
        )
        entry = items.get(canonical)
        if entry is None:
            entry = {
                "path": canonical,
                "kind": record["kind"],
                "capability": capability,
                "hidden": bool(record.get("hidden")),
                "aliases": [],
                "implements": [],
                "prefix": record["kind"] == "reexport",
                "refused": refused,
                "returns": returned(record, names),
            }
            items[canonical] = entry
        else:
            # Two items at one path (a derive and its trait): the stricter
            # reading wins, so a hidden, refused or capability-bearing twin
            # is kept.
            entry["hidden"] = entry["hidden"] or bool(record.get("hidden"))
            entry["refused"] = entry["refused"] or refused
            if entry["capability"] is None:
                entry["capability"] = capability
        if entry["refused"]:
            entry["capability"] = None
        entry["aliases"] = sorted(set(entry["aliases"]) | set(paths[1:]) - {canonical})
        entry["implements"] = sorted(set(entry["implements"]) | set(implements))
        # An enum's variants are named through it and carry its capability.
        if record["kind"] == "enum":
            for variant in (record.get("details") or {}).get("variants") or []:
                variant_path = f"{canonical}::{variant}"
                items.setdefault(variant_path, {
                    "path": variant_path,
                    "kind": "variant",
                    "capability": entry["capability"],
                    "hidden": entry["hidden"],
                    "aliases": sorted(f"{path}::{variant}" for path in paths[1:]),
                    "implements": [],
                    "prefix": False,
                    "refused": entry["refused"],
                })
    hidden_crates = {
        strip_suffix(r["id"])
        for r in rust
        if r.get("crate") == "suprnova" and r["kind"] == "reexport" and r.get("hidden")
        and (r.get("details") or {}).get("target_kind") == "module"
    }
    for path in sorted(hidden_crates):
        items[path] = {"path": path, "kind": "crate", "capability": None, "hidden": True,
                       "aliases": [], "implements": [], "prefix": True, "refused": False}
    for path, capability in CRATES.items():
        if capability == REFUSE:
            continue
        if capability is not None and capability not in CAPABILITIES:
            raise SystemExit(f"unknown capability {capability!r} for {path}")
        items[path] = {"path": path, "kind": "crate", "capability": capability, "hidden": False,
                       "aliases": [], "implements": [], "prefix": True, "refused": False}
    lines = []
    for path in sorted(items):
        entry = items[path]
        lines.append(json.dumps({
            "path": entry["path"],
            "kind": entry["kind"],
            "capability": entry["capability"],
            "hidden": entry["hidden"],
            "prefix": entry["prefix"],
            "refused": entry["refused"],
            "aliases": entry["aliases"],
            "implements": entry["implements"],
            **({"returns": entry["returns"]} if entry.get("returns") is not None else {}),
        }, separators=(",", ":")))
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stdout", action="store_true", help="print the data file instead of writing it")
    parser.add_argument("--check", action="store_true", help="exit 1 when the data file is out of date")
    args = parser.parse_args()
    text = build()
    if args.stdout:
        sys.stdout.write(text)
        return 0
    if args.check:
        current = OUTPUT.read_text() if OUTPUT.exists() else ""
        if current != text:
            print(f"{OUTPUT.relative_to(ROOT)} is out of date; run {Path(__file__).relative_to(ROOT)}", file=sys.stderr)
            return 1
        return 0
    OUTPUT.write_text(text)
    print(f"wrote {OUTPUT.relative_to(ROOT)}: {len(text.splitlines())} paths")
    return 0


if __name__ == "__main__":
    sys.exit(main())
