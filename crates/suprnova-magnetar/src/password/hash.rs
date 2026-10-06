//! One installable password-verification service, two deployed formats.
//!
//! Adapted from the behavior of torii's `password_auth` lane (Argon2id) and
//! the Suprnova framework hasher (bcrypt, cost 12, 71-byte usable limit).
//! Every attempt performs exactly one bcrypt-format call and one
//! Argon2-format call through the installable [`PasswordHashDriver`]: the
//! stored hash is driven in its own format and a warmed dummy stands in for
//! the other, so neither account existence nor the stored format is
//! observable through hash work. The migration target is Argon2id by
//! default, and rehash is then upgrade-only; an application that shares its
//! database with a Laravel application pins it to Laravel's `$2y$` bcrypt
//! instead ([`PasswordTarget::LaravelBcrypt`]). A rehash failure is a
//! post-login outcome, never an authentication failure.

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::{Error, Result};

/// The longest password a `$2b$` hash the framework's bcrypt driver wrote
/// can hold: bcrypt needs a trailing null inside its 72-byte block.
///
/// Verification does not stop at it. Bcrypt judges a password on its first
/// 72 bytes, as PHP's `password_verify` does, so a password of 72 bytes or
/// more verifies against the `$2y$` hash Laravel wrote for it, and cannot
/// match a `$2b$` hash of 71 bytes or fewer.
pub const MAX_BCRYPT_PASSWORD_BYTES: usize = 71;

/// What a credential is minted as, and what a valid sign-in rehashes to.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PasswordTarget {
    /// Argon2id at the configured profile. A bcrypt hash upgrades to it on
    /// a valid sign-in, and a weaker Argon2 hash is raised to it. The
    /// default.
    #[default]
    Argon2id,
    /// `$2y$` bcrypt at the configured bcrypt cost, the hash Laravel 13's
    /// hasher accepts with `HASH_VERIFY=true`. A valid sign-in rewrites any
    /// other stored hash (`$2b$`, `$2a$`, Argon2) as one, and nothing is
    /// upgraded to Argon2id. For an application that shares its database
    /// with a Laravel application.
    LaravelBcrypt,
}

/// The two deployed hash formats.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HashAlgorithm {
    /// Framework-lane bcrypt.
    Bcrypt,
    /// Torii-lane Argon2id (and legacy Argon2 variants at rest).
    Argon2,
}

/// Cost parameters attached to one hash-work call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HashParameters {
    /// Bcrypt cost factor.
    Bcrypt {
        /// Bcrypt cost (log2 rounds).
        cost: u32,
    },
    /// Argon2 memory/time/lanes.
    Argon2 {
        /// Memory in KiB.
        memory_kib: u32,
        /// Iteration count.
        iterations: u32,
        /// Parallelism lanes.
        parallelism: u32,
    },
}

/// The algorithm and parameter profile of one hash-work call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HashWorkProfile {
    /// Hash format.
    pub algorithm: HashAlgorithm,
    /// Cost parameters.
    pub parameters: HashParameters,
}

/// Whether a verification call drives the stored credential or a dummy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallProvenance {
    /// The stored hash for the attempted account.
    Stored,
    /// The warmed dummy for the format the account does not use.
    Dummy,
}

/// One verification unit of work handed to the driver.
pub struct VerificationCall<'a> {
    /// Stored-versus-dummy provenance, exposed for deterministic spy tests.
    pub provenance: CallProvenance,
    /// Profile of the hash being driven.
    pub profile: HashWorkProfile,
    /// Candidate password.
    pub password: &'a SecretString,
    /// Hash driven by this call.
    pub hash: &'a str,
}

/// Installable hash driver. Production uses [`StandardPasswordHashDriver`];
/// tests install counting spies to pin work equivalence without wall clocks.
pub trait PasswordHashDriver: Send + Sync {
    /// Perform one verification call.
    fn verify(&self, call: &VerificationCall<'_>) -> Result<bool>;
    /// Mint a hash under an explicit profile.
    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> Result<String>;
}

/// Deployed-format profiles and the pinned migration target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasswordHashConfig {
    /// Deployed bcrypt cost (framework default 12). Used for the bcrypt
    /// dummy profile, and as the mint cost under
    /// [`PasswordTarget::LaravelBcrypt`].
    pub bcrypt_cost: u32,
    /// Deployed and target Argon2id memory in KiB.
    pub argon2_memory_kib: u32,
    /// Deployed and target Argon2id iterations.
    pub argon2_iterations: u32,
    /// Deployed and target Argon2id parallelism.
    pub argon2_parallelism: u32,
}

impl Default for PasswordHashConfig {
    /// Match the captured legacy corpus: framework bcrypt cost 12 and the
    /// `password_auth` Argon2id profile `m=19456 KiB, t=2, p=1`.
    fn default() -> Self {
        Self {
            bcrypt_cost: 12,
            argon2_memory_kib: 19_456,
            argon2_iterations: 2,
            argon2_parallelism: 1,
        }
    }
}

impl PasswordHashConfig {
    /// The deployed bcrypt work profile.
    #[must_use]
    pub const fn bcrypt_profile(&self) -> HashWorkProfile {
        HashWorkProfile {
            algorithm: HashAlgorithm::Bcrypt,
            parameters: HashParameters::Bcrypt {
                cost: self.bcrypt_cost,
            },
        }
    }

    /// The pinned Argon2id migration target profile.
    #[must_use]
    pub const fn argon2_target(&self) -> HashWorkProfile {
        HashWorkProfile {
            algorithm: HashAlgorithm::Argon2,
            parameters: HashParameters::Argon2 {
                memory_kib: self.argon2_memory_kib,
                iterations: self.argon2_iterations,
                parallelism: self.argon2_parallelism,
            },
        }
    }
}

/// Post-login rehash outcome. Never an authentication failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RehashOutcome {
    /// The stored hash meets the target; nothing to do.
    NotNeeded,
    /// The credential was re-hashed to the verifier's [`PasswordTarget`];
    /// callers persist this value.
    Upgraded(String),
    /// Rehash failed after a successful login; recorded, not fatal.
    Failed {
        /// Failure detail for the post-login record.
        message: String,
    },
}

/// The result of one full verification attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptVerdict {
    /// Whether the stored credential matched.
    pub valid: bool,
    /// Upgrade-only rehash outcome; only meaningful when `valid`.
    pub rehash: RehashOutcome,
}

/// Run one piece of password hash work off the async runtime's workers.
///
/// A password hash is slow on purpose: an Argon2id mint or verify holds a
/// thread for tens of milliseconds. On a runtime worker that blocks every
/// other task scheduled there, so the work runs on Tokio's blocking pool
/// when a runtime is present. Outside a runtime it runs inline, as there is
/// no worker to stall.
///
/// # Errors
///
/// The work's own error, or [`Error::Internal`] when the blocking task did
/// not complete (it panicked or the runtime is shutting down).
pub async fn run_hash_work<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => runtime
            .spawn_blocking(work)
            .await
            .map_err(|error| Error::Internal {
                message: format!("password hash work did not complete: {error}"),
            })?,
        Err(_) => work(),
    }
}

/// The highest cost bcrypt takes: a cost above it is no hash bcrypt wrote.
const MAX_BCRYPT_COST: u32 = 31;

/// The one verification service for both deployed formats.
pub struct PasswordVerifier {
    driver: Arc<dyn PasswordHashDriver>,
    config: PasswordHashConfig,
    target: PasswordTarget,
    bcrypt_dummy: String,
    argon2_dummy: String,
}

impl PasswordVerifier {
    /// Build a verifier and warm one dummy hash per deployed format.
    ///
    /// The dummies are minted through the installed driver - never
    /// hard-coded - so their cost always tracks the configured profiles
    /// (the fork's `dummy_verify` regression).
    pub fn new(driver: Arc<dyn PasswordHashDriver>, config: PasswordHashConfig) -> Result<Self> {
        let secret = SecretString::from(random_secret());
        let bcrypt_dummy = driver.mint(&config.bcrypt_profile(), &secret)?;
        let argon2_dummy = driver.mint(&config.argon2_target(), &secret)?;
        Ok(Self {
            driver,
            config,
            target: PasswordTarget::Argon2id,
            bcrypt_dummy,
            argon2_dummy,
        })
    }

    /// Mint credentials as `target`, and rehash a valid sign-in's stored
    /// hash to it. [`PasswordTarget::Argon2id`] unless set here.
    #[must_use]
    pub fn with_target(mut self, target: PasswordTarget) -> Self {
        self.target = target;
        self
    }

    /// The configured profiles, exposed for deterministic tests.
    #[must_use]
    pub const fn config(&self) -> &PasswordHashConfig {
        &self.config
    }

    /// What this verifier mints and rehashes to.
    #[must_use]
    pub const fn target(&self) -> PasswordTarget {
        self.target
    }

    /// The work profile [`Self::target`] mints with.
    fn target_profile(&self) -> HashWorkProfile {
        match self.target {
            PasswordTarget::Argon2id => self.config.argon2_target(),
            PasswordTarget::LaravelBcrypt => self.config.bcrypt_profile(),
        }
    }

    /// The work profile a valid sign-in's rewrite of a hash stored at
    /// `stored` mints with: [`Self::target_profile`], except that a rewrite
    /// to [`PasswordTarget::LaravelBcrypt`] keeps a stored bcrypt cost above
    /// the configured one. The rewrite changes the variant Laravel reads,
    /// never lowers the work, and Laravel accepts a `$2y$` hash at any cost.
    fn rehash_profile(&self, stored: &HashWorkProfile) -> HashWorkProfile {
        match (self.target, stored.parameters) {
            (PasswordTarget::LaravelBcrypt, HashParameters::Bcrypt { cost })
                if cost > self.config.bcrypt_cost =>
            {
                HashWorkProfile {
                    algorithm: HashAlgorithm::Bcrypt,
                    parameters: HashParameters::Bcrypt {
                        cost: cost.min(MAX_BCRYPT_COST),
                    },
                }
            }
            _ => self.target_profile(),
        }
    }

    /// Verify one attempt with fixed-format work and compute any required
    /// upgrade.
    ///
    /// Exactly one bcrypt-format call and one Argon2-format call run through
    /// the driver regardless of whether the account exists, stores a
    /// password, or stores it in either format.
    pub fn verify_attempt(
        &self,
        stored_hash: Option<&str>,
        password: &SecretString,
    ) -> Result<AttemptVerdict> {
        let (valid, stored_profile) = self.verify_fixed_work(stored_hash, password)?;
        let rehash = if valid {
            match stored_profile {
                Some(profile) if self.needs_rehash(&profile, stored_hash.unwrap_or_default()) => {
                    match self.driver.mint(&self.rehash_profile(&profile), password) {
                        Ok(upgraded) => RehashOutcome::Upgraded(upgraded),
                        Err(error) => RehashOutcome::Failed {
                            message: error.to_string(),
                        },
                    }
                }
                _ => RehashOutcome::NotNeeded,
            }
        } else {
            RehashOutcome::NotNeeded
        };
        Ok(AttemptVerdict { valid, rehash })
    }

    /// Execute only the fixed-format verification work. This deliberately
    /// omits upgrade minting and exposes neither the credential verdict nor a
    /// principal-producing side effect.
    pub fn verify_work_only(
        &self,
        stored_hash: Option<&str>,
        password: &SecretString,
    ) -> Result<()> {
        let _ = self.verify_fixed_work(stored_hash, password)?;
        Ok(())
    }

    fn verify_fixed_work(
        &self,
        stored_hash: Option<&str>,
        password: &SecretString,
    ) -> Result<(bool, Option<HashWorkProfile>)> {
        let stored = stored_hash.filter(|hash| !hash.is_empty());
        let classified = stored.and_then(|hash| classify(hash).map(|profile| (hash, profile)));
        let stored_profile = classified.as_ref().map(|(_, profile)| *profile);

        let (bcrypt_call, argon2_call) = match &classified {
            Some((hash, profile)) if profile.algorithm == HashAlgorithm::Bcrypt => (
                (CallProvenance::Stored, *profile, *hash),
                (
                    CallProvenance::Dummy,
                    self.config.argon2_target(),
                    self.argon2_dummy.as_str(),
                ),
            ),
            Some((hash, profile)) => (
                (
                    CallProvenance::Dummy,
                    self.config.bcrypt_profile(),
                    self.bcrypt_dummy.as_str(),
                ),
                (CallProvenance::Stored, *profile, *hash),
            ),
            None => (
                (
                    CallProvenance::Dummy,
                    self.config.bcrypt_profile(),
                    self.bcrypt_dummy.as_str(),
                ),
                (
                    CallProvenance::Dummy,
                    self.config.argon2_target(),
                    self.argon2_dummy.as_str(),
                ),
            ),
        };

        let mut valid = false;
        for (provenance, profile, hash) in [bcrypt_call, argon2_call] {
            let matched = self.driver.verify(&VerificationCall {
                provenance,
                profile,
                password,
                hash,
            })?;
            if provenance == CallProvenance::Stored {
                valid = matched;
            }
        }
        Ok((valid, stored_profile))
    }

    /// Mint a fresh credential hash at the verifier's [`PasswordTarget`].
    pub fn mint_target(&self, password: &SecretString) -> Result<String> {
        self.driver.mint(&self.target_profile(), password)
    }

    /// [`mint_target`](Self::mint_target), run where it cannot stall the
    /// async runtime (see [`run_hash_work`]).
    pub async fn mint_target_blocking(self: &Arc<Self>, password: SecretString) -> Result<String> {
        let verifier = Arc::clone(self);
        run_hash_work(move || verifier.mint_target(&password)).await
    }

    /// [`verify_attempt`](Self::verify_attempt), run where it cannot stall
    /// the async runtime (see [`run_hash_work`]).
    pub async fn verify_attempt_blocking(
        self: &Arc<Self>,
        stored_hash: Option<String>,
        password: SecretString,
    ) -> Result<AttemptVerdict> {
        let verifier = Arc::clone(self);
        run_hash_work(move || verifier.verify_attempt(stored_hash.as_deref(), &password)).await
    }

    /// [`verify_work_only`](Self::verify_work_only), run where it cannot
    /// stall the async runtime (see [`run_hash_work`]).
    pub async fn verify_work_only_blocking(
        self: &Arc<Self>,
        stored_hash: Option<String>,
        password: SecretString,
    ) -> Result<()> {
        let verifier = Arc::clone(self);
        run_hash_work(move || verifier.verify_work_only(stored_hash.as_deref(), &password)).await
    }

    /// The rehash policy for [`Self::target`].
    ///
    /// [`PasswordTarget::Argon2id`] is upgrade-only: bcrypt always
    /// upgrades; Argon2 upgrades only when a parameter falls below the
    /// pinned target. A stronger stored Argon2 hash is never downgraded.
    ///
    /// [`PasswordTarget::LaravelBcrypt`] rewrites everything that is not a
    /// `$2y$` hash at the configured cost or higher, Argon2 included:
    /// Laravel's hasher refuses any other format.
    fn needs_rehash(&self, stored: &HashWorkProfile, stored_hash: &str) -> bool {
        if self.target == PasswordTarget::LaravelBcrypt {
            return match stored.parameters {
                HashParameters::Bcrypt { cost } => {
                    !stored_hash.starts_with("$2y$") || cost < self.config.bcrypt_cost
                }
                HashParameters::Argon2 { .. } => true,
            };
        }
        match stored.parameters {
            HashParameters::Bcrypt { .. } => true,
            HashParameters::Argon2 {
                memory_kib,
                iterations,
                parallelism,
            } => {
                memory_kib < self.config.argon2_memory_kib
                    || iterations < self.config.argon2_iterations
                    || parallelism < self.config.argon2_parallelism
            }
        }
    }
}

/// Classify a stored hash into its work profile. Unrecognized values return
/// `None` and are treated exactly like a passwordless account.
fn classify(hash: &str) -> Option<HashWorkProfile> {
    if let Some(rest) = hash.strip_prefix("$2") {
        // "$2b$12$..." - legacy $2a/$2x/$2y variants also parse here.
        let cost = rest.split('$').nth(1)?.parse().ok()?;
        return Some(HashWorkProfile {
            algorithm: HashAlgorithm::Bcrypt,
            parameters: HashParameters::Bcrypt { cost },
        });
    }
    if hash.starts_with("$argon2") {
        // Parse the PHC segments textually ("$argon2id$v=19$m=..,t=..,p=..$…"),
        // matching the deployed framework's format inspection: profile
        // classification must not depend on salt/output validity, only the
        // recorded parameters.
        let mut segments = hash.split('$');
        let _empty = segments.next()?;
        let _algorithm = segments.next()?;
        let mut params_segment = segments.next()?;
        if params_segment.starts_with("v=") {
            params_segment = segments.next()?;
        }
        let mut memory_kib = None;
        let mut iterations = None;
        let mut parallelism = None;
        for pair in params_segment.split(',') {
            let (name, value) = pair.split_once('=')?;
            let value = value.parse().ok()?;
            match name {
                "m" => memory_kib = Some(value),
                "t" => iterations = Some(value),
                "p" => parallelism = Some(value),
                _ => {}
            }
        }
        return Some(HashWorkProfile {
            algorithm: HashAlgorithm::Argon2,
            parameters: HashParameters::Argon2 {
                memory_kib: memory_kib?,
                iterations: iterations?,
                parallelism: parallelism?,
            },
        });
    }
    None
}

/// The production driver: real bcrypt and Argon2 work.
#[derive(Clone, Copy, Debug, Default)]
pub struct StandardPasswordHashDriver;

impl PasswordHashDriver for StandardPasswordHashDriver {
    fn verify(&self, call: &VerificationCall<'_>) -> Result<bool> {
        match call.profile.algorithm {
            // Every length verifies, judged on its first 72 bytes as PHP's
            // `password_verify` judges it (see `MAX_BCRYPT_PASSWORD_BYTES`).
            HashAlgorithm::Bcrypt => bcrypt::verify(call.password.expose_secret(), call.hash)
                .map_err(|_| malformed_hash()),
            HashAlgorithm::Argon2 => {
                use argon2::PasswordVerifier as _;
                let parsed = argon2::password_hash::PasswordHash::new(call.hash)
                    .map_err(|_| malformed_hash())?;
                Ok(argon2::Argon2::default()
                    .verify_password(call.password.expose_secret().as_bytes(), &parsed)
                    .is_ok())
            }
        }
    }

    fn mint(&self, profile: &HashWorkProfile, password: &SecretString) -> Result<String> {
        match profile.parameters {
            // `$2y$`, judged on the first 72 bytes, as PHP's
            // `password_hash` writes it: the form Laravel's hasher accepts,
            // and the one `PasswordTarget::LaravelBcrypt` mints. The bcrypt
            // dummy is minted the same way.
            HashParameters::Bcrypt { cost } => {
                bcrypt::hash_with_result(password.expose_secret(), cost)
                    .map(|parts| parts.format_for_version(bcrypt::Version::TwoY))
                    .map_err(|error| Error::Internal {
                        message: format!("bcrypt hashing failed: {error}"),
                    })
            }
            HashParameters::Argon2 {
                memory_kib,
                iterations,
                parallelism,
            } => {
                use argon2::PasswordHasher as _;
                let params = argon2::Params::new(memory_kib, iterations, parallelism, None)
                    .map_err(|error| Error::InvalidInput {
                        field: "argon2".to_owned(),
                        message: error.to_string(),
                    })?;
                let hasher = argon2::Argon2::new(
                    argon2::Algorithm::Argon2id,
                    argon2::Version::V0x13,
                    params,
                );
                let salt = argon2::password_hash::SaltString::generate(
                    &mut argon2::password_hash::rand_core::OsRng,
                );
                hasher
                    .hash_password(password.expose_secret().as_bytes(), &salt)
                    .map(|hash| hash.to_string())
                    .map_err(|error| Error::Internal {
                        message: format!("argon2 hashing failed: {error}"),
                    })
            }
        }
    }
}

fn malformed_hash() -> Error {
    Error::Internal {
        message: "stored password hash is malformed".to_owned(),
    }
}

fn random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    crate::storage::hex_lower(&bytes)
}
