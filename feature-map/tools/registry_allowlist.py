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
    # The engine surfaces the framework holds for the host, re-exported so an
    # application can name every type a public signature leads to. A
    # component names what the engine hands it (identities, limits, errors,
    # upload inspection results, metadata, payload contracts), never what the
    # host installs or the engine trusts.
    # The host ports a bootstrap installs, with their requests and decisions.
    "suprnova::live::HostCapabilities": REFUSE,
    "suprnova::live::SubscriptionAuthorizationPort": REFUSE,
    "suprnova::live::SubscriptionAuthorizationRequest": REFUSE,
    "suprnova::live::SubscriptionAuthorizationDecision": REFUSE,
    "suprnova::live::SubscriptionAuthorizationOperation": REFUSE,
    "suprnova::live::SubscriptionContinuityPort": REFUSE,
    "suprnova::live::SubscriptionCredentialPort": REFUSE,
    "suprnova::live::SubscriptionCredentialRequest": REFUSE,
    "suprnova::live::SubscriptionCredentialRotationRequest": REFUSE,
    "suprnova::live::SubscriptionCredentialRotationOutcome": REFUSE,
    "suprnova::live::SubscriptionCredentialScope": REFUSE,
    "suprnova::live::SubscriptionRegistryPort": REFUSE,
    "suprnova::live::SubscriptionRegistryRequest": REFUSE,
    "suprnova::live::SubscriptionBaselineRequest": REFUSE,
    "suprnova::live::SubscriptionBinding": REFUSE,
    "suprnova::live::UploadAuthorizationPort": REFUSE,
    "suprnova::live::UploadAuthorizationRequest": REFUSE,
    "suprnova::live::UploadAuthorizationDecision": REFUSE,
    "suprnova::live::action::ActionAuthorizationPort": REFUSE,
    "suprnova::live::action::ActionAuthorizationRequest": REFUSE,
    "suprnova::live::action::AuthorizationDecision": REFUSE,
    # The trusted contexts, verified values and fingerprints the engine
    # mints; a component that could build one would forge trust.
    "suprnova::live::TrustedLiveRequestContext": REFUSE,
    "suprnova::live::TrustedMountParameters": REFUSE,
    "suprnova::live::TrustedPromotionContext": REFUSE,
    "suprnova::live::VerifiedMountCatalogMatch": REFUSE,
    "suprnova::live::VerifiedChildParametersV1": REFUSE,
    "suprnova::live::EligibleChildParametersV2": REFUSE,
    "suprnova::live::ChildParameterAdmissionCarrier": REFUSE,
    "suprnova::live::PrincipalFingerprint": REFUSE,
    "suprnova::live::SessionFingerprint": REFUSE,
    "suprnova::live::TenantFingerprint": REFUSE,
    "suprnova::live::ScopeFingerprint": REFUSE,
    "suprnova::live::HostScopeFacts": REFUSE,
    "suprnova::live::BrowserNonce": REFUSE,
    "suprnova::live::TransferGrantScope": REFUSE,
    "suprnova::live::TransportCredential": REFUSE,
    "suprnova::live::AuthorizationMemo": REFUSE,
    "suprnova::live::AuthoritativeStreamPosition": REFUSE,
    "suprnova::live::CurrentSubscriptionRegistration": REFUSE,
    "suprnova::live::KeyId": REFUSE,
    # The snapshot key material.
    "suprnova::render_cache::RootKey": REFUSE,
    "suprnova::render_cache::KeyRecord": REFUSE,
    "suprnova::render_cache::SnapshotKeyRing": REFUSE,
    "suprnova::render_cache::SignedMac": REFUSE,
    "suprnova::render_cache::SnapshotSignature": REFUSE,
    "suprnova::render_cache::SnapshotPurpose": REFUSE,
    "suprnova::render_cache::PrivateMaterial": REFUSE,
    # The wire protocol and proposal application the framework performs.
    "suprnova::live::UpdateRequest": REFUSE,
    "suprnova::live::UpdateRequestV2": REFUSE,
    "suprnova::live::VersionedUpdateRequest": REFUSE,
    "suprnova::live::Operation": REFUSE,
    "suprnova::live::OperationV2": REFUSE,
    "suprnova::live::ProposalBatch": REFUSE,
    "suprnova::live::RawModelProposal": REFUSE,
    "suprnova::live::ProposedValue": REFUSE,
    "suprnova::live::ProposalApplication": REFUSE,
    "suprnova::live::SnapshotInput": REFUSE,
    "suprnova::live::RequiredChecks": REFUSE,
    "suprnova::live::CheckKind": REFUSE,
    "suprnova::live::CheckDisposition": REFUSE,
    # The component hooks and schemas the macros implement.
    "suprnova::live::ComponentDescriptor": REFUSE,
    "suprnova::live::ComponentFactory": REFUSE,
    "suprnova::live::ComponentInstance": REFUSE,
    "suprnova::live::ComponentHooks": REFUSE,
    "suprnova::live::MountContext": REFUSE,
    "suprnova::live::HydrationContext": REFUSE,
    "suprnova::live::RenderContext": REFUSE,
    "suprnova::live::BrowserRenderContext": REFUSE,
    "suprnova::live::LiveFuture": REFUSE,
    "suprnova::live::SnapshotSchemaSet": REFUSE,
    "suprnova::live::StateSchema": REFUSE,
    "suprnova::live::StateCodec": REFUSE,
    "suprnova::live::StateExposure": REFUSE,
    "suprnova::live::ModelBindingSchema": REFUSE,
    "suprnova::live::ModelField": REFUSE,
    "suprnova::live::ModelFieldBinding": REFUSE,
    "suprnova::live::ModelCodec": REFUSE,
    "suprnova::live::FieldSpec": REFUSE,
    "suprnova::live::FieldCategory": REFUSE,
    "suprnova::live::UrlBinding": REFUSE,
    "suprnova::live::UrlBindingMode": REFUSE,
    "suprnova::live::BindingTiming": REFUSE,
    "suprnova::live::action::ActionDispatchFn": REFUSE,
    "suprnova::live::action::ActionFuture": REFUSE,
    "suprnova::live::action::ActionTarget": REFUSE,
    "suprnova::live::action::PreparedActionArguments": REFUSE,
    "suprnova::live::action::RawActionArguments": REFUSE,
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
    # Test fakes and app-wide hooks (REG-030). A fake swaps a facade's driver
    # for every caller in the process, a global hook rewrites every request
    # the application makes, an install or a set_driver rebinds a service
    # for the whole application, and the test container or clock changes
    # what every other caller sees: none of that is a component's to do,
    # whatever capability the facade's chapter grants, as App::bind and the
    # global middleware registration are refused above.
    "suprnova::Bus::fake": REFUSE,
    "suprnova::Cache::set_connection": REFUSE,
    "suprnova::CacheStore::set_connection": REFUSE,
    "suprnova::EventFacade::fake": REFUSE,
    "suprnova::EventFacade::fake_except": REFUSE,
    "suprnova::EventFacade::fake_only": REFUSE,
    "suprnova::FailOnRealCallsGuard": REFUSE,
    "suprnova::FakeDescription": REFUSE,
    "suprnova::FakeHandler": REFUSE,
    "suprnova::FakeResult": REFUSE,
    "suprnova::FakeSequence": REFUSE,
    "suprnova::Http::allow_stray_requests": REFUSE,
    "suprnova::Http::fake": REFUSE,
    "suprnova::Http::fake_response_text": REFUSE,
    "suprnova::Http::fake_sequence": REFUSE,
    "suprnova::Http::fake_url": REFUSE,
    "suprnova::Http::fake_using": REFUSE,
    "suprnova::Http::global_options": REFUSE,
    "suprnova::Http::global_request_middleware": REFUSE,
    "suprnova::Http::global_response_middleware": REFUSE,
    "suprnova::Http::prevent_stray_requests": REFUSE,
    "suprnova::Inertia::install": REFUSE,
    # Declares the application's Inertia root document, the template a first
    # visit renders into: the application's to write, never a component's.
    "suprnova::inertia_root": REFUSE,
    "suprnova::Mail::fake": REFUSE,
    "suprnova::MailFake": REFUSE,
    "suprnova::Process::fake": REFUSE,
    "suprnova::ProcessFake": REFUSE,
    "suprnova::Queue::fake": REFUSE,
    "suprnova::Queue::fake_except": REFUSE,
    "suprnova::Queue::set_driver": REFUSE,
    "suprnova::RenderCache::install": REFUSE,
    "suprnova::SessionMiddleware::install": REFUSE,
    "suprnova::Storage::fake": REFUSE,
    "suprnova::VectorRegistry::install": REFUSE,
    "suprnova::testing::TestClock": REFUSE,
    "suprnova::testing::TestContainer": REFUSE,
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

# The engine crates (the Live engine, its macros and Magnetar) and the paths
# the component scan guards: every item of an engine crate, and every item
# under suprnova::live, suprnova::render_cache or suprnova::view, reaches a
# component only when ADMIT names it or its type, each with the capability
# its chapter and module decide. The framework re-exports engine types so an
# application can name every type a public signature leads to, and it keeps
# adding Live items of its own; neither lands on the component allowlist by
# default, a rule here admits it. The table is the boundary as it stood
# before the 4.0.0 re-exports (REG-030); an ITEMS rule that refuses a path
# wins over an entry here. Magnetar has no entry: authentication is the
# application's, through the framework's auth flows, never a component's.
ENGINE_CRATES = {"suprnova_live", "suprnova_live_macros", "magnetar"}
GATED_PREFIXES = ("suprnova::live", "suprnova::render_cache", "suprnova::view")
ADMIT = {
    "suprnova::live::AcceptedOutcomeKind": None,
    "suprnova::live::ActionOutcome": None,
    "suprnova::live::ActionResult": None,
    "suprnova::live::AuthorizedAction": None,
    "suprnova::live::BoundedHeaders": None,
    "suprnova::live::BuildId": None,
    "suprnova::live::CancellationFlag": None,
    "suprnova::live::CanonicalValue": None,
    "suprnova::live::ChunkDisposition": None,
    "suprnova::live::ChunkReceipt": None,
    "suprnova::live::Clock": None,
    "suprnova::live::ClockError": None,
    "suprnova::live::ClockErrorKind": None,
    "suprnova::live::ComponentContract": None,
    "suprnova::live::ComponentName": None,
    "suprnova::live::ContentDigest": None,
    "suprnova::live::DirectPartReference": None,
    "suprnova::live::DirectTransferInstruction": None,
    "suprnova::live::DirectUploadProvider": None,
    "suprnova::live::DocumentMountKey": None,
    "suprnova::live::DurableUpload": None,
    "suprnova::live::DurableUploadId": None,
    "suprnova::live::EffectPayloadMetadata": None,
    "suprnova::live::ErrorBag": None,
    "suprnova::live::ErrorCategory": None,
    "suprnova::live::EventPayloadMetadata": None,
    "suprnova::live::FailedFinalize": None,
    "suprnova::live::FinalizeRequest": None,
    "suprnova::live::FinalizeToken": None,
    "suprnova::live::IntegrityEvidence": None,
    "suprnova::live::IslandSlot": None,
    "suprnova::live::LIVE_UI_ASSET_PATH_PREFIX": None,
    "suprnova::live::LIVE_UI_TEMPLATE_ROOT": None,
    "suprnova::live::LedgerDriver": None,
    "suprnova::live::LiveBootstrap": None,
    "suprnova::live::LiveBootstrapOptions": None,
    "suprnova::live::LiveBootstrapStrategy": None,
    "suprnova::live::LiveConfig": None,
    "suprnova::live::LiveConfigBuilder": None,
    "suprnova::live::LiveConfigError": None,
    "suprnova::live::LiveConfigErrorKind": None,
    "suprnova::live::LiveDocument": None,
    "suprnova::live::LiveDocumentError": None,
    "suprnova::live::LiveDocumentErrorKind": None,
    "suprnova::live::LiveError": None,
    "suprnova::live::LiveEventTarget": None,
    "suprnova::live::LiveLimitExceeded": None,
    "suprnova::live::LiveMorphLimits": None,
    "suprnova::live::LiveMount": None,
    "suprnova::live::LiveMountKind": None,
    "suprnova::live::LiveNestedSegment": None,
    "suprnova::live::LiveOutcomeAccepted": None,
    "suprnova::live::LiveRegistry": None,
    "suprnova::live::LiveRegistryBuilder": None,
    "suprnova::live::LiveRouteGuard": None,
    "suprnova::live::LiveRuntime": None,
    "suprnova::live::LiveServerLimits": None,
    "suprnova::live::LiveStreamError": None,
    "suprnova::live::LiveStreamErrorKind": None,
    "suprnova::live::LiveStreams": None,
    "suprnova::live::LiveTenantMiddleware": None,
    "suprnova::live::LiveTenantResolver": None,
    "suprnova::live::LiveUploadHost": None,
    "suprnova::live::LiveUploadLimits": None,
    "suprnova::live::MountFlags": None,
    "suprnova::live::MountedDocumentPath": None,
    "suprnova::live::MountedIsland": None,
    "suprnova::live::PrepareTransfer": None,
    "suprnova::live::PreparedFinalize": None,
    "suprnova::live::QuarantineBytes": None,
    "suprnova::live::ReadUpload": None,
    "suprnova::live::RecoveryInstruction": None,
    "suprnova::live::RegistryError": None,
    "suprnova::live::RegistryErrorKind": None,
    "suprnova::live::ReportDirectPart": None,
    "suprnova::live::RouteIdentity": None,
    "suprnova::live::SafeDiagnosticCode": None,
    "suprnova::live::ScanDisposition": None,
    "suprnova::live::ScanInput": None,
    "suprnova::live::StitchFailurePolicy": None,
    "suprnova::live::StitchSlotDescriptor": None,
    "suprnova::live::TransferDisposition": None,
    "suprnova::live::TransferInstruction": None,
    "suprnova::live::TransferMethod": None,
    "suprnova::live::TransferPlan": None,
    "suprnova::live::TrustedProviderOrigin": None,
    "suprnova::live::TrustedProviderUrl": None,
    "suprnova::live::UnixMillis": None,
    "suprnova::live::UploadApplicationValidator": None,
    "suprnova::live::UploadError": None,
    "suprnova::live::UploadErrorKind": None,
    "suprnova::live::UploadFinalizer": None,
    "suprnova::live::UploadFuture": None,
    "suprnova::live::UploadHandle": None,
    "suprnova::live::UploadLimitConfig": None,
    "suprnova::live::UploadLimits": None,
    "suprnova::live::UploadPart": None,
    "suprnova::live::UploadPolicy": None,
    "suprnova::live::UploadPolicyBuilder": None,
    "suprnova::live::UploadPolicyError": None,
    "suprnova::live::UploadProvider": None,
    "suprnova::live::UploadReplacement": None,
    "suprnova::live::UploadScan": None,
    "suprnova::live::UploadScanFailure": None,
    "suprnova::live::UploadScanner": None,
    "suprnova::live::UploadType": None,
    "suprnova::live::ValidationIssue": None,
    "suprnova::live::ValidationMessageId": None,
    "suprnova::live::ValidationStatus": None,
    "suprnova::live::VerifyTransfer": None,
    "suprnova::live::__private::askama": None,
    "suprnova::live::__private::session::load": None,
    "suprnova::live::__private::session::stage": None,
    "suprnova::live::action::FlashIntent": None,
    "suprnova::live::action::OutcomeError": None,
    "suprnova::live::action::OutcomeErrorKind": None,
    "suprnova::live::action::OutcomeMetadata": None,
    "suprnova::live::action::ResponseIntentError": None,
    "suprnova::live::action::ResponseIntentErrorKind": None,
    "suprnova::live::action::RouteIntent": None,
    "suprnova::live::action::RouteIntentError": None,
    "suprnova::live::action::RouteIntentErrorKind": None,
    "suprnova::live::action::UrlIntent": None,
    "suprnova::live::action::action_result": None,
    "suprnova::live::action::flash_intent": None,
    "suprnova::live::action::route_intent": None,
    "suprnova::live::action::url_intent": None,
    "suprnova::live::assets::ArtifactError": None,
    "suprnova::live::assets::ArtifactErrorKind": None,
    "suprnova::live::assets::ArtifactRole": None,
    "suprnova::live::assets::BootScript": None,
    "suprnova::live::assets::LIVE_ASSET_PATH_PREFIX": None,
    "suprnova::live::assets::LiveAssetCatalog": None,
    "suprnova::live::assets::PreloadRelation": None,
    "suprnova::live::assets::RuntimeArtifact": None,
    "suprnova::live::assets::ScriptKind": None,
    "suprnova::live::assets::live_asset_catalog": None,
    "suprnova::live::charts::ChartError": None,
    "suprnova::live::charts::ChartErrorKind": None,
    "suprnova::live::charts::ChartKind": None,
    "suprnova::live::charts::ChartSeries": None,
    "suprnova::live::charts::render_chart": None,
    "suprnova::live::current_tenant": None,
    "suprnova::live::ledger::CasOutcome": None,
    "suprnova::live::ledger::InstanceRecordKey": None,
    "suprnova::live::ledger::InstanceRecordStore": None,
    "suprnova::live::ledger::LedgerError": None,
    "suprnova::live::ledger::LedgerErrorKind": None,
    "suprnova::live::ledger::PromotionRecordKey": None,
    "suprnova::live::ledger::StoredRecord": None,
    "suprnova::live::production_ledger_limits": None,
    "suprnova::live::registry::sealed::Sealed": None,
    "suprnova::live::tooling_protocol::AssetKind": None,
    "suprnova::live::tooling_protocol::AssetReport": None,
    "suprnova::live::tooling_protocol::Body": None,
    "suprnova::live::tooling_protocol::COMMAND_NAME": None,
    "suprnova::live::tooling_protocol::CheckSummary": None,
    "suprnova::live::tooling_protocol::ComponentReport": None,
    "suprnova::live::tooling_protocol::ConfigReport": None,
    "suprnova::live::tooling_protocol::DiagnosticReport": None,
    "suprnova::live::tooling_protocol::EndReport": None,
    "suprnova::live::tooling_protocol::Envelope": None,
    "suprnova::live::tooling_protocol::LimitReport": None,
    "suprnova::live::tooling_protocol::MAX_ASSETS": None,
    "suprnova::live::tooling_protocol::MAX_ASSET_BYTES": None,
    "suprnova::live::tooling_protocol::MAX_COMPONENTS": None,
    "suprnova::live::tooling_protocol::MAX_DIAGNOSTICS": None,
    "suprnova::live::tooling_protocol::MAX_ENVELOPES": None,
    "suprnova::live::tooling_protocol::MAX_LIMITS": None,
    "suprnova::live::tooling_protocol::MAX_LINE_BYTES": None,
    "suprnova::live::tooling_protocol::MAX_TEMPLATE_DEPTH": None,
    "suprnova::live::tooling_protocol::MAX_TEMPLATE_FILES": None,
    "suprnova::live::tooling_protocol::MAX_TEMPLATE_FILE_BYTES": None,
    "suprnova::live::tooling_protocol::MAX_TEMPLATE_ROOTS": None,
    "suprnova::live::tooling_protocol::MAX_TEMPLATE_TOTAL_BYTES": None,
    "suprnova::live::tooling_protocol::MAX_TEXT_BYTES": None,
    "suprnova::live::tooling_protocol::MAX_TOTAL_BYTES": None,
    "suprnova::live::tooling_protocol::MIN_PROTOCOL_VERSION": None,
    "suprnova::live::tooling_protocol::Operation": None,
    "suprnova::live::tooling_protocol::Outcome": None,
    "suprnova::live::tooling_protocol::PROTOCOL_VERSION": None,
    "suprnova::live::tooling_protocol::ReadinessReport": None,
    "suprnova::live::tooling_protocol::RuntimeReport": None,
    "suprnova::live::tooling_protocol::Severity": None,
    "suprnova::live::tooling_protocol::UploadHostReport": None,
    "suprnova::live::validation::BagPolicy": None,
    "suprnova::live::verify_ledger_backend": None,
    "suprnova::live::verify_ledger_driver_for_test": None,
    "suprnova::render_cache::AuthorizationConsult": None,
    "suprnova::render_cache::CoherenceMode": None,
    "suprnova::render_cache::CoordinatorConfig": None,
    "suprnova::render_cache::DeclineReason": None,
    "suprnova::render_cache::DependencyIdentity": None,
    "suprnova::render_cache::Eligibility": None,
    "suprnova::render_cache::EntryInspection": None,
    "suprnova::render_cache::EntryKind": None,
    "suprnova::render_cache::FailurePolicy": None,
    "suprnova::render_cache::FreshnessPolicy": None,
    "suprnova::render_cache::GenerationLedger": None,
    "suprnova::render_cache::GenerationSet": None,
    "suprnova::render_cache::HintsConfig": None,
    "suprnova::render_cache::L0Limits": None,
    "suprnova::render_cache::L1Config": None,
    "suprnova::render_cache::L1Provider": None,
    "suprnova::render_cache::LeaseAttempt": None,
    "suprnova::render_cache::LeaseStore": None,
    "suprnova::render_cache::NegotiatedPolicy": None,
    "suprnova::render_cache::PolicyPatch": None,
    "suprnova::render_cache::Profile": None,
    "suprnova::render_cache::QueryPolicy": None,
    "suprnova::render_cache::RedisRenderStore": None,
    "suprnova::render_cache::RenderCacheConfig": None,
    "suprnova::render_cache::RenderCacheError": None,
    "suprnova::render_cache::RenderCacheErrorKind": None,
    "suprnova::render_cache::RenderCacheMiddleware": None,
    "suprnova::render_cache::RenderCachePolicy": None,
    "suprnova::render_cache::RenderCachePolicyBuilder": None,
    "suprnova::render_cache::RenderCacheRuntime": None,
    "suprnova::render_cache::RenderKey": None,
    "suprnova::render_cache::RepresentationClass": None,
    "suprnova::render_cache::Segment": None,
    "suprnova::render_cache::SharedCachePolicy": None,
    "suprnova::render_cache::ShellIsland": None,
    "suprnova::render_cache::SlotFailurePolicy": None,
    "suprnova::render_cache::SqlRenderStore": None,
    "suprnova::render_cache::StorageLayers": None,
    "suprnova::render_cache::StoreInspection": None,
    "suprnova::render_cache::SweepOutcome": None,
    "suprnova::render_cache::VarianceDimension": None,
    "suprnova::render_cache::collector::CollectedContext": None,
    "suprnova::render_cache::collector::Collector": None,
    "suprnova::render_cache::collector::CollectorReport": None,
    "suprnova::render_cache::collector::ConsultWindow": None,
    "suprnova::render_cache::collector::GateReport": None,
    "suprnova::render_cache::collector::PERMISSION_VERSION_CONFIG_KEY": None,
    "suprnova::render_cache::collector::begin_authorization_decision": None,
    "suprnova::render_cache::collector::begin_handler": None,
    "suprnova::render_cache::collector::current_report": None,
    "suprnova::render_cache::collector::end_authorization_decision": None,
    "suprnova::render_cache::collector::is_active": None,
    "suprnova::render_cache::collector::observe": None,
    "suprnova::render_cache::collector::observe_feature_read": None,
    "suprnova::render_cache::collector::observe_foreign_connection_read": None,
    "suprnova::render_cache::collector::observe_live_document_bootstrap_nonce": None,
    "suprnova::render_cache::collector::observe_live_document_digest": None,
    "suprnova::render_cache::collector::observe_live_document_mount": None,
    "suprnova::render_cache::collector::observe_live_document_no_store": None,
    "suprnova::render_cache::collector::observe_live_document_shell_island": None,
    "suprnova::render_cache::collector::observe_live_document_stitch_invalid": None,
    "suprnova::render_cache::collector::observe_live_document_stitch_slot": None,
    "suprnova::render_cache::collector::observe_locale_value": None,
    "suprnova::render_cache::collector::observe_principal_read": None,
    "suprnova::render_cache::collector::observe_principal_value": None,
    "suprnova::render_cache::collector::observe_record_read": None,
    "suprnova::render_cache::collector::observe_record_read_json": None,
    "suprnova::render_cache::collector::observe_secret_context_read": None,
    "suprnova::render_cache::collector::observe_session_read": None,
    "suprnova::render_cache::collector::observe_table_read": None,
    "suprnova::render_cache::collector::observe_tenant_read": None,
    "suprnova::render_cache::collector::observe_tenant_value": None,
    "suprnova::render_cache::collector::observe_undeclared": None,
    "suprnova::render_cache::collector::observe_unkeyed_write": None,
    "suprnova::render_cache::collector::observe_unobservable_read": None,
    "suprnova::render_cache::collector::permission_version_identity": None,
    "suprnova::render_cache::collector::record_identity": None,
    "suprnova::render_cache::collector::resolvable_reads": None,
    "suprnova::render_cache::collector::slot_scope": None,
    "suprnova::render_cache::collector::strip_classification_reasons_for_test": None,
    "suprnova::render_cache::console::epoch_advance_report_for_test": None,
    "suprnova::render_cache::console::inspect_report_for_test": None,
    "suprnova::render_cache::file_store::FileRenderStore": None,
    "suprnova::render_cache::file_store::PublishIoHook": None,
    "suprnova::render_cache::file_store::PublishIoStage": None,
    "suprnova::render_cache::hints::MAX_HINT_DIGESTS": None,
    "suprnova::render_cache::hints::MAX_INBOUND_HINTS": None,
    "suprnova::render_cache::ledger::SqlGenerationLedger": None,
    "suprnova::render_cache::ledger::advance_in_current_transaction": None,
    "suprnova::render_cache::ledger::advance_via_handle": None,
    "suprnova::render_cache::ledger::advance_via_tx": None,
    "suprnova::render_cache::ledger::tier_migration_present": None,
    "suprnova::render_cache::live::CapturedSlot": None,
    "suprnova::render_cache::live::LiveDocumentDecline": None,
    "suprnova::render_cache::live::LiveDocumentFacts": None,
    "suprnova::render_cache::live::StitchCapture": None,
    "suprnova::render_cache::live::document_declines": None,
    "suprnova::render_cache::live::record_bootstrap_nonce": None,
    "suprnova::render_cache::live::record_document_digest": None,
    "suprnova::render_cache::live::record_document_intent": None,
    "suprnova::render_cache::live::record_mount": None,
    "suprnova::render_cache::live::record_shell_island": None,
    "suprnova::render_cache::live::record_stitch_capture_invalid": None,
    "suprnova::render_cache::live::record_stitch_slot": None,
    "suprnova::render_cache::live::seed_remaining_ms": None,
    "suprnova::render_cache::mark_installed": None,
    "suprnova::render_cache::middleware::key_input_for_test": None,
    "suprnova::render_cache::middleware::race_points::AFTER_REREAD": None,
    "suprnova::render_cache::middleware::race_points::AFTER_VIEW_CLOSE": None,
    "suprnova::render_cache::middleware::race_points::AUTHORITY_READ_RETURNED": None,
    "suprnova::render_cache::middleware::race_points::BEFORE_VIEW": None,
    "suprnova::render_cache::middleware::race_points::DURING_REREAD": None,
    "suprnova::render_cache::middleware::race_points::EPOCH_CAPTURED": None,
    "suprnova::render_cache::middleware::race_points::Hook": None,
    "suprnova::render_cache::middleware::race_points::RacePoint": None,
    "suprnova::render_cache::middleware::race_points::arm": None,
    "suprnova::render_cache::middleware::race_points::disarm": None,
    "suprnova::render_cache::migration::Migration": None,
    "suprnova::render_cache::migration::TierMigration": None,
    "suprnova::render_cache::orm::after_bulk_write": None,
    "suprnova::render_cache::orm::after_bulk_write_with_handle": None,
    "suprnova::render_cache::orm::after_model_write": None,
    "suprnova::render_cache::orm::after_model_write_with_tx": None,
    "suprnova::render_cache::orm::after_row_write": None,
    "suprnova::render_cache::orm::after_row_write_with_handle": None,
    "suprnova::render_cache::orm::after_table_write": None,
    "suprnova::render_cache::orm::after_unknown_write": None,
    "suprnova::render_cache::providers::RedisInstanceRecordStore": None,
    "suprnova::render_cache::providers::RedisLeaseStore": None,
    "suprnova::render_cache::providers::RedisProviderConfig": None,
    "suprnova::render_cache::providers::SqlInstanceRecordStore": None,
    "suprnova::render_cache::providers::SqlLeaseStore": None,
    "suprnova::render_cache::providers::redis::ping": None,
    "suprnova::render_cache::providers::sql_instances::ReclaimPause": None,
    "suprnova::render_cache::providers::sql_now_ms": None,
    "suprnova::render_cache::providers::store_now_ms": None,
    "suprnova::render_cache::registry::GroupPolicy": None,
    "suprnova::render_cache::registry::RenderCachePolicyTable": None,
    "suprnova::render_cache::store::PublicationFence": None,
    "suprnova::render_cache::store::PublishOutcome": None,
    "suprnova::render_cache::store::RenderStore": None,
    "suprnova::render_cache::store::StoreInspection": None,
    "suprnova::render_cache::store::StoredEntry": None,
    "suprnova::render_cache::sweep_l1": None,
    "suprnova::render_cache::telemetry::CAUSE": None,
    "suprnova::render_cache::telemetry::EPOCH_REWINDS": None,
    "suprnova::render_cache::telemetry::HINTS": None,
    "suprnova::render_cache::telemetry::HITS": None,
    "suprnova::render_cache::telemetry::LOOKUPS": None,
    "suprnova::render_cache::telemetry::OUTCOME": None,
    "suprnova::render_cache::telemetry::PUBLICATIONS": None,
    "suprnova::render_cache::telemetry::REASON": None,
    "suprnova::render_cache::telemetry::REBUILDS": None,
    "suprnova::render_cache::telemetry::RecordedLookup": None,
    "suprnova::render_cache::telemetry::STITCH_ASSEMBLIES": None,
    "suprnova::render_cache::telemetry::STITCH_NESTED": None,
    "suprnova::render_cache::telemetry::STITCH_SLOTS": None,
    "suprnova::render_cache::telemetry::await_hint_for_test": None,
    "suprnova::render_cache::telemetry::decline_reason_labels_for_test": None,
    "suprnova::render_cache::telemetry::recorded_hints_for_test": None,
    "suprnova::render_cache::telemetry::recorded_lookups_for_test": None,
    "suprnova::render_cache::telemetry::reset_recorded_hints_for_test": None,
    "suprnova::render_cache::telemetry::reset_recorded_lookups_for_test": None,
    "suprnova::render_cache::write_side::WriteSideDecision": None,
    "suprnova::render_cache::write_side::decide": None,
    "suprnova::render_cache::write_side::decision": None,
    "suprnova::view::AssetSet": None,
    "suprnova::view::CanonicalDocumentConformance": None,
    "suprnova::view::CanonicalDocumentIntent": None,
    "suprnova::view::CanonicalDocumentRequest": None,
    "suprnova::view::ChildMount": None,
    "suprnova::view::DocumentCachePolicy": None,
    "suprnova::view::DocumentMediaType": None,
    "suprnova::view::DocumentRender": None,
    "suprnova::view::DocumentResponseError": None,
    "suprnova::view::DocumentResponseErrorKind": None,
    "suprnova::view::DocumentResponseIntent": None,
    "suprnova::view::DocumentValidator": None,
    "suprnova::view::FilterResult": None,
    "suprnova::view::FilterValues": None,
    "suprnova::view::HeaderName": None,
    "suprnova::view::HeaderValue": None,
    "suprnova::view::MountMetadata": None,
    "suprnova::view::MountSnapshotKind": None,
    "suprnova::view::RegisteredSanitizer": None,
    "suprnova::view::RenderLimits": None,
    "suprnova::view::SanitizerFailure": None,
    "suprnova::view::SanitizerId": None,
    "suprnova::view::StatusCode": None,
    "suprnova::view::TemplateFailure": None,
    "suprnova::view::TrustedHtml": None,
    "suprnova::view::TrustedMarkupError": None,
    "suprnova::view::TrustedMarkupErrorKind": None,
    "suprnova::view::TrustedMarkupReason": None,
    "suprnova::view::ViewError": None,
    "suprnova::view::ViewErrorKind": None,
    "suprnova::view::ViewRenderer": None,
    "suprnova::view::ViewTemplate": None,
    "suprnova::view::document_response": None,
    "suprnova::view::filters::live_key": None,
    "suprnova::view::filters::live_key::execute": None,
    "suprnova::view::filters::live_key_digest": None,
    "suprnova::view::filters::live_key_digest::execute": None,
    "suprnova::view::filters::trusted_html": None,
    "suprnova::view::filters::trusted_html::execute": None,
    "suprnova::view::sealed::Sealed": None,
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


def admission(record, canonical, module):
    """The capability of one item, or REFUSE: the tables' decision, and for
    an engine crate's item or an item under a guarded path only when ADMIT
    names it or its type."""
    capability = decide(canonical, module, record.get("chapter"))
    if capability == REFUSE:
        return REFUSE
    guarded = record.get("crate") in ENGINE_CRATES or any(
        canonical.startswith(prefix + "::") for prefix in GATED_PREFIXES
    )
    if guarded and longest_rule(ADMIT, canonical) is None:
        return REFUSE
    return capability


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
        capability = admission(record, canonical, module_of(record, canonical))
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
