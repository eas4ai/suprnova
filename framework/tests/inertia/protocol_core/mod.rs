//! The Inertia protocol core against Laravel's adapter (PAR-046, PAR-047,
//! PAR-053 and PAR-056 of `docs/spec/laravel-parity.md`): request
//! detection and the asset version, partial reload parsing and narrowing,
//! the first-visit JSON, and the page `url`.
//!
//! Most tests drive `InertiaResponse::resolve` through an
//! `InertiaRequestExt` mock. The ones that need a real `Request` (the
//! header detection on the type every middleware reads, the version
//! middleware's 409) serve a router on a loopback socket.

mod partial;
mod request;
mod support;
mod version;
