//! Mail boot wiring — reads `MAIL_DRIVER` env and binds the matching
//! transport via [`Mail::set_transport`]. Outside production, defaults to
//! the `log` driver when `MAIL_DRIVER` is unset or names an unknown driver;
//! in production those defaults are a hard boot failure unless the operator
//! opts in (SEC-03 — see [`select_driver`]).

use crate::error::FrameworkError;
use crate::lock;
use crate::mail::Mail;
use crate::mail::log::LogMailTransport;
use crate::mail::mailgun::MailgunMailTransport;
use crate::mail::memory::InMemoryMailTransport;
use crate::mail::postmark::PostmarkMailTransport;
use crate::mail::resend::ResendMailTransport;
use crate::mail::sendgrid::SendGridMailTransport;
use crate::mail::ses::SesMailTransport;
use crate::mail::smtp::SmtpMailTransport;
use std::sync::{Arc, RwLock};

// `RwLock<Option<...>>` (not `OnceLock`) so successive bootstrap calls can
// install a fresh capture handle when the driver is toggled back to memory.
// `OnceLock::set` only succeeds once per process — that would silently leak
// the stale Arc from the FIRST memory bootstrap into every subsequent one,
// confusing tests that switch drivers between cases.
static MEMORY_CAPTURE: RwLock<Option<Arc<InMemoryMailTransport>>> = RwLock::new(None);

/// If the memory driver was selected via env on the most recent call to
/// [`bootstrap_from_env`], return the shared [`InMemoryMailTransport`] so
/// tests can inspect captured messages. Returns `None` after a switch to
/// any non-memory driver.
pub fn captured_in_memory() -> Option<Arc<InMemoryMailTransport>> {
    // Read accessor: degrade to `None` on a poisoned lock rather than
    // panicking (crate-wide read-poison policy — see `crate::lock`).
    lock::read(&MEMORY_CAPTURE, "mail memory capture")
        .ok()
        .and_then(|g| g.clone())
}

fn set_memory_capture(t: Arc<InMemoryMailTransport>) -> Result<(), FrameworkError> {
    *lock::write(&MEMORY_CAPTURE, "mail memory capture")? = Some(t);
    Ok(())
}

fn clear_memory_capture() -> Result<(), FrameworkError> {
    *lock::write(&MEMORY_CAPTURE, "mail memory capture")? = None;
    Ok(())
}

/// Operator opt-in that lets a production deployment boot on a mail driver
/// which renders messages and discards them. Set it to a truthy value
/// (`1` / `true` / `yes` / `on`) to acknowledge that outgoing mail —
/// including password resets and email verifications — will not be
/// delivered. Anything else, including leaving it unset, keeps the
/// fail-closed behaviour.
const ALLOW_NON_DELIVERING_ENV: &str = "MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION";

/// Every `MAIL_DRIVER` value this build recognises.
///
/// Modelled as an enum rather than matched as a bare string so the
/// production fail-closed check in [`select_driver`] and the transport
/// construction in [`bootstrap_from_env`] cannot drift apart: adding a
/// driver forces [`MailDriver::parse`], [`MailDriver::delivers`], and the
/// exhaustive `match` in the bootstrap to be updated together. The old
/// catch-all `other =>` arm is what let an unrecognised value silently
/// become the `log` transport everywhere, production included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MailDriver {
    Log,
    Memory,
    Smtp,
    Postmark,
    Ses,
    SendGrid,
    Mailgun,
    Resend,
}

impl MailDriver {
    /// Map a raw `MAIL_DRIVER` value onto a driver, or `None` when this
    /// build has no such driver.
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "log" => Some(Self::Log),
            "memory" => Some(Self::Memory),
            "smtp" => Some(Self::Smtp),
            "postmark" => Some(Self::Postmark),
            "ses" => Some(Self::Ses),
            "sendgrid" => Some(Self::SendGrid),
            "mailgun" => Some(Self::Mailgun),
            "resend" => Some(Self::Resend),
            _ => None,
        }
    }

    /// Whether this driver hands the rendered message to something that can
    /// actually deliver it. `log` and `memory` render and drop — which is
    /// the entire point in development and a silent outage in production.
    fn delivers(self) -> bool {
        !matches!(self, Self::Log | Self::Memory)
    }

    /// The canonical `MAIL_DRIVER` spelling, for log fields.
    fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Memory => "memory",
            Self::Smtp => "smtp",
            Self::Postmark => "postmark",
            Self::Ses => "ses",
            Self::SendGrid => "sendgrid",
            Self::Mailgun => "mailgun",
            Self::Resend => "resend",
        }
    }
}

/// The outcome of resolving `MAIL_DRIVER`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DriverSelection {
    /// The transport to bind.
    driver: MailDriver,
    /// `Some(raw)` when `MAIL_DRIVER` named a driver this build does not
    /// know and selection fell back to `log`. Carried out to the caller so
    /// the warn can quote the operator's literal value instead of a
    /// sanitised one — that string is usually the typo itself.
    unknown_value: Option<String>,
}

/// Decide which transport `MAIL_DRIVER` selects, refusing in production the
/// values that silently discard mail.
///
/// Takes its inputs explicitly rather than reading `MAIL_DRIVER` / `APP_ENV`
/// itself so the whole decision matrix is unit-testable without mutating
/// process-global env — this crate's tests run massively parallel inside one
/// binary, where an env write races every other test in flight.
///
/// SEC-03: `log` and `memory` render a message and drop it on the floor. An
/// unset `MAIL_DRIVER`, and any value this build does not recognise, both
/// land on that same `log` transport, so all four cases have to fail closed
/// together — otherwise a production deploy that forgot the variable (or
/// typo'd `MAIL_DRIVER=SMTP`) reports every password reset and email
/// verification as sent while nothing leaves the process, and the failure
/// only surfaces when a locked-out user complains. `allow_non_delivering`
/// is the operator's explicit acknowledgement — see
/// [`ALLOW_NON_DELIVERING_ENV`].
///
/// Only `is_production()` gates this, not staging: a staging environment
/// pointed at the `log` driver is a normal, deliberate configuration, and
/// hard-failing it would push teams towards setting the override globally —
/// which would disarm the check where it actually matters.
fn select_driver(
    raw: Option<&str>,
    is_production: bool,
    allow_non_delivering: bool,
) -> Result<DriverSelection, FrameworkError> {
    let selection = match raw {
        Some(value) => match MailDriver::parse(value) {
            Some(driver) => DriverSelection {
                driver,
                unknown_value: None,
            },
            None => DriverSelection {
                driver: MailDriver::Log,
                unknown_value: Some(value.to_string()),
            },
        },
        None => DriverSelection {
            driver: MailDriver::Log,
            unknown_value: None,
        },
    };

    if is_production && !allow_non_delivering && !selection.driver.delivers() {
        let cause = match (&selection.unknown_value, raw) {
            (Some(bad), _) => format!(
                "MAIL_DRIVER=`{bad}` is not a driver this build knows, so it would \
                 fall back to the `log` transport"
            ),
            (None, Some(known)) => {
                format!("MAIL_DRIVER=`{known}` renders mail and then discards it")
            }
            (None, None) => {
                "MAIL_DRIVER is unset, which defaults to the `log` transport".to_string()
            }
        };
        return Err(FrameworkError::internal(format!(
            "refusing to boot in production: {cause}. Password resets and email \
             verifications would report success while nothing is delivered. Set \
             MAIL_DRIVER to a delivering driver (smtp | postmark | ses | sendgrid \
             | mailgun | resend), or set {ALLOW_NON_DELIVERING_ENV}=true to \
             acknowledge that outgoing mail is intentionally discarded."
        )));
    }

    Ok(selection)
}

/// Read a boolean opt-in env var.
fn env_flag_enabled(name: &str) -> bool {
    flag_is_truthy(std::env::var(name).ok().as_deref())
}

/// Whether an opt-in flag value counts as "yes".
///
/// Anything outside the recognised truthy set — including an unparseable
/// value — is `false`. A security override has to be affirmed exactly; a
/// deploy that writes `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=maybe` must
/// keep the guard armed rather than treat "the variable is present" as
/// consent.
fn flag_is_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Read `MAIL_DRIVER` and bind the matching transport globally. Defaults to
/// the `log` driver when the env var is unset.
///
/// Supported values: `log` | `memory` | `smtp` | `postmark` | `ses` |
/// `sendgrid` | `mailgun` | `resend`. Unknown values warn and fall back to
/// `log`.
///
/// # Production fail-closed (SEC-03)
///
/// When `APP_ENV` resolves to production, an unset, unknown, `log`, or
/// `memory` driver returns `Err` instead of binding a transport that
/// discards mail — see [`select_driver`] for why all four cases collapse
/// into one. Set `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true` to boot
/// anyway; the boot then warns loudly on every startup. Outside production
/// nothing changes.
///
/// HTTP-backed providers (postmark, ses, sendgrid, mailgun, resend) also
/// honor a corresponding `MAIL_<PROVIDER>_ENDPOINT` override for pointing
/// at a regional URL or a mock server.
///
/// Synchronous: every supported transport's constructor is sync today.
/// If a future transport adds async initialization (e.g. a connection
/// pre-warm), flip this back to `async` and update the call sites — only
/// `Server::serve` and the boot tests need to add `.await`.
pub fn bootstrap_from_env() -> Result<(), FrameworkError> {
    // Release any previous in-memory capture handle BEFORE matching, so
    // toggling `memory → postmark → memory` always exposes a fresh buffer
    // for the subsequent memory bootstrap.
    clear_memory_capture()?;

    let raw = std::env::var("MAIL_DRIVER").ok();
    let is_production = crate::config::Environment::detect().is_production();
    let selection = select_driver(
        raw.as_deref(),
        is_production,
        env_flag_enabled(ALLOW_NON_DELIVERING_ENV),
    )?;

    if let Some(bad) = &selection.unknown_value {
        tracing::warn!(driver = %bad, "unknown MAIL_DRIVER, falling back to log");
    }
    if is_production && !selection.driver.delivers() {
        // Reachable only via the explicit override — `select_driver` has
        // already returned `Err` otherwise. Restate it every boot so the
        // acknowledgement stays visible in the logs of the deployment it
        // was set on, rather than being a one-time decision nobody recalls.
        tracing::warn!(
            driver = selection.driver.as_str(),
            override_env = ALLOW_NON_DELIVERING_ENV,
            "booting production with a mail driver that discards messages — \
             outgoing mail, including password resets, will NOT be delivered"
        );
    }

    match selection.driver {
        MailDriver::Log => {
            Mail::set_transport(Arc::new(LogMailTransport::new()))?;
        }
        MailDriver::Memory => {
            let t = Arc::new(InMemoryMailTransport::new());
            set_memory_capture(t.clone())?;
            Mail::set_transport(t)?;
        }
        MailDriver::Smtp => {
            let host = std::env::var("MAIL_SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
            let port: u16 = std::env::var("MAIL_SMTP_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(587);
            let user = std::env::var("MAIL_SMTP_USER").ok();
            let pass = std::env::var("MAIL_SMTP_PASS").ok();
            // Both unset → silent dev mode (local maildev / mailpit /
            // mailhog all listen on 1025 unauthenticated). Both set →
            // STARTTLS with the credentials. Exactly one set is almost
            // always a misconfiguration (env file copied without the
            // pair, secret-manager rotation half-applied) and silently
            // booting unencrypted there would let mail go out without
            // the auth the operator clearly intended — surface a loud
            // warn so it shows up in `mail:configure` logs immediately
            // instead of after a forensic dive.
            let transport = match (user, pass) {
                (Some(u), Some(p)) => SmtpMailTransport::starttls(&host, port, &u, &p)?,
                (Some(_), None) => {
                    tracing::warn!(
                        host = %host,
                        port = port,
                        "MAIL_SMTP_USER is set but MAIL_SMTP_PASS is not — SMTP \
                         auth is DISABLED and mail will go out unencrypted; set \
                         BOTH variables to authenticate"
                    );
                    SmtpMailTransport::unencrypted(&host, port)?
                }
                (None, Some(_)) => {
                    tracing::warn!(
                        host = %host,
                        port = port,
                        "MAIL_SMTP_PASS is set but MAIL_SMTP_USER is not — SMTP \
                         auth is DISABLED and mail will go out unencrypted; set \
                         BOTH variables to authenticate"
                    );
                    SmtpMailTransport::unencrypted(&host, port)?
                }
                (None, None) => {
                    // Both unset is the local-catcher path, and silence is
                    // right for it in development. In production it means
                    // mail leaves the process unauthenticated and in
                    // cleartext — almost always because the credentials
                    // were never wired, which is silent failure at exactly
                    // the wrong moment.
                    if crate::config::Environment::detect().is_production() {
                        tracing::warn!(
                            host = %host,
                            port = port,
                            "MAIL_DRIVER=smtp with neither MAIL_SMTP_USER nor \
                             MAIL_SMTP_PASS set: sending unauthenticated cleartext \
                             SMTP in production. Set both, or choose a non-SMTP \
                             MAIL_DRIVER."
                        );
                    }
                    SmtpMailTransport::unencrypted(&host, port)?
                }
            };
            Mail::set_transport(Arc::new(transport))?;
        }
        MailDriver::Postmark => {
            let token = std::env::var("MAIL_POSTMARK_TOKEN").map_err(|_| {
                FrameworkError::internal("MAIL_POSTMARK_TOKEN is required for MAIL_DRIVER=postmark")
            })?;
            let transport = match std::env::var("MAIL_POSTMARK_ENDPOINT") {
                Ok(ep) => PostmarkMailTransport::with_endpoint(token, ep),
                Err(_) => PostmarkMailTransport::new(token),
            };
            Mail::set_transport(Arc::new(transport))?;
        }
        MailDriver::Ses => {
            let key = std::env::var("MAIL_SES_ACCESS_KEY").map_err(|_| {
                FrameworkError::internal("MAIL_SES_ACCESS_KEY is required for MAIL_DRIVER=ses")
            })?;
            let secret = std::env::var("MAIL_SES_SECRET_KEY").map_err(|_| {
                FrameworkError::internal("MAIL_SES_SECRET_KEY is required for MAIL_DRIVER=ses")
            })?;
            let region = std::env::var("MAIL_SES_REGION").unwrap_or_else(|_| "us-east-1".into());
            let transport = match std::env::var("MAIL_SES_ENDPOINT") {
                Ok(ep) => SesMailTransport::with_endpoint(key, secret, region, ep),
                Err(_) => SesMailTransport::new(key, secret, region),
            };
            Mail::set_transport(Arc::new(transport))?;
        }
        MailDriver::SendGrid => {
            let key = std::env::var("MAIL_SENDGRID_API_KEY").map_err(|_| {
                FrameworkError::internal(
                    "MAIL_SENDGRID_API_KEY is required for MAIL_DRIVER=sendgrid",
                )
            })?;
            let transport = match std::env::var("MAIL_SENDGRID_ENDPOINT") {
                Ok(ep) => SendGridMailTransport::with_endpoint(key, ep),
                Err(_) => SendGridMailTransport::new(key),
            };
            Mail::set_transport(Arc::new(transport))?;
        }
        MailDriver::Mailgun => {
            let key = std::env::var("MAIL_MAILGUN_API_KEY").map_err(|_| {
                FrameworkError::internal("MAIL_MAILGUN_API_KEY is required for MAIL_DRIVER=mailgun")
            })?;
            let domain = std::env::var("MAIL_MAILGUN_DOMAIN").map_err(|_| {
                FrameworkError::internal("MAIL_MAILGUN_DOMAIN is required for MAIL_DRIVER=mailgun")
            })?;
            let transport = match std::env::var("MAIL_MAILGUN_ENDPOINT") {
                Ok(ep) => MailgunMailTransport::with_endpoint(key, domain, ep),
                Err(_) => MailgunMailTransport::new(key, domain),
            };
            Mail::set_transport(Arc::new(transport))?;
        }
        MailDriver::Resend => {
            let key = std::env::var("MAIL_RESEND_API_KEY").map_err(|_| {
                FrameworkError::internal("MAIL_RESEND_API_KEY is required for MAIL_DRIVER=resend")
            })?;
            let transport = match std::env::var("MAIL_RESEND_ENDPOINT") {
                Ok(ep) => ResendMailTransport::with_endpoint(key, ep),
                Err(_) => ResendMailTransport::new(key),
            };
            Mail::set_transport(Arc::new(transport))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! SEC-03 decision matrix. Every test here drives [`select_driver`]
    //! with explicit arguments — no `MAIL_DRIVER` / `APP_ENV` mutation, so
    //! these are safe in the massively-parallel lib test binary. The
    //! env-reading path is covered end-to-end by the dedicated
    //! `framework/tests/mail_production_fail_closed.rs` binary.

    use super::*;

    fn err_message(raw: Option<&str>) -> String {
        let err = select_driver(raw, true, false)
            .expect_err("production must refuse a non-delivering driver");
        format!("{err}")
    }

    #[test]
    fn outside_production_unset_still_defaults_to_log() {
        let s = select_driver(None, false, false).expect("non-production boot is unchanged");
        assert_eq!(s.driver, MailDriver::Log);
        assert_eq!(s.unknown_value, None);
    }

    #[test]
    fn outside_production_log_and_memory_are_accepted() {
        for (raw, expected) in [("log", MailDriver::Log), ("memory", MailDriver::Memory)] {
            let s = select_driver(Some(raw), false, false)
                .unwrap_or_else(|e| panic!("{raw} must boot outside production: {e}"));
            assert_eq!(s.driver, expected);
        }
    }

    #[test]
    fn outside_production_unknown_driver_falls_back_to_log_and_reports_the_value() {
        let s = select_driver(Some("bogusdriver"), false, false)
            .expect("unknown driver keeps falling back outside production");
        assert_eq!(s.driver, MailDriver::Log);
        assert_eq!(s.unknown_value.as_deref(), Some("bogusdriver"));
    }

    #[test]
    fn production_refuses_an_unset_driver() {
        let msg = err_message(None);
        assert!(
            msg.contains("MAIL_DRIVER is unset"),
            "names the cause: {msg}"
        );
        assert!(
            msg.contains(ALLOW_NON_DELIVERING_ENV),
            "names the override: {msg}"
        );
    }

    #[test]
    fn production_refuses_log_and_memory() {
        for raw in ["log", "memory"] {
            let msg = err_message(Some(raw));
            assert!(
                msg.contains(&format!("MAIL_DRIVER=`{raw}`")),
                "quotes the configured driver: {msg}"
            );
            assert!(
                msg.contains("discards it"),
                "explains why it is refused: {msg}"
            );
        }
    }

    #[test]
    fn production_refuses_an_unknown_driver_instead_of_falling_back_to_log() {
        // The pre-SEC-03 behaviour — warn, then bind `log` — is exactly the
        // fail-open path. A typo'd `MAIL_DRIVER=SMTP` must not deploy.
        let msg = err_message(Some("SMTP"));
        assert!(
            msg.contains("MAIL_DRIVER=`SMTP`"),
            "quotes the operator's literal value: {msg}"
        );
        assert!(
            msg.contains("not a driver this build knows"),
            "explains the fallback it would have taken: {msg}"
        );
    }

    #[test]
    fn production_accepts_non_delivering_drivers_with_the_explicit_override() {
        for raw in [None, Some("log"), Some("memory")] {
            let s = select_driver(raw, true, true)
                .unwrap_or_else(|e| panic!("override must permit {raw:?}: {e}"));
            assert!(!s.driver.delivers());
        }
    }

    #[test]
    fn production_accepts_every_delivering_driver_without_the_override() {
        for raw in ["smtp", "postmark", "ses", "sendgrid", "mailgun", "resend"] {
            let s = select_driver(Some(raw), true, false)
                .unwrap_or_else(|e| panic!("{raw} delivers and must boot in production: {e}"));
            assert!(s.driver.delivers(), "{raw} must be a delivering driver");
            assert_eq!(s.driver.as_str(), raw, "round-trips its env spelling");
        }
    }

    #[test]
    fn only_exact_truthy_values_arm_the_override() {
        for yes in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(flag_is_truthy(Some(yes)), "{yes:?} must count as opt-in");
        }
        // Failure mode: a present-but-negative or garbled value must NOT be
        // read as consent — "the variable exists" is not an acknowledgement.
        for no in ["0", "false", "no", "off", "maybe", "", "  "] {
            assert!(!flag_is_truthy(Some(no)), "{no:?} must not opt in");
        }
        assert!(!flag_is_truthy(None), "unset must not opt in");
    }
}
