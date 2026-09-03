//! Process-wide RenderCache configuration.

pub use suprnova_live::render_cache::FailurePolicy;

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
#[derive(Clone, Debug, Eq, PartialEq)]
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
}

impl RenderCacheConfig {
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
        }
    }
}
