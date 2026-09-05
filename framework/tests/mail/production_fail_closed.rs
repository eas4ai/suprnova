//! SEC-03: mail must not fail open in production.
//!
//! `mail::boot::bootstrap_from_env` used to default to the `log` driver when
//! `MAIL_DRIVER` was unset, and to fall back to `log` for any value it did
//! not recognise. Both transports render a message and discard it, so a
//! production deploy that forgot the variable (or typo'd it) reported every
//! password reset and email verification as sent while nothing left the
//! process - a silent outage that only surfaces when a locked-out user
//! complains.
//!
//! `Environment::detect()` reads the process-wide `APP_ENV` var and these
//! tests mutate it alongside `MAIL_DRIVER`, so - like
//! `inertia_production_fail_closed.rs` and `app_key_production_fail_closed.rs` -
//! they live in their own test binary and serialise against each other with
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

/// The P2-03 counterpart: same reasoning, so it is spelled out here too.
const INSECURE_SMTP_ENV: &str = "MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION";

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
    smtp_user: Option<String>,
    smtp_pass: Option<String>,
    smtp_encryption: Option<String>,
    insecure_flag: Option<String>,
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
            smtp_user: std::env::var("MAIL_SMTP_USER").ok(),
            smtp_pass: std::env::var("MAIL_SMTP_PASS").ok(),
            smtp_encryption: std::env::var("MAIL_SMTP_ENCRYPTION").ok(),
            insecure_flag: std::env::var(INSECURE_SMTP_ENV).ok(),
        };
        unsafe {
            std::env::remove_var("APP_ENV");
            std::env::remove_var("MAIL_DRIVER");
            std::env::remove_var(OVERRIDE_ENV);
            std::env::remove_var("MAIL_SMTP_HOST");
            std::env::remove_var("MAIL_SMTP_PORT");
            std::env::remove_var("MAIL_SMTP_USER");
            std::env::remove_var("MAIL_SMTP_PASS");
            std::env::remove_var("MAIL_SMTP_ENCRYPTION");
            std::env::remove_var(INSECURE_SMTP_ENV);
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
                ("MAIL_SMTP_USER", &self.smtp_user),
                ("MAIL_SMTP_PASS", &self.smtp_pass),
                ("MAIL_SMTP_ENCRYPTION", &self.smtp_encryption),
                (INSECURE_SMTP_ENV, &self.insecure_flag),
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

    // Fail closed means no transport was bound either - a later send must
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
    // Correct driver, wrong case - the exact shape that used to warn once
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

    // And the bound transport is usable - the override is a real escape
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
    // Credentials are supplied because P2-03 added a second, orthogonal
    // production requirement: delivering is necessary but no longer
    // sufficient - the connection must also be encrypted. Without these
    // this test would now be exercising the P2-03 refusal rather than the
    // SEC-03 pass-through it is named for.
    set("MAIL_SMTP_USER", "relay-user");
    set("MAIL_SMTP_PASS", "relay-pass");

    // No send here: `smtp.example.com` is not reachable and this test must
    // not depend on the network. Constructing the transport is the assertion -
    // the SEC-03 gate is upstream of it.
    suprnova::mail::boot::bootstrap_from_env()
        .expect("a driver that actually delivers must boot in production untouched");
}

// ---------------------------------------------------------------------
// P2-03 - production must not send SMTP in the clear.
// ---------------------------------------------------------------------

/// The finding, through the real env-reading path. Three of the four
/// `(user, pass)` arms used to land on `builder_dangerous` - no TLS, no
/// certificate check - and the both-unset arm logged a `warn!` in
/// production and booted plaintext anyway.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn production_smtp_without_credentials_refuses_to_boot() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_PORT", "2587");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("production SMTP with no credentials resolves to cleartext");
    let msg = format!("{err}");
    assert!(
        msg.contains(INSECURE_SMTP_ENV),
        "the refusal must name the variable that unblocks it: {msg}"
    );
}

/// Explicitly asking for no encryption is refused the same way. An
/// operator who set this deliberately still has to acknowledge it.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn production_smtp_with_encryption_none_refuses_to_boot() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_PORT", "2587");
    set("MAIL_SMTP_USER", "relay-user");
    set("MAIL_SMTP_PASS", "relay-pass");
    set("MAIL_SMTP_ENCRYPTION", "none");

    suprnova::mail::boot::bootstrap_from_env()
        .expect_err("MAIL_SMTP_ENCRYPTION=none in production must fail closed");
}

/// The escape hatch, for a relay on a private network.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn the_insecure_override_lets_production_boot_in_the_clear() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_PORT", "2587");
    set(INSECURE_SMTP_ENV, "true");

    suprnova::mail::boot::bootstrap_from_env()
        .expect("the override exists precisely to permit this");
}

/// Same truthiness discipline as its SEC-03 sibling: presence is not
/// consent. A deploy that writes `=false` must keep the guard armed.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn a_non_truthy_insecure_override_keeps_the_guard_armed() {
    for value in ["false", "0", "no", "maybe", ""] {
        let _guard = EnvGuard::take();
        set("APP_ENV", "production");
        set("MAIL_DRIVER", "smtp");
        set("MAIL_SMTP_HOST", "smtp.example.com");
        set(INSECURE_SMTP_ENV, value);

        let result = suprnova::mail::boot::bootstrap_from_env();
        assert!(
            result.is_err(),
            "{INSECURE_SMTP_ENV}={value:?} must not count as consent - the \
             guard stays armed, so boot must still refuse"
        );
    }
}

/// Implicit TLS, previously unreachable from any combination of
/// environment variables.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn implicit_tls_boots_from_the_environment() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_PORT", "465");
    set("MAIL_SMTP_USER", "relay-user");
    set("MAIL_SMTP_PASS", "relay-pass");
    set("MAIL_SMTP_ENCRYPTION", "tls");

    suprnova::mail::boot::bootstrap_from_env()
        .expect("MAIL_SMTP_ENCRYPTION=tls must build an implicit-TLS transport");
}

/// An encrypted mode with no credentials is refused by the *caller*, with
/// a message about the credentials rather than about encryption - the two
/// failures must stay distinguishable to whoever is reading the log.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn an_encrypted_mode_without_credentials_names_the_credentials() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "production");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "smtp.example.com");
    set("MAIL_SMTP_ENCRYPTION", "starttls");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("starttls has nothing to authenticate with");
    let msg = format!("{err}");
    assert!(
        msg.contains("MAIL_SMTP_USER") && msg.contains("MAIL_SMTP_PASS"),
        "the error must name the missing credentials: {msg}"
    );
}

/// The compatibility guarantee, end to end: `suprnova new` writes no
/// credentials and no encryption setting, and its Mailpit speaks no TLS.
/// If this fails, a fresh scaffold cannot send mail locally.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn development_smtp_still_boots_against_a_local_catcher() {
    for app_env in [None, Some("local"), Some("development"), Some("testing")] {
        let _guard = EnvGuard::take();
        if let Some(v) = app_env {
            set("APP_ENV", v);
        }
        set("MAIL_DRIVER", "smtp");
        set("MAIL_SMTP_HOST", "localhost");
        set("MAIL_SMTP_PORT", "1025");

        suprnova::mail::boot::bootstrap_from_env().unwrap_or_else(|e| {
            panic!(
                "APP_ENV={app_env:?} must still reach a local mail catcher with zero config: {e}"
            )
        });
    }
}

/// A typo must not degrade to plaintext, and must surface on the
/// developer's machine rather than in the deploy.
#[tokio::test]
#[serial(mail_sec03_env)]
async fn an_unrecognised_encryption_value_fails_outside_production_too() {
    let _guard = EnvGuard::take();
    set("APP_ENV", "local");
    set("MAIL_DRIVER", "smtp");
    set("MAIL_SMTP_HOST", "localhost");
    set("MAIL_SMTP_PORT", "1025");
    set("MAIL_SMTP_ENCRYPTION", "tsl");

    let err = suprnova::mail::boot::bootstrap_from_env()
        .expect_err("a transposed `tls` must not silently mean `none`");
    assert!(
        format!("{err}").contains("tsl"),
        "the error must quote the typo: {err}"
    );
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
