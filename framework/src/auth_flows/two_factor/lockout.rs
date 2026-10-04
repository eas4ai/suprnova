//! The threshold and window of the second-factor brute-force counter.

use crate::error::FrameworkError;

/// Failed second-factor attempts that lock the [`super::TwoFactor`] proof
/// paths, when `TWO_FACTOR_MAX_ATTEMPTS` is unset. The same default as
/// Magnetar's password lockout.
const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Minutes an attempt counts, when `TWO_FACTOR_LOCKOUT_MINUTES` is unset.
const DEFAULT_LOCKOUT_MINUTES: u32 = 15;

/// The longest window, thirty days. The window's start is compared with
/// stored attempt times, and a start reaching back past 1970 falls outside
/// the TIMESTAMP range MySQL and MariaDB store; thirty days is far inside
/// it and longer than any lockout an application needs.
const MAX_LOCKOUT_MINUTES: u32 = 43_200;

/// How many second-factor failures lock the [`super::TwoFactor`] proof
/// paths, and for how long each failure counts.
///
/// Read from `TWO_FACTOR_MAX_ATTEMPTS` (default 5) and
/// `TWO_FACTOR_LOCKOUT_MINUTES` (default 15, at most 43200, thirty days)
/// in the application's `.env`
/// file, or built in code with [`Self::new`] and bound with
/// `App::singleton`, which wins over the environment. `Config::init`
/// checks the environment values at boot, so a bad value stops the app
/// with the variable named instead of weakening or breaking the lockout
/// at the first sign-in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TwoFactorLockout {
    max_attempts: u32,
    window: chrono::Duration,
}

impl Default for TwoFactorLockout {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            window: chrono::Duration::minutes(i64::from(DEFAULT_LOCKOUT_MINUTES)),
        }
    }
}

impl TwoFactorLockout {
    /// A lockout of `max_attempts` failures inside `window_minutes`.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when either value is zero - a zero
    /// threshold would lock every proof, and a zero window would count
    /// nothing - or when the window is longer than thirty days (43200
    /// minutes).
    pub fn new(max_attempts: u32, window_minutes: u32) -> Result<Self, FrameworkError> {
        if max_attempts == 0 {
            return Err(rejected(
                "TWO_FACTOR_MAX_ATTEMPTS",
                "0",
                "must be at least 1",
            ));
        }
        if window_minutes == 0 {
            return Err(rejected(
                "TWO_FACTOR_LOCKOUT_MINUTES",
                "0",
                "must be at least 1",
            ));
        }
        if window_minutes > MAX_LOCKOUT_MINUTES {
            return Err(rejected(
                "TWO_FACTOR_LOCKOUT_MINUTES",
                &window_minutes.to_string(),
                "must be at most 43200 (thirty days)",
            ));
        }
        Ok(Self {
            max_attempts,
            window: chrono::Duration::minutes(i64::from(window_minutes)),
        })
    }

    /// Reads `TWO_FACTOR_MAX_ATTEMPTS` and `TWO_FACTOR_LOCKOUT_MINUTES` from
    /// the environment, which `Config::init` has loaded from the
    /// application's `.env` file. An unset or empty key takes its default.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the key, the value and the rule
    /// when a key is not a whole number of at least 1.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] over any reader, so the parser can be proven
    /// against fixed pairs rather than against the process environment.
    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let max_attempts = positive(read, "TWO_FACTOR_MAX_ATTEMPTS", DEFAULT_MAX_ATTEMPTS)?;
        let window_minutes = positive(read, "TWO_FACTOR_LOCKOUT_MINUTES", DEFAULT_LOCKOUT_MINUTES)?;
        Self::new(max_attempts, window_minutes)
    }

    /// The lockout the proof paths run under: the one bound in the
    /// container when the application built one in code, otherwise the one
    /// the environment describes.
    pub(crate) fn resolve() -> Result<Self, FrameworkError> {
        crate::App::resolve::<Self>().or_else(|_| Self::from_env())
    }

    /// Failures inside the window that lock the proof paths.
    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    /// How long each attempt counts. A lock lifts once the failure that
    /// completed it is this old.
    #[must_use]
    pub const fn window(self) -> chrono::Duration {
        self.window
    }
}

/// Parse `key` as a whole number of at least 1, or take `default` when it
/// is unset or empty.
fn positive(
    read: &dyn Fn(&str) -> Option<String>,
    key: &str,
    default: u32,
) -> Result<u32, FrameworkError> {
    let Some(raw) = read(key).filter(|value| !value.trim().is_empty()) else {
        return Ok(default);
    };
    match raw.trim().parse::<u32>() {
        Ok(0) => Err(rejected(key, raw.trim(), "must be at least 1")),
        Ok(value) => Ok(value),
        Err(_) => Err(rejected(
            key,
            raw.trim(),
            "must be a whole number of at least 1",
        )),
    }
}

fn rejected(key: &str, value: &str, rule: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "Two-factor configuration rejected: {key}={value} {rule}. Change it in the \
         application's .env file."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(pairs: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn unset_or_empty_keys_take_the_defaults() {
        assert_eq!(
            TwoFactorLockout::from_source(&read(vec![])).unwrap(),
            TwoFactorLockout::default()
        );
        let lockout = TwoFactorLockout::from_source(&read(vec![
            ("TWO_FACTOR_MAX_ATTEMPTS", " "),
            ("TWO_FACTOR_LOCKOUT_MINUTES", ""),
        ]))
        .unwrap();
        assert_eq!(lockout.max_attempts(), 5);
        assert_eq!(lockout.window(), chrono::Duration::minutes(15));
    }

    #[test]
    fn whole_numbers_set_the_threshold_and_the_window() {
        let lockout = TwoFactorLockout::from_source(&read(vec![
            ("TWO_FACTOR_MAX_ATTEMPTS", "3"),
            ("TWO_FACTOR_LOCKOUT_MINUTES", " 60 "),
        ]))
        .unwrap();
        assert_eq!(lockout.max_attempts(), 3);
        assert_eq!(lockout.window(), chrono::Duration::minutes(60));
    }

    #[test]
    fn zero_or_malformed_values_are_rejected_with_the_key_named() {
        for (key, value) in [
            ("TWO_FACTOR_MAX_ATTEMPTS", "0"),
            ("TWO_FACTOR_MAX_ATTEMPTS", "-1"),
            ("TWO_FACTOR_MAX_ATTEMPTS", "many"),
            ("TWO_FACTOR_LOCKOUT_MINUTES", "0"),
            ("TWO_FACTOR_LOCKOUT_MINUTES", "1.5"),
            ("TWO_FACTOR_LOCKOUT_MINUTES", "99999999999"),
            ("TWO_FACTOR_LOCKOUT_MINUTES", "43201"),
        ] {
            let error = TwoFactorLockout::from_source(&read(vec![(key, value)])).unwrap_err();
            assert!(error.to_string().contains(key), "{error}");
        }
        assert!(TwoFactorLockout::new(0, 15).is_err());
        assert!(TwoFactorLockout::new(5, 0).is_err());
        assert!(TwoFactorLockout::new(5, 43_201).is_err());
        assert!(TwoFactorLockout::new(5, 43_200).is_ok());
    }
}
