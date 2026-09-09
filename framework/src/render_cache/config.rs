//! Process-wide RenderCache configuration.
//!
//! # Profiles
//!
//! A [`Profile`] is a deployment shape, not a feature switch: it names which
//! providers this process should use for the two things a RenderCache
//! deployment has to agree on across nodes - where published entries live
//! ([`L1Config`]) and who is allowed to rebuild an entry
//! ([`CoordinatorConfig`]). Selecting a profile sets both; every individual
//! knob below can then override either one, so a deployment that wants its
//! entries in the database but its rebuild leases in process says exactly
//! that rather than choosing the nearest whole profile.

use std::sync::Arc;

use suprnova_live::clock::Clock;
pub use suprnova_live::render_cache::FailurePolicy;
use suprnova_live::render_cache::singleflight::RebuildCoordinator;

use super::providers::redis::REDACTED_URL;
use crate::FrameworkError;

/// The rebuild lease lifetime a profile takes when nothing overrides it.
const DEFAULT_LEASE_MS: u64 = 30_000;
/// The in-process waiter ceiling a profile takes when nothing overrides it.
const DEFAULT_MAX_WAITERS: usize = 128;
/// The L1 byte budget a profile takes when nothing overrides it.
const DEFAULT_L1_BYTES: u64 = 1024 * 1024 * 1024;
/// Where a Redis tier connects when neither `RENDER_CACHE_REDIS_URL` nor
/// `REDIS_URL` names an endpoint.
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";
/// The key namespace a Redis tier writes under when nothing overrides it.
const DEFAULT_REDIS_PREFIX: &str = "suprnova_render:";

/// In-process store bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct L0Limits {
    /// Most entries.
    pub max_entries: usize,
    /// Most bytes.
    pub max_bytes: usize,
}

/// The deployment shape this process runs under.
///
/// It selects the defaults for [`RenderCacheConfig::l1`] and
/// [`RenderCacheConfig::coordinator`] together, because those two decisions
/// are what a deployment has to make consistently: entries published where
/// every node can read them are worth little if two nodes still rebuild the
/// same key at once, and a shared rebuild lease is worth little if each node
/// then publishes into a store only it can see.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    /// One process, its own store, its own leases. The shape every
    /// application has today: L1 is the file tier when a directory is
    /// configured and nothing otherwise, and rebuilds are led in process.
    Embedded,
    /// The database every node already shares carries both the published
    /// entries and the rebuild leases. No second system to operate.
    Database,
    /// Redis carries both, as an accelerator in front of the database
    /// authority. Losing Redis loses cached bytes, never truth: the
    /// coherence check against the database generation ledger still runs on
    /// every hit.
    Redis,
}

/// The L1 provider.
#[derive(Clone, Eq, PartialEq)]
pub enum L1Config {
    /// No L1.
    Disabled,
    /// Atomic files under a directory the process owns.
    File {
        /// Directory the L1 provider owns.
        directory: std::path::PathBuf,
        /// Most bytes the L1 provider may occupy. This is the one tier whose
        /// budget bounds the *whole* store: the directory belongs to this
        /// process, so it is the process's to keep within a size.
        max_bytes: u64,
    },
    /// One row per key in `suprnova_render_entries`, shared by every node
    /// pointed at the same database.
    Database {
        /// Most bytes *one entry* may occupy, checked before any statement
        /// runs. It does not bound the table: the rows are shared by every
        /// node, so no single process has an accurate picture of the whole,
        /// and the table's growth is bounded by retention and
        /// [`RenderCache::sweep`](super::RenderCache::sweep) instead.
        max_bytes: u64,
    },
    /// One hash per key in Redis, shared by every node pointed at the same
    /// instance and prefix.
    Redis {
        /// Where the entries are stored. It can carry a password, so it is
        /// never printed: see this type's [`std::fmt::Debug`] implementation.
        url: String,
        /// The literal string every key this deployment writes begins with.
        prefix: String,
        /// Most bytes *one entry* may occupy, checked before any command is
        /// built. It does not bound the keyspace, for the reason
        /// [`Self::Database`]'s own bound does not bound the table; Redis
        /// reclaims an entry's bytes on its own when its retention elapses.
        max_bytes: u64,
    },
}

/// Manual, not derived: the [`Self::Redis`] endpoint can carry a password,
/// and a configuration reaches logs and boot errors. Everything else prints
/// as it is.
impl std::fmt::Debug for L1Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Disabled"),
            Self::File {
                directory,
                max_bytes,
            } => formatter
                .debug_struct("File")
                .field("directory", directory)
                .field("max_bytes", max_bytes)
                .finish(),
            Self::Database { max_bytes } => formatter
                .debug_struct("Database")
                .field("max_bytes", max_bytes)
                .finish(),
            Self::Redis {
                url: _,
                prefix,
                max_bytes,
            } => formatter
                .debug_struct("Redis")
                .field("url", &REDACTED_URL)
                .field("prefix", prefix)
                .field("max_bytes", max_bytes)
                .finish(),
        }
    }
}

/// Who is allowed to rebuild an entry, and for how long.
///
/// `lease_ms` and `max_waiters` mean the same thing in every variant: how
/// long one rebuild may hold the key before a peer may take it over, and how
/// many in-process requests may park behind the leader rather than render
/// their own copy. The variants differ only in where the leadership decision
/// is made.
#[derive(Clone, Eq, PartialEq)]
pub enum CoordinatorConfig {
    /// Leadership is decided in this process alone. Two nodes may rebuild
    /// the same key at once, which the contract permits: bounded duplicate
    /// computation is allowed, two accepted publications are not.
    Local {
        /// How long one rebuild may hold a key.
        lease_ms: u64,
        /// How many in-process requests may park behind the leader.
        max_waiters: usize,
    },
    /// Leadership is decided by a row in `suprnova_render_leases`, measured
    /// by the database's own clock, and every publication carries a token
    /// that row minted.
    Database {
        /// How long one rebuild may hold a key.
        lease_ms: u64,
        /// How many in-process requests may park behind the leader.
        max_waiters: usize,
    },
    /// Leadership is decided by a Redis key, measured by Redis's own clock,
    /// with the same fencing token discipline.
    Redis {
        /// Where the leases are kept. It can carry a password, so it is
        /// never printed: see this type's [`std::fmt::Debug`] implementation.
        url: String,
        /// The literal string every key this deployment writes begins with.
        prefix: String,
        /// How long one rebuild may hold a key.
        lease_ms: u64,
        /// How many in-process requests may park behind the leader.
        max_waiters: usize,
    },
}

/// Manual, not derived, for the reason [`L1Config`]'s own is.
impl std::fmt::Debug for CoordinatorConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local {
                lease_ms,
                max_waiters,
            } => formatter
                .debug_struct("Local")
                .field("lease_ms", lease_ms)
                .field("max_waiters", max_waiters)
                .finish(),
            Self::Database {
                lease_ms,
                max_waiters,
            } => formatter
                .debug_struct("Database")
                .field("lease_ms", lease_ms)
                .field("max_waiters", max_waiters)
                .finish(),
            Self::Redis {
                url: _,
                prefix,
                lease_ms,
                max_waiters,
            } => formatter
                .debug_struct("Redis")
                .field("url", &REDACTED_URL)
                .field("prefix", prefix)
                .field("lease_ms", lease_ms)
                .field("max_waiters", max_waiters)
                .finish(),
        }
    }
}

/// Configuration read once at install.
#[derive(Clone)]
pub struct RenderCacheConfig {
    /// Master switch; disabled means every request bypasses.
    pub enabled: bool,
    /// The deployment shape the providers below were defaulted from.
    /// Recorded rather than inferred: an operator reading a boot log needs
    /// to see which profile produced these providers, and a profile whose
    /// knobs were individually overridden is no longer readable from them.
    pub profile: Profile,
    /// L0 bounds.
    pub l0: L0Limits,
    /// L1 provider.
    pub l1: L1Config,
    /// Who leads a rebuild.
    pub coordinator: CoordinatorConfig,
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
    /// builds the one [`Self::coordinator`] describes.
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
            && self.profile == other.profile
            && self.l0 == other.l0
            && self.l1 == other.l1
            && self.coordinator == other.coordinator
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
            .field("profile", &self.profile)
            .field("l0", &self.l0)
            .field("l1", &self.l1)
            .field("coordinator", &self.coordinator)
            .field("failure", &self.failure)
            .field("build_id", &self.build_id)
            .field("clock_override", &self.clock_override.is_some())
            .field("coordinator_override", &self.coordinator_override.is_some())
            .finish()
    }
}

/// A configuration value outside the closed set its variable accepts.
///
/// The rejected value is deliberately absent from the message. One of these
/// variables names a Redis endpoint that routinely carries a password, and a
/// boot error reaches logs; naming the variable and listing what it accepts
/// is what an operator needs to fix it, and echoing the value adds nothing
/// to that while risking a secret in a log line.
fn unknown_value(variable: &str, accepted: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "RenderCacheConfig::from_env: {variable} is set to a value that is not one of \
         {accepted}. The rejected value is not repeated here: an environment value can carry \
         a secret. Set {variable} to one of the accepted values, or unset it to take the \
         profile's default."
    ))
}

impl RenderCacheConfig {
    /// Test-only: inject the clock the runtime reads instead of the system
    /// clock. `#[doc(hidden)]`: not part of the public contract.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_clock_for_test(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock_override = Some(clock);
        self
    }

    /// Test-only: inject the rebuild coordinator the runtime uses instead of
    /// the one [`Self::coordinator`] describes. `#[doc(hidden)]`: not part
    /// of the public contract.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_coordinator_for_test(mut self, coordinator: Arc<dyn RebuildCoordinator>) -> Self {
        self.coordinator_override = Some(coordinator);
        self
    }

    /// Reads the process environment.
    ///
    /// `RENDER_CACHE_ENABLED` (default `true`), `RENDER_CACHE_L0_ENTRIES`
    /// (default 4096), `RENDER_CACHE_L0_BYTES` (default 128 MiB),
    /// `RENDER_CACHE_FAILURE` (`open` default or `closed`), and
    /// `APP_BUILD_ID` keep the meaning they have always had.
    ///
    /// `APP_BUILD_ID`'s default is `env!("CARGO_PKG_VERSION")`, which
    /// expands at compile time inside *this* crate, so the fallback is the
    /// framework crate's own version rather than the host application's. It
    /// matches the application's only where both inherit one workspace
    /// version, and either way it moves only when someone bumps a version
    /// number. A deployment should set `APP_BUILD_ID` explicitly to
    /// something that changes every release (a commit id, say): it is mixed
    /// into every lookup key, so a deploy that changes a template or a
    /// handler without a version bump otherwise keeps the previous build's
    /// entries reachable.
    ///
    /// `RENDER_CACHE_PROFILE` (`embedded` default, `database`, `redis`)
    /// selects the deployment shape and with it the defaults for the L1
    /// provider and the rebuild coordinator. Each of those can then be
    /// overridden on its own: `RENDER_CACHE_L1` (`disabled`, `file`,
    /// `database`, `redis`) and `RENDER_CACHE_COORDINATOR` (`local`,
    /// `database`, `redis`).
    ///
    /// The tiers those select read: `RENDER_CACHE_L1_DIR` (the file tier's
    /// directory; under the embedded profile, setting it is what turns L1
    /// on at all), `RENDER_CACHE_L1_BYTES` (default 1 GiB - the whole
    /// directory for the file tier, one entry for the database and Redis
    /// tiers), `RENDER_CACHE_REDIS_URL` (falling back to `REDIS_URL`, then
    /// to `redis://127.0.0.1:6379`), `RENDER_CACHE_REDIS_PREFIX` (default
    /// `suprnova_render:`), `RENDER_CACHE_LEASE_MS` (default 30000), and
    /// `RENDER_CACHE_MAX_WAITERS` (default 128).
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when a variable with a closed set of
    /// accepted values is set to something outside it, or when
    /// `RENDER_CACHE_L1=file` names no directory. The message names the
    /// variable and never repeats the rejected value.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    /// Whether `RENDER_CACHE_ENABLED` permits RenderCache in this process,
    /// read on its own.
    ///
    /// The write side's probe uses this rather than [`Self::from_env`] on
    /// purpose: a worker has no L1 tier, no coordinator, and no profile to
    /// get right, and a malformed value for any of those must not stop its
    /// writes from advancing generations. Same rule as `from_env`: enabled
    /// unless the variable is exactly `false` or `0`.
    #[must_use]
    pub fn enabled_from_env() -> bool {
        Self::enabled_from_source(&|name| std::env::var(name).ok())
    }

    /// [`Self::enabled_from_env`] over any reader, so the rule can be
    /// proven against fixed pairs rather than the process environment.
    fn enabled_from_source(read: &dyn Fn(&str) -> Option<String>) -> bool {
        read("RENDER_CACHE_ENABLED").is_none_or(|value| value != "false" && value != "0")
    }

    /// [`Self::from_env`] over any reader, so the parser can be proven
    /// against fixed pairs rather than against the process environment.
    ///
    /// The process environment is shared by every test in a binary; a table
    /// is not, which is what keeps these tests order-independent.
    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let non_empty = |name: &str| read(name).filter(|value| !value.is_empty());
        let parse = |name: &str, default: u64| {
            non_empty(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };

        let profile = match non_empty("RENDER_CACHE_PROFILE").as_deref() {
            None | Some("embedded") => Profile::Embedded,
            Some("database") => Profile::Database,
            Some("redis") => Profile::Redis,
            Some(_) => {
                return Err(unknown_value(
                    "RENDER_CACHE_PROFILE",
                    "embedded, database, or redis",
                ));
            }
        };

        let l1_bytes = parse("RENDER_CACHE_L1_BYTES", DEFAULT_L1_BYTES);
        let lease_ms = parse("RENDER_CACHE_LEASE_MS", DEFAULT_LEASE_MS);
        let max_waiters = usize::try_from(parse(
            "RENDER_CACHE_MAX_WAITERS",
            DEFAULT_MAX_WAITERS as u64,
        ))
        .unwrap_or(DEFAULT_MAX_WAITERS);
        let url = non_empty("RENDER_CACHE_REDIS_URL")
            .or_else(|| non_empty("REDIS_URL"))
            .unwrap_or_else(|| DEFAULT_REDIS_URL.to_owned());
        let prefix = non_empty("RENDER_CACHE_REDIS_PREFIX")
            .unwrap_or_else(|| DEFAULT_REDIS_PREFIX.to_owned());

        // The file tier is the one that has always been selected by the
        // presence of its directory rather than by a tier name, and it stays
        // that way: an embedded deployment that sets `RENDER_CACHE_L1_DIR`
        // gets the file tier exactly as it always has.
        let file_tier = |required: bool| match non_empty("RENDER_CACHE_L1_DIR") {
            Some(directory) => Ok(L1Config::File {
                directory: directory.into(),
                max_bytes: l1_bytes,
            }),
            None if required => Err(FrameworkError::internal(
                "RenderCacheConfig::from_env: RENDER_CACHE_L1=file names no directory. Set \
                 RENDER_CACHE_L1_DIR to a directory this process owns, or select another L1 \
                 tier with RENDER_CACHE_L1.",
            )),
            None => Ok(L1Config::Disabled),
        };

        let l1 = match non_empty("RENDER_CACHE_L1").as_deref() {
            Some("disabled") => L1Config::Disabled,
            Some("file") => file_tier(true)?,
            Some("database") => L1Config::Database {
                max_bytes: l1_bytes,
            },
            Some("redis") => L1Config::Redis {
                url: url.clone(),
                prefix: prefix.clone(),
                max_bytes: l1_bytes,
            },
            Some(_) => {
                return Err(unknown_value(
                    "RENDER_CACHE_L1",
                    "disabled, file, database, or redis",
                ));
            }
            None => match profile {
                Profile::Embedded => file_tier(false)?,
                Profile::Database => L1Config::Database {
                    max_bytes: l1_bytes,
                },
                Profile::Redis => L1Config::Redis {
                    url: url.clone(),
                    prefix: prefix.clone(),
                    max_bytes: l1_bytes,
                },
            },
        };

        let local = CoordinatorConfig::Local {
            lease_ms,
            max_waiters,
        };
        let database = CoordinatorConfig::Database {
            lease_ms,
            max_waiters,
        };
        let redis = || CoordinatorConfig::Redis {
            url: url.clone(),
            prefix: prefix.clone(),
            lease_ms,
            max_waiters,
        };
        let coordinator = match non_empty("RENDER_CACHE_COORDINATOR").as_deref() {
            Some("local") => local,
            Some("database") => database,
            Some("redis") => redis(),
            Some(_) => {
                return Err(unknown_value(
                    "RENDER_CACHE_COORDINATOR",
                    "local, database, or redis",
                ));
            }
            None => match profile {
                Profile::Embedded => local,
                Profile::Database => database,
                Profile::Redis => redis(),
            },
        };

        Ok(Self {
            enabled: read("RENDER_CACHE_ENABLED").is_none_or(|v| v != "false" && v != "0"),
            profile,
            l0: L0Limits {
                max_entries: parse("RENDER_CACHE_L0_ENTRIES", 4_096) as usize,
                max_bytes: parse("RENDER_CACHE_L0_BYTES", 128 * 1024 * 1024) as usize,
            },
            l1,
            coordinator,
            failure: if read("RENDER_CACHE_FAILURE").as_deref() == Some("closed") {
                FailurePolicy::Closed
            } else {
                FailurePolicy::Open
            },
            build_id: read("APP_BUILD_ID").unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned()),
            clock_override: None,
            coordinator_override: None,
        })
    }
}

#[cfg(test)]
mod tests {
    //! Profile defaults, individual overrides, and what a rejected value is
    //! allowed to say.
    //!
    //! Every test here reads a map rather than the process environment.
    //! `from_env` is one line over [`RenderCacheConfig::from_source`], so a
    //! map proves the same parser without a process-wide variable that a
    //! neighbouring test could see - which is what makes these tests
    //! order-independent under plain `cargo test`.
    use super::*;

    /// A reader over fixed pairs, standing in for the process environment.
    fn source(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn parsed(pairs: &[(&str, &str)]) -> RenderCacheConfig {
        let table = source(pairs);
        RenderCacheConfig::from_source(&|name| table.get(name).cloned())
            .expect("the fixture configuration parses")
    }

    fn refusal(pairs: &[(&str, &str)]) -> String {
        let table = source(pairs);
        RenderCacheConfig::from_source(&|name| table.get(name).cloned())
            .expect_err("the fixture configuration is refused")
            .to_string()
    }

    #[test]
    fn the_embedded_profile_is_the_default_and_keeps_the_shape_it_always_had() {
        let config = parsed(&[]);
        assert_eq!(config.profile, Profile::Embedded);
        assert_eq!(config.l1, L1Config::Disabled);
        assert_eq!(
            config.coordinator,
            CoordinatorConfig::Local {
                lease_ms: 30_000,
                max_waiters: 128,
            }
        );

        // And a directory still turns the file tier on, with the same
        // byte budget it has always defaulted to.
        let config = parsed(&[("RENDER_CACHE_L1_DIR", "/var/cache/suprnova")]);
        assert_eq!(
            config.l1,
            L1Config::File {
                directory: std::path::PathBuf::from("/var/cache/suprnova"),
                max_bytes: 1024 * 1024 * 1024,
            }
        );
    }

    #[test]
    fn the_database_profile_selects_the_database_tier_for_both() {
        let config = parsed(&[("RENDER_CACHE_PROFILE", "database")]);
        assert_eq!(config.profile, Profile::Database);
        assert_eq!(
            config.l1,
            L1Config::Database {
                max_bytes: 1024 * 1024 * 1024
            }
        );
        assert_eq!(
            config.coordinator,
            CoordinatorConfig::Database {
                lease_ms: 30_000,
                max_waiters: 128,
            }
        );
    }

    #[test]
    fn the_redis_profile_selects_the_redis_tier_for_both_with_one_endpoint() {
        let config = parsed(&[("RENDER_CACHE_PROFILE", "redis")]);
        assert_eq!(config.profile, Profile::Redis);
        assert_eq!(
            config.l1,
            L1Config::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_render:".to_owned(),
                max_bytes: 1024 * 1024 * 1024,
            }
        );
        assert_eq!(
            config.coordinator,
            CoordinatorConfig::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_render:".to_owned(),
                lease_ms: 30_000,
                max_waiters: 128,
            }
        );
    }

    #[test]
    fn the_render_cache_endpoint_falls_back_to_the_shared_one_and_then_to_loopback() {
        let shared = parsed(&[
            ("RENDER_CACHE_PROFILE", "redis"),
            ("REDIS_URL", "redis://shared:6379"),
        ]);
        assert_eq!(
            shared.l1,
            L1Config::Redis {
                url: "redis://shared:6379".to_owned(),
                prefix: "suprnova_render:".to_owned(),
                max_bytes: 1024 * 1024 * 1024,
            }
        );

        // The RenderCache's own variable wins over the shared one.
        let own = parsed(&[
            ("RENDER_CACHE_PROFILE", "redis"),
            ("REDIS_URL", "redis://shared:6379"),
            ("RENDER_CACHE_REDIS_URL", "redis://cache:6379"),
            ("RENDER_CACHE_REDIS_PREFIX", "tenant_a:"),
        ]);
        assert_eq!(
            own.l1,
            L1Config::Redis {
                url: "redis://cache:6379".to_owned(),
                prefix: "tenant_a:".to_owned(),
                max_bytes: 1024 * 1024 * 1024,
            }
        );
    }

    #[test]
    fn one_explicit_knob_overrides_the_profile_default_and_leaves_the_rest() {
        // A database profile that keeps its rows but leads rebuilds in
        // process, which is what a single-node deployment on a shared
        // database wants.
        let config = parsed(&[
            ("RENDER_CACHE_PROFILE", "database"),
            ("RENDER_CACHE_COORDINATOR", "local"),
            ("RENDER_CACHE_LEASE_MS", "5000"),
            ("RENDER_CACHE_MAX_WAITERS", "8"),
        ]);
        assert_eq!(config.profile, Profile::Database);
        assert_eq!(
            config.l1,
            L1Config::Database {
                max_bytes: 1024 * 1024 * 1024
            },
            "the L1 tier is still the profile's"
        );
        assert_eq!(
            config.coordinator,
            CoordinatorConfig::Local {
                lease_ms: 5_000,
                max_waiters: 8,
            }
        );

        // And the other direction: an embedded profile that shares one
        // Redis lease across nodes.
        let config = parsed(&[
            ("RENDER_CACHE_L1_DIR", "/var/cache/suprnova"),
            ("RENDER_CACHE_COORDINATOR", "redis"),
        ]);
        assert_eq!(config.profile, Profile::Embedded);
        assert!(matches!(config.l1, L1Config::File { .. }));
        assert_eq!(
            config.coordinator,
            CoordinatorConfig::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_render:".to_owned(),
                lease_ms: 30_000,
                max_waiters: 128,
            }
        );
    }

    #[test]
    fn every_l1_tier_can_be_selected_on_its_own() {
        assert_eq!(
            parsed(&[
                ("RENDER_CACHE_PROFILE", "database"),
                ("RENDER_CACHE_L1", "disabled")
            ])
            .l1,
            L1Config::Disabled
        );
        assert_eq!(
            parsed(&[
                ("RENDER_CACHE_L1", "file"),
                ("RENDER_CACHE_L1_DIR", "/tmp/x"),
                ("RENDER_CACHE_L1_BYTES", "4096"),
            ])
            .l1,
            L1Config::File {
                directory: std::path::PathBuf::from("/tmp/x"),
                max_bytes: 4096,
            }
        );
        assert_eq!(
            parsed(&[
                ("RENDER_CACHE_L1", "database"),
                ("RENDER_CACHE_L1_BYTES", "4096")
            ])
            .l1,
            L1Config::Database { max_bytes: 4096 }
        );
        assert_eq!(
            parsed(&[
                ("RENDER_CACHE_L1", "redis"),
                ("RENDER_CACHE_L1_BYTES", "4096")
            ])
            .l1,
            L1Config::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_render:".to_owned(),
                max_bytes: 4096,
            }
        );
    }

    #[test]
    fn an_unknown_value_names_the_variable_and_repeats_nothing() {
        for (variable, pairs) in [
            (
                "RENDER_CACHE_PROFILE",
                vec![("RENDER_CACHE_PROFILE", "postgres-ish")],
            ),
            ("RENDER_CACHE_L1", vec![("RENDER_CACHE_L1", "memcached")]),
            (
                "RENDER_CACHE_COORDINATOR",
                vec![("RENDER_CACHE_COORDINATOR", "zookeeper")],
            ),
        ] {
            let message = refusal(&pairs);
            assert!(message.contains(variable), "{message}");
            assert!(
                !message.contains(pairs[0].1),
                "the rejected value is never repeated: {message}"
            );
        }
    }

    #[test]
    fn the_file_tier_without_a_directory_is_refused_rather_than_silently_disabled() {
        let message = refusal(&[("RENDER_CACHE_L1", "file")]);
        assert!(message.contains("RENDER_CACHE_L1_DIR"), "{message}");
    }

    #[test]
    fn a_redis_endpoint_never_reaches_a_printed_configuration() {
        let config = parsed(&[
            ("RENDER_CACHE_PROFILE", "redis"),
            (
                "RENDER_CACHE_REDIS_URL",
                "redis://someone:hunter2@10.0.0.1:6379",
            ),
        ]);
        let printed = format!("{config:?} {:?} {:?}", config.l1, config.coordinator);
        assert!(printed.contains("redis://<redacted>"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("10.0.0.1"), "{printed}");
    }
}
