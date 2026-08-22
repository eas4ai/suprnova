//! One installable password-verification service, two deployed formats.
//!
//! Adapted from the behavior of torii's `password_auth` lane (Argon2id) and
//! the Suprnova framework hasher (bcrypt, cost 12, 71-byte usable limit).
//! Every attempt performs exactly one bcrypt-format call and one
//! Argon2-format call through the installable [`PasswordHashDriver`]: the
//! stored hash is driven in its own format and a warmed dummy stands in for
//! the other, so neither account existence nor the stored format is
//! observable through hash work. The migration target is pinned to Argon2id;
//! rehash is upgrade-only and a rehash failure is a post-login outcome, never
//! an authentication failure.

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::{Error, Result};

/// Maximum usable password bytes for the deployed bcrypt lane.
///
/// Bcrypt needs a trailing null inside its 72-byte block, so 71 bytes is the
/// usable ceiling. Longer inputs can never match a Magnetar-accepted bcrypt
/// hash; verification reports a mismatch instead of an error so length leaks
/// nothing, exactly as the deployed framework behaves.
pub const MAX_BCRYPT_PASSWORD_BYTES: usize = 71;

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
    /// dummy profile only; bcrypt is never a mint target.
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
    /// The credential was re-hashed to the Argon2id target; callers persist
    /// this value.
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

/// The one verification service for both deployed formats.
pub struct PasswordVerifier {
    driver: Arc<dyn PasswordHashDriver>,
    config: PasswordHashConfig,
    bcrypt_dummy: String,
    argon2_dummy: String,
}

impl PasswordVerifier {
    /// Build a verifier and warm one dummy hash per deployed format.
    ///
    /// The dummies are minted through the installed driver — never
    /// hard-coded — so their cost always tracks the configured profiles
    /// (the fork's `dummy_verify` regression).
    pub fn new(driver: Arc<dyn PasswordHashDriver>, config: PasswordHashConfig) -> Result<Self> {
        let secret = SecretString::from(random_secret());
        let bcrypt_dummy = driver.mint(&config.bcrypt_profile(), &secret)?;
        let argon2_dummy = driver.mint(&config.argon2_target(), &secret)?;
        Ok(Self {
            driver,
            config,
            bcrypt_dummy,
            argon2_dummy,
        })
    }

    /// The configured profiles, exposed for deterministic tests.
    #[must_use]
    pub const fn config(&self) -> &PasswordHashConfig {
        &self.config
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
                Some(profile) if self.needs_rehash(&profile) => {
                    match self.driver.mint(&self.config.argon2_target(), password) {
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

    /// Mint a fresh credential hash at the pinned Argon2id target.
    pub fn mint_target(&self, password: &SecretString) -> Result<String> {
        self.driver.mint(&self.config.argon2_target(), password)
    }

    /// Upgrade-only rehash policy: bcrypt always upgrades; Argon2 upgrades
    /// only when a parameter falls below the pinned target. A stronger
    /// stored Argon2 hash is never downgraded.
    fn needs_rehash(&self, stored: &HashWorkProfile) -> bool {
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
        // "$2b$12$..." — legacy $2a/$2x/$2y variants also parse here.
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
            HashAlgorithm::Bcrypt => {
                // Over-length inputs can never match a Magnetar-accepted
                // bcrypt hash; report a mismatch, not an error, so length
                // discloses nothing (deployed framework behavior).
                if call.password.expose_secret().len() > MAX_BCRYPT_PASSWORD_BYTES {
                    return Ok(false);
                }
                bcrypt::verify(call.password.expose_secret(), call.hash)
                    .map_err(|_| malformed_hash())
            }
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
            HashParameters::Bcrypt { cost } => {
                if password.expose_secret().len() > MAX_BCRYPT_PASSWORD_BYTES {
                    return Err(Error::InvalidInput {
                        field: "password".to_owned(),
                        message: format!(
                            "password exceeds the {MAX_BCRYPT_PASSWORD_BYTES}-byte bcrypt limit"
                        ),
                    });
                }
                bcrypt::hash(password.expose_secret(), cost).map_err(|error| Error::Internal {
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
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
