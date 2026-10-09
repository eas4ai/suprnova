//! The second-factor lockout threshold and window come from configuration.
//! A bad value must stop the app at boot, with the variable named, rather
//! than leave the lockout weaker or broken until the first sign-in.

use suprnova::Config;
use suprnova::config::__reset_loaded_keys_for_tests;

use crate::env_snapshot::{EnvSnapshot, set_env};

const KEYS: [&str; 2] = ["TWO_FACTOR_MAX_ATTEMPTS", "TWO_FACTOR_LOCKOUT_MINUTES"];

fn boot() -> Result<suprnova::config::Environment, suprnova::FrameworkError> {
    let root = tempfile::tempdir().expect("tempdir");
    Config::init(root.path())
}

fn boot_with(
    key: &str,
    value: &str,
) -> Result<suprnova::config::Environment, suprnova::FrameworkError> {
    set_env(key, Some(value));
    boot()
}

#[test]
fn a_zero_or_invalid_threshold_fails_boot_and_names_the_variable() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&KEYS);

    for value in ["0", "-3", "five", "2.5"] {
        let error = boot_with("TWO_FACTOR_MAX_ATTEMPTS", value)
            .expect_err("a threshold that is not a positive whole number must fail boot");
        let message = error.to_string();
        assert!(message.contains("TWO_FACTOR_MAX_ATTEMPTS"), "{message}");
    }
}

#[test]
fn a_zero_or_invalid_window_fails_boot_and_names_the_variable() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&KEYS);

    for value in ["0", "-1", "quarter-hour", "99999999999999999"] {
        let error = boot_with("TWO_FACTOR_LOCKOUT_MINUTES", value)
            .expect_err("a window that is not a positive whole number of minutes must fail boot");
        let message = error.to_string();
        assert!(message.contains("TWO_FACTOR_LOCKOUT_MINUTES"), "{message}");
    }
}

#[test]
fn valid_or_unset_values_boot() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&KEYS);

    set_env("TWO_FACTOR_MAX_ATTEMPTS", None);
    set_env("TWO_FACTOR_LOCKOUT_MINUTES", None);
    boot().expect("unset values boot with the defaults");

    set_env("TWO_FACTOR_MAX_ATTEMPTS", Some("3"));
    set_env("TWO_FACTOR_LOCKOUT_MINUTES", Some("60"));
    boot().expect("positive whole numbers boot");
}

#[test]
fn a_window_past_the_maximum_fails_boot_and_names_the_variable() {
    let _env = crate::env_lock::lock_env();
    __reset_loaded_keys_for_tests();
    let _snap = EnvSnapshot::capture(&KEYS);

    // Thirty days is the most a window may span; a longer one reaches back
    // past the timestamp range some engines store.
    for value in ["43201", "4294967295"] {
        let error = boot_with("TWO_FACTOR_LOCKOUT_MINUTES", value)
            .expect_err("a window past the maximum must fail boot");
        let message = error.to_string();
        assert!(message.contains("TWO_FACTOR_LOCKOUT_MINUTES"), "{message}");
    }
    set_env("TWO_FACTOR_LOCKOUT_MINUTES", Some("43200"));
    boot().expect("thirty days is accepted");
}
