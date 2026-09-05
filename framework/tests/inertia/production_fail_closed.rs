//! CFG-01: Inertia must not default to development mode in production.
//!
//! `InertiaConfig::default()` used to hardcode `development: true`
//! regardless of `APP_ENV`, so a production deployment that never
//! explicitly called `.production()` rendered asset URLs pointing at a
//! local Vite dev server. The fix derives the default from
//! `Environment::detect().is_production()` and makes `Inertia::install`
//! fail closed when production mode has no build manifest to back it.
//!
//! `Environment::detect()` reads the process-wide `APP_ENV` var. These
//! tests mutate it, so - like `app_key_production_fail_closed.rs` -
//! they run in their own test binary and serialize on the same lock
//! (`#[serial_test::serial]`) with each other so they don't race within
//! this file; no other integration test file's tests can interleave with
//! these since each `tests/*.rs` file is a separate process.
//!
//! The manifest-missing / fail-closed error itself, independent of
//! `APP_ENV`, is covered by an env-var-free unit test in
//! `framework/src/inertia/facade.rs` (`install_fails_closed_in_production_without_a_manifest`).

use std::collections::HashMap;
use suprnova::{Inertia, InertiaConfig, InertiaRequestExt, InertiaResponse};

/// Minimal `InertiaRequestExt` impl for a full HTML page-load (no
/// `X-Inertia` header). Mirrors the one in `inertia.rs`; small enough
/// that duplicating it beats wiring up a shared `tests/common` module.
struct MockReq {
    path: String,
    headers: HashMap<String, String>,
}

impl MockReq {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            headers: HashMap::new(),
        }
    }
}

impl InertiaRequestExt for MockReq {
    fn path(&self) -> &str {
        &self.path
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|s| s.as_str())
    }
}

/// Set `APP_ENV=production` and return whatever value was there before,
/// so the caller can restore it.
///
/// # Safety
/// Mutates a process-global env var. Safe here because every test in
/// this file that calls it is `#[serial_test::serial]`-locked against
/// the others, and this binary contains no other tests.
fn set_app_env_production() -> Option<String> {
    let prior = std::env::var("APP_ENV").ok();
    unsafe {
        std::env::set_var("APP_ENV", "production");
    }
    prior
}

/// # Safety
/// See [`set_app_env_production`].
fn restore_app_env(prior: Option<String>) {
    unsafe {
        match prior {
            Some(v) => std::env::set_var("APP_ENV", v),
            None => std::env::remove_var("APP_ENV"),
        }
    }
}

#[test]
#[serial_test::serial(inertia_cfg01_app_env)]
fn default_config_resolves_production_mode_under_app_env_production() {
    let prior = set_app_env_production();

    let cfg = InertiaConfig::default();
    assert!(
        !cfg.development,
        "InertiaConfig::default() must resolve to production mode \
         (development = false) when APP_ENV=production (CFG-01)"
    );

    restore_app_env(prior);
}

#[tokio::test]
#[serial_test::serial(inertia_cfg01_app_env)]
async fn production_html_shell_never_references_a_localhost_dev_server() {
    let prior = set_app_env_production();

    // No manifest exists at the default path relative to this crate's
    // test working directory, so this exercises the "manifest absent"
    // fallback inside `render_prod_head` - the important thing is that
    // it's a fallback path at all (production mode engaged), not
    // `render_dev_head`, which is what CFG-01 let slip through.
    let req = MockReq::new("/");
    let resp = InertiaResponse::new("Home")
        .resolve(&req)
        .await
        .expect("resolve should succeed via the legacy fallback even without a manifest");

    let body = {
        use http_body_util::BodyExt;
        let bytes = resp
            .into_hyper()
            .into_body()
            .collect()
            .await
            .expect("collecting the response body should not fail")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("HTML body must be valid UTF-8")
    };

    assert!(
        !body.contains("localhost"),
        "production HTML shell must never reference a localhost dev-server URL, got: {body}"
    );
    assert!(
        !body.contains("@vite/client"),
        "production HTML shell must not load the Vite dev-server client script, got: {body}"
    );

    restore_app_env(prior);
}

#[test]
#[serial_test::serial(inertia_cfg01_app_env)]
fn install_with_default_config_fails_closed_in_production_without_a_manifest() {
    let prior = set_app_env_production();

    // No explicit `.production()` override - this is the environment-
    // derived default an app gets by simply not thinking about it,
    // which is exactly the scenario CFG-01 left exposed.
    let cfg = InertiaConfig::default();
    assert!(
        !cfg.development,
        "sanity check: default should already be production mode here"
    );

    let err = Inertia::install(&cfg)
        .expect_err("production boot without a manifest must fail closed (CFG-01)");
    let msg = format!("{err}");
    assert!(
        msg.contains("manifest"),
        "error should mention the missing manifest: {msg}"
    );

    restore_app_env(prior);
}
