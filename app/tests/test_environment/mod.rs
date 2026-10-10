//! The environment a test process of this application runs in.
//!
//! An unset `APP_ENV` is production, as in Laravel, and production refuses
//! what these tests boot with: an Inertia shell with no built Vite manifest,
//! in-memory drivers, a server without an `APP_KEY`. Laravel's `phpunit.xml`
//! sets `APP_ENV=testing` for its test processes; a test here calls
//! [`testing`] before it boots the application.

use std::sync::Once;

/// Make this test process a `testing` one, unless it already names an
/// environment. Every boot after the first call sees it.
pub fn testing() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("APP_ENV").is_none() {
            // SAFETY: writing the environment races a thread that reads it
            // at the same moment. This is one write, made once per process
            // before its first boot of the application, so the application's
            // own threads do not exist yet. Under nextest each test is a
            // process of its own; under `cargo test` a test on another thread
            // may read the environment at that moment, the risk every test
            // that sets a variable in this workspace takes.
            unsafe { std::env::set_var("APP_ENV", "testing") };
        }
    });
}
