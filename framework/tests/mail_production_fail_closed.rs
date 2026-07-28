//! SEC-03: mail must not fail open in production.
//!
//! `mail::boot::bootstrap_from_env` used to default to the `log` driver when
//! `MAIL_DRIVER` was unset, and to fall back to `log` for any value it did
//! not recognise. Both transports render a message and discard it, so a
//! production deploy that forgot the variable (or typo'd it) reported every
//! password reset and email verification as sent while nothing left the
//! process — a silent outage that only surfaces when a locked-out user
//! complains.
//!
//! `Environment::detect()` reads the process-wide `APP_ENV` var and these
//! tests mutate it alongside `MAIL_DRIVER`, so — like
//! `inertia_production_fail_closed.rs` and `app_key_production_fail_closed.rs`
//! — they live in their own test binary and serialise against each other with
//! `#[serial_test::serial]`. Each `tests/*.rs` file is a separate process, so
//! no other integration test can interleave with these.
//!
//! The env-free half of the matrix (`select_driver` driven with explicit
//! arguments) is unit-tested in `framework/src/mail/boot.rs`.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

/// The operator opt-in that permits a discarding mail driver in production.
/// Spelled out literally here rather than imported: the name is the public
/// contract an operator types into a deployment config, so a rename must
/// break this test.
const OVERRIDE_ENV: &str = "MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION";

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct Ping {
    // Tera needs a JSON object for its context; an empty named struct
    // serialises to `{}` where a unit struct would serialise to `null`.
    _placeholder: (),
}

#[async_trait]
impl Mailable for Ping {
    fn mailable_name() -> &'static str {
        "Ping"
    }
    fn subject(&self) -> String {
        "p".into()
    }
    fn text_template_source(&self) -> Option<String> {
        Some("pong".into())
    }
    fn from(&self) -> Option<Address> {
        Some("noreply@suprnova.dev".into())
    }
}

/// Snapshot of every env var these tests touch, so each one restores the
/// process to what it found.
struct EnvGuard {
    app_env: Option<String>,
    mail_driver: Option<String>,
    override_flag: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<String>,
}

impl EnvGuard {
    /// Capture the current values and clear all of them.
    ///
    /// # Safety
    /// Mutates process-global env. Safe here because every test in this file
    /// is `#[serial]`-locked against the others and this binary contains no
    /// other tests.
    fn take() -> Self {
        let guard = Self {
            app_env: std::env::var("APP_ENV").ok(),
            mail_driver: std::env::var("MAIL_DRIVER").ok(),
            override_flag: std::env::var(OVERRIDE_ENV).ok(),
            smtp_host: std::env::var("MAIL_SMTP_HOST").ok(),
            smtp_port: std::env::var("MAIL_SMTP_PORT").ok(),
        };
        unsafe {
            std::env::remove_var("APP_ENV");
            std::env::remove_var("MAIL_DRIVER");
            std::env::remove_var(OVERRIDE_ENV);
            std::env::remove_var("MAIL_SMTP_HOST");
            std::env::remove_var("MAIL_SMTP_PORT");
        }
        let _ = Mail::clear_transport();
        guard
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        let _ = Mail::clear_transport();
        // SAFETY: see `EnvGuard::take`.
        unsafe {
            for (name, value) in [
                ("APP_ENV", &self.app_env),
                ("MAIL_DRIVER", &self.mail_driver),
                (OVERRIDE_ENV, &self.override_flag),
                ("MAIL_SMTP_HOST", &self.smtp_host),
                ("MAIL_SMTP_PORT", &self.smtp_port),
            ] {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// # Safety
/// See [`EnvGuard::take`].
fn set(name: &str, value: &str) {
    unsafe {
        std::env::set_var(name, value);
    }
}

#[tokio::test]
#[serial(mail_sec03_env)]
async fn production_boot_without_a_mail_driver_fails_closed() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("production boot with no MAIL_DRIVER must fail (SEC-03)");
    let msg = format!("{err}");
    assert!(
        msg.contains("MAIL_DRIVER is unset"),
        "error names the cause: {msg}"
    );
    assert!(
        msg.contains(OVERRIDE_ENV),
        "error names the opt-in override: {msg}"
    );

    // Fail closed means no transport was bound either — a later send must
    // not quietly succeed through a leftover default.
    let send_err = Mail::to("alice@example.org")
        .send(Ping::default())
        .await
        .expect_err("a refused boot must leave no transport bound");
    assert!(
        format!("{send_err}").contains("no mail transport configured"),
        "send fails for want of a transport: {send_err}"
    );
}

#[test]
#[serial(mail_sec03_env)]
fn production_boot_on_the_log_driver_fails_closed() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "log");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("MAIL_DRIVER=log in production must fail (SEC-03)");
    let msg = format!("{err}");
    assert!(
        msg.contains("MAIL_DRIVER=`log`"),
        "quotes the driver: {msg}"
    );
}

#[test]
#[serial(mail_sec03_env)]
fn production_boot_on_the_memory_driver_fails_closed() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "memory");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("MAIL_DRIVER=memory in production must fail (SEC-03)");
    assert!(
        format!("{err}").contains("MAIL_DRIVER=`memory`"),
        "quotes the driver: {err}"
    );
    assert!(
        suprnova::mail::boot::captured_in_memory().is_none(),
        "a refused boot must not hand back a capture buffer"
    );
}

#[test]
#[serial(mail_sec03_env)]
fn production_boot_on_an_unknown_driver_fails_instead_of_falling_back_to_log() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    // Correct driver, wrong case — the exact shape that used to warn once
    // and then silently deliver nothing for the life of the deployment.
    set("MAIL_DRIVER", "SMTP");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("an unrecognised MAIL_DRIVER must not fall back to log in production");
    let msg = format!("{err}");
    assert!(
        msg.contains("MAIL_DRIVER=`SMTP`"),
        "quotes the operator's literal value: {msg}"
    );
}

#[tokio::test]
#[serial(mail_sec03_env)]
async fn production_boot_succeeds_with_the_explicit_override() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "log");
    set(OVERRIDE_ENV, "true");

    suprnova::mail::boot::bootstrap_from_env()
        .expect("the explicit override must permit a non-delivering driver in production");

    // And the bound transport is usable — the override is a real escape
    // hatch, not a boot that leaves mail unwired.
    Mail::to("alice@example.org")
        .send(Ping::default())
        .await
        .expect("log transport accepts the send");
}

#[test]
#[serial(mail_sec03_env)]
fn a_negative_override_value_does_not_open_the_gate() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set(OVERRIDE_ENV, "false");

    // Failure mode: the variable is present, so a naive `env::var(..).is_ok()`
    // check would treat it as consent. Only a truthy value counts.
    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=false must keep the guard armed");
    assert!(
        format!("{err}").contains("MAIL_DRIVER is unset"),
        "still the unset-driver refusal: {err}"
    );
}

// `#[tokio::test]`, not `#[test]`: lettre's SMTP transport binds to the
// ambient Tokio reactor both when it is constructed and when it is dropped.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn production_boot_on_a_delivering_driver_is_unaffected() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_PORT", "2587");

    // No send here: `smtp.example.com` is not reachable and this test must
    // not depend on the network. Constructing the transport is the assertion
    // — the SEC-03 gate is upstream of it.
    suprnova::mail::boot::bootstrap_from_env()
        .expect("a driver that actually delivers must boot in production untouched");
}

#[tokio::test]
#[serial(mail_sec03_env)]
async fn non_production_boot_is_unchanged() {
    for app_env in [None, Some("local"), Some("development"), Some("staging")] {
        let _guard = EnvGuard::take();
        if let Some(v) = app_env {
            set("APP_ENV", v);
        }

        // Unset driver still defaults to `log` and still sends.
        suprnova::mail::boot::bootstrap_from_env()
            .unwrap_or_else(|e| panic!("APP_ENV={app_env:?} must keep the log default: {e}"));
        Mail::to("alice@example.org")
            .send(Ping::default())
            .await
            .expect("log transport accepts the send");

        // And an unknown driver still warns-and-falls-back rather than failing.
        set("MAIL_DRIVER", "bogusdriver");
        suprnova::mail::boot::bootstrap_from_env().unwrap_or_else(|e| {
            panic!("APP_ENV={app_env:?} must keep the unknown-driver fallback: {e}")
        });
    }
}
