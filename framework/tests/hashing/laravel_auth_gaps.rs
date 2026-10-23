//! Tests for the `laravel_auth_gaps` block PAR-129: `ARGON_TIME` sets the
//! Argon time when `HASH_TIME` is unset, and `hashing::extend` registers a
//! named driver that `HASH_DRIVER` selects.
//!
//! Each test reads the process environment and the hashing facade's
//! process-wide state (the default driver, built once, and the registry of
//! named drivers), so each runs alone in a child process of this binary
//! (see `own_process`). The parent starts the child with the environment
//! the test needs, so no test mutates its own environment.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use suprnova::FrameworkError;
use suprnova::hashing::{self, Algorithm, HashConfig, Hasher};

/// Every variable the hashing configuration reads, cleared in each child so
/// the developer's own environment cannot reach a test.
const HASHING_VARIABLES: [&str; 9] = [
    "HASH_DRIVER",
    "HASH_ROUNDS",
    "HASH_MEMORY",
    "HASH_TIME",
    "ARGON_TIME",
    "HASH_THREADS",
    "HASH_VERIFY",
    "HASH_MAX_CONCURRENCY",
    "LARAVEL_SHARED_DATABASE",
];

/// Run the test `child` (its path in this binary) alone in a child process
/// whose hashing environment is exactly `variables`, and fail unless it ran
/// and passed.
fn run_alone_with(child: &str, variables: &[(&str, &str)]) {
    let child = {
        // The child inherits the rest of the environment; the lock waits
        // out any test between setting a variable and restoring it.
        let _env = crate::env_lock::lock_env();
        let mut command = crate::own_process::child_command(child);
        for name in HASHING_VARIABLES {
            command.env_remove(name);
        }
        for (name, value) in variables {
            command.env(name, value);
        }
        command.spawn().expect("spawn the child process")
    };
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
}

/// A driver of the application's own: it prefixes the password, so a test
/// can tell its output from any built-in driver's.
struct PepperHasher;

impl Hasher for PepperHasher {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Bcrypt
    }

    fn hash(&self, password: &str) -> Result<String, FrameworkError> {
        Ok(format!("pepper${password}"))
    }

    fn verify(&self, password: &str, hash: &str) -> Result<bool, FrameworkError> {
        Ok(hash == format!("pepper${password}"))
    }

    fn needs_rehash(&self, _hash: &str) -> bool {
        false
    }
}

fn pepper() -> Result<Box<dyn Hasher>, FrameworkError> {
    Ok(Box::new(PepperHasher))
}

fn is_param_error(error: &FrameworkError) -> bool {
    matches!(error, FrameworkError::ParamError { .. })
}

// --- ARGON_TIME -------------------------------------------------------------

#[test]
fn argon_time_sets_the_time_when_hash_time_is_unset() {
    run_alone_with(
        "laravel_auth_gaps::argon_time_sets_the_time_when_hash_time_is_unset_child",
        &[
            ("ARGON_TIME", "3"),
            ("HASH_DRIVER", "argon2id"),
            ("HASH_MEMORY", "8"),
        ],
    );
}

#[test]
fn argon_time_sets_the_time_when_hash_time_is_unset_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let config = HashConfig::from_env().expect("ARGON_TIME=3 is a valid time");
    assert_eq!(config.time, 3);

    let hash = hashing::hash("secret").expect("hash with the configured driver");
    assert_eq!(
        hashing::info(&hash).time,
        Some(3),
        "the Argon driver hashes at the time ARGON_TIME sets"
    );
}

#[test]
fn hash_time_wins_over_argon_time() {
    run_alone_with(
        "laravel_auth_gaps::hash_time_wins_over_argon_time_child",
        &[("HASH_TIME", "2"), ("ARGON_TIME", "5")],
    );
}

#[test]
fn hash_time_wins_over_argon_time_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let config = HashConfig::from_env().expect("both times are valid");
    assert_eq!(config.time, 2);
}

#[test]
fn argon_time_of_zero_fails_the_configuration() {
    run_alone_with(
        "laravel_auth_gaps::argon_time_of_zero_fails_the_configuration_child",
        &[("ARGON_TIME", "0")],
    );
}

#[test]
fn argon_time_of_zero_fails_the_configuration_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let error = HashConfig::from_env().expect_err("ARGON_TIME=0 is below the minimum of 1");
    assert!(is_param_error(&error), "a param error, not {error:?}");
    assert!(error.to_string().contains("ARGON_TIME"), "{error}");
    assert!(
        hashing::default_driver().is_err(),
        "no driver is built from an invalid time"
    );
}

#[test]
fn argon_time_that_is_no_number_fails_the_configuration() {
    run_alone_with(
        "laravel_auth_gaps::argon_time_that_is_no_number_fails_the_configuration_child",
        &[("ARGON_TIME", "soon")],
    );
}

#[test]
fn argon_time_that_is_no_number_fails_the_configuration_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let error = HashConfig::from_env().expect_err("ARGON_TIME=soon is no number");
    assert!(is_param_error(&error), "a param error, not {error:?}");
    assert!(error.to_string().contains("ARGON_TIME"), "{error}");
}

// --- hashing::extend ----------------------------------------------------------

#[test]
fn hash_driver_selects_a_registered_driver() {
    run_alone_with(
        "laravel_auth_gaps::hash_driver_selects_a_registered_driver_child",
        &[("HASH_DRIVER", "pepper")],
    );
}

#[test]
fn hash_driver_selects_a_registered_driver_child() {
    if !crate::own_process::is_child() {
        return;
    }
    hashing::extend("pepper", pepper).expect("pepper is a free name");

    let config = HashConfig::from_env().expect("HASH_DRIVER names a registered driver");
    assert_eq!(
        config.driver,
        Algorithm::Bcrypt,
        "the algorithm field keeps its default for a registered driver"
    );
    assert_eq!(
        hashing::hash("secret").expect("hash"),
        "pepper$secret",
        "the facade hashes with the registered driver"
    );
    assert!(
        hashing::verify("secret", "pepper$secret").expect("verify"),
        "a hash in no known algorithm is verified by the registered driver"
    );
    assert!(!hashing::verify("wrong", "pepper$secret").expect("verify"));
}

#[test]
fn hash_driver_naming_nothing_registered_fails() {
    run_alone_with(
        "laravel_auth_gaps::hash_driver_naming_nothing_registered_fails_child",
        &[("HASH_DRIVER", "missing")],
    );
}

#[test]
fn hash_driver_naming_nothing_registered_fails_child() {
    if !crate::own_process::is_child() {
        return;
    }
    hashing::extend("pepper", pepper).expect("pepper is a free name");

    let error = HashConfig::from_env().expect_err("`missing` is neither built in nor registered");
    assert!(is_param_error(&error), "a param error, not {error:?}");
    assert!(error.to_string().contains("HASH_DRIVER"), "{error}");
    assert!(
        hashing::default_driver().is_err(),
        "no driver is built, and none is fallen back to"
    );
    assert!(hashing::hash("secret").is_err());
}

#[test]
fn extend_refuses_built_in_names_blank_names_and_a_second_registration() {
    run_alone_with(
        "laravel_auth_gaps::extend_refuses_built_in_names_blank_names_and_a_second_registration_child",
        &[],
    );
}

#[test]
fn extend_refuses_built_in_names_blank_names_and_a_second_registration_child() {
    if !crate::own_process::is_child() {
        return;
    }
    for built_in in ["bcrypt", "argon", "argon2i", "argon2id", "ARGON2ID"] {
        let error = hashing::extend(built_in, pepper)
            .expect_err("a built-in algorithm's name cannot be registered");
        assert!(is_param_error(&error), "{built_in}: {error:?}");
    }
    for blank in ["", "  ", " padded"] {
        assert!(
            hashing::extend(blank, pepper).is_err(),
            "`{blank}` can never be selected"
        );
    }

    hashing::extend("once", pepper).expect("the first registration");
    let error = hashing::extend("once", pepper).expect_err("a name registered before");
    assert!(error.to_string().contains("already registered"), "{error}");
}

#[test]
fn extend_is_refused_once_the_default_driver_is_initialised() {
    run_alone_with(
        "laravel_auth_gaps::extend_is_refused_once_the_default_driver_is_initialised_child",
        &[],
    );
}

#[test]
fn extend_is_refused_once_the_default_driver_is_initialised_child() {
    if !crate::own_process::is_child() {
        return;
    }
    hashing::extend("early", pepper).expect("before the default driver exists");
    hashing::default_driver().expect("the default driver initialises");

    let error = hashing::extend("late", pepper)
        .expect_err("a registration after the first use could not take effect");
    assert!(error.to_string().contains("already initialised"), "{error}");
}

#[test]
fn a_registration_leaves_the_default_driver_as_it_was() {
    run_alone_with(
        "laravel_auth_gaps::a_registration_leaves_the_default_driver_as_it_was_child",
        &[("HASH_ROUNDS", "4")],
    );
}

#[test]
fn a_registration_leaves_the_default_driver_as_it_was_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let built = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&built);
    hashing::extend("unused", move || {
        seen.store(true, Ordering::SeqCst);
        pepper()
    })
    .expect("a free name");

    let driver = hashing::default_driver().expect("the default driver");
    assert_eq!(driver.algorithm(), Algorithm::Bcrypt);
    assert!(
        hashing::hash("secret").expect("hash").starts_with("$2b$"),
        "the default driver still writes bcrypt"
    );
    assert!(
        !built.load(Ordering::SeqCst),
        "a factory runs only when HASH_DRIVER names it"
    );
}
