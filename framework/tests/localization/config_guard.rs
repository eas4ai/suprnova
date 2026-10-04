//! Puts the process-global `LocalizationConfig` back when a test that
//! registered its own ends.
//!
//! `Config::register` writes to a process-global repository with no
//! unregister, and `Lang` reads its fallback locale and parent chains from
//! it on every call. A test that registered a config with fallback `fr`
//! and left it there made every later test in the same process that relies
//! on the default `en` fallback fail, in whatever order libtest ran them.

use suprnova::{Config, Locale, LocalizationConfig};

/// Restores, on drop, the `LocalizationConfig` that was in effect before
/// [`LocalizationConfigGuard::register`] replaced it. Hold it for as long as
/// the test needs its own config; every caller runs under
/// `#[serial_test::serial]`, so no other test reads the config meanwhile.
#[must_use = "the previous config is restored when the guard drops"]
pub struct LocalizationConfigGuard {
    previous: LocalizationConfig,
}

impl LocalizationConfigGuard {
    /// Register `config` and remember what was in effect before it.
    pub fn register(config: LocalizationConfig) -> Self {
        let previous = Config::get::<LocalizationConfig>().unwrap_or_else(unregistered);
        Config::register(config);
        Self { previous }
    }
}

impl Drop for LocalizationConfigGuard {
    fn drop(&mut self) {
        Config::register(self.previous.clone());
    }
}

/// What `Lang` resolves when no `LocalizationConfig` is registered: the
/// environment's, or `en`/`en` when the environment holds a malformed
/// value. The repository has no unregister, so registering this is how a
/// guard puts back "nothing registered" with the same effect.
fn unregistered() -> LocalizationConfig {
    LocalizationConfig::from_env().unwrap_or_else(|_| LocalizationConfig {
        default_locale: Locale::parse("en").expect("en is a valid locale"),
        fallback_locale: Locale::parse("en").expect("en is a valid locale"),
        use_isolating: false,
        detection: Vec::new(),
        session_key: "locale".into(),
        cookie_name: "locale".into(),
        parents: Default::default(),
    })
}
