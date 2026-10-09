//! Sharing the database with a Laravel application.
//!
//! A Suprnova application runs on a database Laravel 13 created without
//! any setting: the framework's stores use Laravel's table layouts (see
//! `manual/laravel-database.md`). Running *beside* a Laravel application
//! on that database needs one more thing, which this module's setting
//! turns on:
//!
//! - Password hashes are written as `$2y$` bcrypt, which Laravel's
//!   hasher accepts with `HASH_VERIFY=true`. A valid sign-in rewrites a
//!   `$2b$` or Argon2id hash as `$2y$`, and Magnetar stops upgrading
//!   bcrypt hashes to Argon2id.
//! - The database queue puts an unrouted job on the queue
//!   [`SHARED_DEFAULT_QUEUE`] instead of `default`, so Laravel's default
//!   worker, which reads `default`, never reserves a Suprnova job.
//!
//! Both are trade-offs: `$2b$` and Argon2id are the framework's defaults
//! for good reasons, and `default` is the queue name every Suprnova guide
//! uses. So the setting is off unless an application turns it on, with
//! `LARAVEL_SHARED_DATABASE=true` or [`LaravelDatabase::share`].

use std::sync::atomic::{AtomicU8, Ordering};

use crate::error::FrameworkError;

/// The environment variable that turns the setting on: `true`, `1`, `yes`
/// or `on`; `false`, `0`, `no` or `off` (or unset) leave it off.
pub const SHARED_DATABASE_ENV: &str = "LARAVEL_SHARED_DATABASE";

/// The queue an unrouted job goes to on the database queue while the
/// setting is on. Laravel's `database` connection reads `default` unless
/// `DB_QUEUE` names another, so this name keeps the two workers apart.
pub const SHARED_DEFAULT_QUEUE: &str = "suprnova";

/// `0` = follow the environment, `1` = off, `2` = on.
static OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// The one setting for an application that shares its database with a
/// Laravel application. See the [module docs](self) for what it changes.
pub struct LaravelDatabase;

impl LaravelDatabase {
    /// Whether the application shares its database with a Laravel
    /// application: [`Self::share`] when it was called, otherwise
    /// [`SHARED_DATABASE_ENV`].
    pub fn is_shared() -> bool {
        match OVERRIDE.load(Ordering::Acquire) {
            1 => false,
            2 => true,
            _ => matches!(
                parse_flag(std::env::var(SHARED_DATABASE_ENV).ok()),
                Ok(true)
            ),
        }
    }

    /// Turn the setting on or off for this process, whatever
    /// [`SHARED_DATABASE_ENV`] says. For bootstrap code that reads it from
    /// its own configuration, and for tests.
    pub fn share(shared: bool) {
        OVERRIDE.store(if shared { 2 } else { 1 }, Ordering::Release);
    }

    /// Follow [`SHARED_DATABASE_ENV`] again after [`Self::share`].
    pub fn follow_environment() {
        OVERRIDE.store(0, Ordering::Release);
    }

    /// The name of the queue an unrouted job is stored under on the
    /// database queue: [`SHARED_DEFAULT_QUEUE`] while the setting is on,
    /// `default` otherwise.
    pub fn default_queue() -> &'static str {
        if Self::is_shared() {
            SHARED_DEFAULT_QUEUE
        } else {
            crate::queue::envelope::DEFAULT_QUEUE
        }
    }

    /// Check [`SHARED_DATABASE_ENV`] at boot, so a value that is neither
    /// on nor off stops the application instead of reading as off.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the variable and its value.
    pub fn validate_environment() -> Result<(), FrameworkError> {
        parse_flag(std::env::var(SHARED_DATABASE_ENV).ok()).map(|_| ())
    }
}

fn parse_flag(raw: Option<String>) -> Result<bool, FrameworkError> {
    let Some(raw) = raw else {
        return Ok(false);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err(FrameworkError::internal(format!(
            "{SHARED_DATABASE_ENV}={raw:?} is neither on nor off; use true or false"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_values_parse_strictly() {
        assert!(!parse_flag(None).unwrap());
        assert!(parse_flag(Some("TRUE".into())).unwrap());
        assert!(parse_flag(Some(" on ".into())).unwrap());
        assert!(!parse_flag(Some("0".into())).unwrap());
        let refused = parse_flag(Some("maybe".into())).unwrap_err().to_string();
        assert!(refused.contains(SHARED_DATABASE_ENV), "{refused}");
    }
}
