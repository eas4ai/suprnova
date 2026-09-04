//! Process-wide RenderCache configuration.

use std::sync::Arc;

use suprnova_live::clock::Clock;
pub use suprnova_live::render_cache::FailurePolicy;
use suprnova_live::render_cache::singleflight::RebuildCoordinator;

/// In-process store bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct L0Limits {
    /// Most entries.
    pub max_entries: usize,
    /// Most bytes.
    pub max_bytes: usize,
}

/// The L1 provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum L1Config {
    /// No L1.
    Disabled,
    /// Atomic files under a directory the process owns.
    File {
        /// Directory the L1 provider owns.
        directory: std::path::PathBuf,
        /// Most bytes the L1 provider may occupy.
        max_bytes: u64,
    },
}

/// Configuration read once at install.
#[derive(Clone)]
pub struct RenderCacheConfig {
    /// Master switch; disabled means every request bypasses.
    pub enabled: bool,
    /// L0 bounds.
    pub l0: L0Limits,
    /// L1 provider.
    pub l1: L1Config,
    /// Provider failure behavior.
    pub failure: FailurePolicy,
    /// Application and view build identity namespace.
    pub build_id: String,
    /// Test-only clock override; `None` means `install` uses the system
    /// clock. `#[doc(hidden)]`: not part of the public contract, set only
    /// through [`Self::with_clock_for_test`].
    #[doc(hidden)]
    pub clock_override: Option<Arc<dyn Clock>>,
    /// Test-only rebuild coordinator override; `None` means `install`
    /// builds its own [`suprnova_live::render_cache::singleflight::LocalRebuildCoordinator`].
    /// `#[doc(hidden)]`: not part of the public contract, set only through
    /// [`Self::with_coordinator_for_test`] - needed to observe singleflight
    /// admission (a waiter actually parked) from a test without a
    /// timing-based wait.
    #[doc(hidden)]
    pub coordinator_override: Option<Arc<dyn RebuildCoordinator>>,
}

/// Manual, not derived: `clock_override` is `Option<Arc<dyn Clock>>`, and
/// trait objects have no `PartialEq`. Equality (used by tests that assert
/// two configs came out the same, and nowhere in production logic) compares
/// every field a real config difference could show up in; the clock
/// override is a test seam, never part of what "the same configuration"
/// means.
impl PartialEq for RenderCacheConfig {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.l0 == other.l0
            && self.l1 == other.l1
            && self.failure == other.failure
            && self.build_id == other.build_id
    }
}

impl Eq for RenderCacheConfig {}

/// Manual, not derived: `dyn Clock` has no `Debug` impl. Prints the clock
/// override as present or absent rather than omitting the field, so a test
/// failure that hinges on which clock was installed still shows that much.
impl std::fmt::Debug for RenderCacheConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RenderCacheConfig")
            .field("enabled", &self.enabled)
            .field("l0", &self.l0)
            .field("l1", &self.l1)
            .field("failure", &self.failure)
            .field("build_id", &self.build_id)
            .field("clock_override", &self.clock_override.is_some())
            .field("coordinator_override", &self.coordinator_override.is_some())
            .finish()
    }
}

impl RenderCacheConfig {
    /// Test-only: inject the clock the runtime reads instead of the system
    /// clock. `#[doc(hidden)]`: not part of the public contract.
    #[doc(hidden)]
    #[must_use]
    pub fn with_clock_for_test(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock_override = Some(clock);
        self
    }

    /// Test-only: inject the rebuild coordinator the runtime uses instead of
    /// the default `LocalRebuildCoordinator`. `#[doc(hidden)]`: not part of
    /// the public contract.
    #[doc(hidden)]
    #[must_use]
    pub fn with_coordinator_for_test(mut self, coordinator: Arc<dyn RebuildCoordinator>) -> Self {
        self.coordinator_override = Some(coordinator);
        self
    }

    /// Reads `RENDER_CACHE_ENABLED` (default `true`), `RENDER_CACHE_L0_ENTRIES`
    /// (default 4096), `RENDER_CACHE_L0_BYTES` (default 128 MiB),
    /// `RENDER_CACHE_L1_DIR` (unset disables L1), `RENDER_CACHE_L1_BYTES`
    /// (default 1 GiB), `RENDER_CACHE_FAILURE` (`open` default or `closed`),
    /// and `APP_BUILD_ID` (default `CARGO_PKG_VERSION` of the application).
    #[must_use]
    pub fn from_env() -> Self {
        let read = |name: &str| std::env::var(name).ok();
        let parse =
            |name: &str, default: u64| read(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        Self {
            enabled: read("RENDER_CACHE_ENABLED").is_none_or(|v| v != "false" && v != "0"),
            l0: L0Limits {
                max_entries: parse("RENDER_CACHE_L0_ENTRIES", 4_096) as usize,
                max_bytes: parse("RENDER_CACHE_L0_BYTES", 128 * 1024 * 1024) as usize,
            },
            l1: match read("RENDER_CACHE_L1_DIR") {
                Some(dir) if !dir.is_empty() => L1Config::File {
                    directory: dir.into(),
                    max_bytes: parse("RENDER_CACHE_L1_BYTES", 1024 * 1024 * 1024),
                },
                _ => L1Config::Disabled,
            },
            failure: if read("RENDER_CACHE_FAILURE").as_deref() == Some("closed") {
                FailurePolicy::Closed
            } else {
                FailurePolicy::Open
            },
            build_id: read("APP_BUILD_ID").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
            clock_override: None,
            coordinator_override: None,
        }
    }
}
