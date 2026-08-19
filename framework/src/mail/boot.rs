//! Mail boot wiring — reads `MAIL_DRIVER` env and binds the matching
//! transport via [`Mail::set_transport`]. Outside production, defaults to
//! the `log` driver when `MAIL_DRIVER` is unset or names an unknown driver;
//! in production those defaults are a hard boot failure unless the operator
//! opts in (SEC-03 — see `select_driver` in this module).

use crate::error::FrameworkError;
use crate::lock;
use crate::mail::Mail;
use crate::mail::file::FileMailTransport;
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

/// Operator opt-in that lets a production deployment boot on an
/// unencrypted SMTP connection. Same shape and same truthiness rules as
/// [`ALLOW_NON_DELIVERING_ENV`] — deliberately, so operators learn one
/// pattern rather than two.
///
/// The legitimate use is a relay reachable only over a private network
/// (a sidecar, a VPC-internal Postfix). Everywhere else, cleartext SMTP
/// puts the credentials and every password-reset link on the wire.
const ALLOW_INSECURE_SMTP_ENV: &str = "MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION";

/// How `MAIL_DRIVER=smtp` should encrypt its connection.
///
/// Selected by `MAIL_SMTP_ENCRYPTION`. The variant names follow Laravel's
/// `MAIL_ENCRYPTION` so a `.env` carried across from a Laravel app means
/// the same thing here; the aliases exist for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmtpEncryption {
    /// Connect in the clear, then upgrade with `STARTTLS`. The submission
    /// standard, and what port 587 expects.
    StartTls,
    /// TLS from the first byte. What port 465 expects.
    ///
    /// [`SmtpMailTransport::tls`] has existed and been tested since the
    /// transport was written, but nothing could reach it:
    /// `bootstrap_from_env` only ever constructed `starttls` or
    /// `unencrypted`, so an operator whose relay requires 465 had no
    /// combination of environment variables that worked.
    Tls,
    /// No encryption at all. For local mail catchers — Mailpit, MailHog,
    /// maildev — which listen unauthenticated on 1025.
    Plaintext,
}

impl SmtpEncryption {
    /// Parse a `MAIL_SMTP_ENCRYPTION` value, or `None` if unrecognised.
    ///
    /// `ssl` and `null` are accepted as aliases because that is what
    /// Laravel's `MAIL_ENCRYPTION` uses, and a copied `.env` is the most
    /// likely way this variable gets its first value.
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "starttls" => Some(Self::StartTls),
            "tls" | "ssl" | "implicit" => Some(Self::Tls),
            "none" | "null" | "plaintext" | "insecure" => Some(Self::Plaintext),
            _ => None,
        }
    }

    /// The canonical spelling, for error messages and log fields.
    fn as_str(self) -> &'static str {
        match self {
            Self::StartTls => "starttls",
            Self::Tls => "tls",
            Self::Plaintext => "none",
        }
    }

    /// Whether this mode puts the connection inside TLS.
    fn is_encrypted(self) -> bool {
        !matches!(self, Self::Plaintext)
    }
}

/// Decide how the SMTP transport encrypts, refusing to boot production in
/// cleartext.
///
/// Takes its inputs explicitly for the same reason [`select_driver`] does:
/// this crate's tests run massively parallel in one binary, where an env
/// write races every other test in flight.
///
/// Three of the four `(user, pass)` arms used to land on
/// [`SmtpMailTransport::unencrypted`] — `builder_dangerous`, no TLS and no
/// certificate check — and the both-unset arm merely logged a `warn!` in
/// production before booting plaintext anyway. A warning is the wrong
/// instrument here: it appears once at boot, in a log nobody reads on a
/// green deploy, while every password-reset link the application ever
/// sends crosses the network in the clear.
///
/// **Unset is not a fixed default.** It derives from whether credentials
/// were supplied, which reproduces the previous behaviour exactly: no
/// credentials means the local-catcher path (Mailpit on 1025 speaks no
/// TLS, so defaulting to `starttls` would break `suprnova new` out of the
/// box), and credentials mean STARTTLS. So this knob changes nothing for
/// anyone who does not set it — except in production, where the plaintext
/// branch now fails closed.
///
/// An unrecognised value is an error in *every* environment, not just
/// production. `MAIL_SMTP_ENCRYPTION=tsl` silently degrading to plaintext
/// is precisely the failure this exists to prevent, and a typo should
/// surface on the developer's machine rather than in the deploy.
fn resolve_smtp_encryption(
    raw: Option<&str>,
    has_credentials: bool,
    is_production: bool,
    allow_insecure: bool,
) -> Result<SmtpEncryption, FrameworkError> {
    let encryption = match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => SmtpEncryption::parse(value).ok_or_else(|| {
            FrameworkError::internal(format!(
                "MAIL_SMTP_ENCRYPTION=`{value}` is not a value this build \
                 recognises. Use `starttls` (STARTTLS on the submission port, \
                 usually 587), `tls` (implicit TLS, usually 465), or `none` \
                 (no encryption — local mail catchers only)."
            ))
        })?,
        None if has_credentials => SmtpEncryption::StartTls,
        None => SmtpEncryption::Plaintext,
    };

    if is_production && !encryption.is_encrypted() && !allow_insecure {
        return Err(FrameworkError::internal(format!(
            "refusing to boot in production: MAIL_DRIVER=smtp resolved to an \
             unencrypted connection, so the SMTP credentials and every message \
             body — including password-reset and email-verification links — \
             would cross the network in cleartext. Set MAIL_SMTP_USER and \
             MAIL_SMTP_PASS (STARTTLS is then the default), or set \
             MAIL_SMTP_ENCRYPTION=tls for a relay that expects implicit TLS on \
             465, or set {ALLOW_INSECURE_SMTP_ENV}=true to acknowledge \
             cleartext SMTP — which is only defensible when the relay is \
             reachable solely over a private network."
        )));
    }

    Ok(encryption)
}

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
    File,
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
            "file" => Some(Self::File),
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
        !matches!(self, Self::Log | Self::Memory | Self::File)
    }

    /// The canonical `MAIL_DRIVER` spelling, for log fields.
    fn as_str(self) -> &'static str {
        match self {
            Self::Log => "log",
            Self::Memory => "memory",
            Self::File => "file",
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

// The truthiness rule for security opt-ins lives in `config::env` so
// every guard that ships one agrees on what "yes" means. Two escape
// hatches that disagree about whether `RATE_LIMIT_..._IN_PRODUCTION=Y`
// counts is a footgun nobody would ever debug.
use crate::config::env::env_flag_enabled;

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
/// discards mail — see `select_driver` for why all four cases collapse
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
        MailDriver::File => {
            let dir = std::env::var("MAIL_FILE_PATH").unwrap_or_else(|_| "storage/mail".into());
            Mail::set_transport(Arc::new(FileMailTransport::new(dir)))?;
        }
        MailDriver::Smtp => {
            let host = std::env::var("MAIL_SMTP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
            let port: u16 = std::env::var("MAIL_SMTP_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(587);
            let user = std::env::var("MAIL_SMTP_USER").ok();
            let pass = std::env::var("MAIL_SMTP_PASS").ok();

            // Exactly one of the pair set is almost always a
            // misconfiguration (env file copied without its partner,
            // secret-manager rotation half-applied). Say so before
            // resolving, because what it resolves *to* — no credentials,
            // therefore the local-catcher default — is not what the
            // operator was reaching for.
            match (&user, &pass) {
                (Some(_), None) => tracing::warn!(
                    host = %host,
                    port = port,
                    "MAIL_SMTP_USER is set but MAIL_SMTP_PASS is not — SMTP auth \
                     is DISABLED; set BOTH variables to authenticate"
                ),
                (None, Some(_)) => tracing::warn!(
                    host = %host,
                    port = port,
                    "MAIL_SMTP_PASS is set but MAIL_SMTP_USER is not — SMTP auth \
                     is DISABLED; set BOTH variables to authenticate"
                ),
                _ => {}
            }

            let is_production = crate::config::Environment::detect().is_production();
            let encryption = resolve_smtp_encryption(
                std::env::var("MAIL_SMTP_ENCRYPTION").ok().as_deref(),
                user.is_some() && pass.is_some(),
                is_production,
                env_flag_enabled(ALLOW_INSECURE_SMTP_ENV),
            )?;

            let transport = match encryption {
                SmtpEncryption::StartTls | SmtpEncryption::Tls => {
                    // Both encrypted constructors take credentials, so a
                    // half-set or absent pair cannot reach them. Refusing
                    // with the variable names beats lettre's connection
                    // error three seconds into the first send.
                    let (Some(u), Some(p)) = (user, pass) else {
                        return Err(FrameworkError::internal(format!(
                            "MAIL_SMTP_ENCRYPTION={} requires both MAIL_SMTP_USER \
                             and MAIL_SMTP_PASS. Set them, or set \
                             MAIL_SMTP_ENCRYPTION=none for an unauthenticated \
                             local mail catcher.",
                            encryption.as_str()
                        )));
                    };
                    match encryption {
                        SmtpEncryption::Tls => SmtpMailTransport::tls(&host, port, &u, &p)?,
                        _ => SmtpMailTransport::starttls(&host, port, &u, &p)?,
                    }
                }
                SmtpEncryption::Plaintext => {
                    // Reaching here in production means the operator set
                    // the override — `resolve_smtp_encryption` refuses
                    // otherwise — so this is an acknowledged risk, not a
                    // discovery. Still worth one line per boot: the
                    // override tends to outlive the sidecar that justified
                    // it.
                    if is_production {
                        tracing::warn!(
                            host = %host,
                            port = port,
                            "sending cleartext SMTP in production because \
                             {ALLOW_INSECURE_SMTP_ENV} is set; credentials and \
                             message bodies are not encrypted in transit"
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
    // The truthiness rule moved to `config::env` so every production
    // escape hatch shares one definition; this suite still owns the
    // assertions about what counts as consent.
    use crate::config::env::flag_is_truthy;

    fn err_message(raw: Option<&str>) -> String {
        let err = select_driver(raw, true, false)
            .expect_err("production must refuse a non-delivering driver");
        format!("{err}")
    }

    // P2-03 — SMTP encryption decision matrix. Same discipline as the
    // SEC-03 tests above: explicit arguments, no env mutation.

    fn encryption(raw: Option<&str>, has_credentials: bool, is_production: bool) -> SmtpEncryption {
        resolve_smtp_encryption(raw, has_credentials, is_production, false)
            .unwrap_or_else(|e| panic!("expected a resolved encryption mode, got: {e}"))
    }

    /// The compatibility guarantee. `suprnova new` writes no
    /// `MAIL_SMTP_ENCRYPTION` and no credentials, and its Mailpit listens
    /// on 1025 speaking no TLS. If this test fails, a fresh scaffold can
    /// no longer send mail on a developer's machine.
    #[test]
    fn without_credentials_development_still_gets_the_local_catcher_path() {
        assert_eq!(encryption(None, false, false), SmtpEncryption::Plaintext);
    }

    /// The other half of that guarantee: credentials still mean STARTTLS,
    /// exactly as the `(Some, Some)` arm did before this knob existed.
    #[test]
    fn with_credentials_the_default_is_starttls() {
        assert_eq!(encryption(None, true, false), SmtpEncryption::StartTls);
        assert_eq!(encryption(None, true, true), SmtpEncryption::StartTls);
    }

    /// The finding. Production plus the plaintext path is a boot failure,
    /// where it used to be a `warn!` that scrolled past.
    #[test]
    fn production_refuses_to_boot_on_an_unencrypted_smtp_connection() {
        // Explicitly asking for plaintext is refused whether or not
        // credentials exist. Credentials do not make a cleartext
        // connection safe — they are among the things being sent in it.
        for has_credentials in [false, true] {
            for raw in ["none", "null", "plaintext", "insecure"] {
                let resolved = resolve_smtp_encryption(Some(raw), has_credentials, true, false);
                assert!(
                    resolved.is_err(),
                    "MAIL_SMTP_ENCRYPTION={raw} (credentials: {has_credentials}) \
                     must refuse to boot in production, but resolved to \
                     {resolved:?}"
                );
            }
        }

        // And the case that actually ships: a production deploy that set
        // MAIL_DRIVER=smtp and never wired MAIL_SMTP_USER / MAIL_SMTP_PASS.
        // This is the arm that used to log a warning and boot anyway.
        let resolved = resolve_smtp_encryption(None, false, true, false);
        assert!(
            resolved.is_err(),
            "MAIL_DRIVER=smtp with no credentials in production must refuse to \
             boot, but resolved to {resolved:?}"
        );
    }

    /// Split out so the message itself is asserted — an operator who hits
    /// this needs to be told which variable unblocks them.
    #[test]
    fn the_production_refusal_names_the_override_and_the_alternatives() {
        let err = resolve_smtp_encryption(None, false, true, false)
            .expect_err("production plaintext SMTP must refuse to boot");
        let msg = format!("{err}");

        assert!(
            msg.contains(ALLOW_INSECURE_SMTP_ENV),
            "the refusal must name the override that unblocks it: {msg}"
        );
        assert!(
            msg.contains("MAIL_SMTP_USER") && msg.contains("MAIL_SMTP_PASS"),
            "and the variables that fix it properly: {msg}"
        );
        assert!(
            msg.contains("cleartext"),
            "and say plainly what is wrong: {msg}"
        );
    }

    #[test]
    fn the_production_override_is_honoured_when_explicitly_set() {
        let resolved = resolve_smtp_encryption(None, false, true, true)
            .expect("the override exists precisely to permit this");
        assert_eq!(resolved, SmtpEncryption::Plaintext);
    }

    /// Implicit TLS was written and tested but unreachable from
    /// configuration — `bootstrap_from_env` only ever built `starttls` or
    /// `unencrypted`. An operator on a 465-only relay had no working
    /// combination of environment variables.
    #[test]
    fn implicit_tls_is_reachable_from_configuration() {
        for raw in ["tls", "ssl", "implicit", "TLS", "  Ssl  "] {
            assert_eq!(
                encryption(Some(raw), true, true),
                SmtpEncryption::Tls,
                "MAIL_SMTP_ENCRYPTION={raw} must select implicit TLS"
            );
        }
    }

    #[test]
    fn starttls_can_be_named_explicitly_and_is_case_insensitive() {
        for raw in ["starttls", "STARTTLS", " StartTls "] {
            assert_eq!(encryption(Some(raw), true, true), SmtpEncryption::StartTls);
        }
    }

    /// An encrypted mode is never blocked by the production guard, even
    /// with no credentials — the missing-credentials refusal is a
    /// different error raised by the caller, with a different message.
    #[test]
    fn an_encrypted_mode_passes_the_production_guard_without_credentials() {
        assert_eq!(
            encryption(Some("starttls"), false, true),
            SmtpEncryption::StartTls
        );
        assert_eq!(encryption(Some("tls"), false, true), SmtpEncryption::Tls);
    }

    /// A typo must not silently become plaintext, and must not wait for a
    /// production deploy to say so. `tsl` is the one that matters — it is
    /// a transposition of an encrypted mode, so the operator believes
    /// they asked for TLS.
    #[test]
    fn an_unrecognised_value_fails_in_every_environment() {
        for is_production in [false, true] {
            for raw in ["tsl", "yes", "on", "starttls!", "TLSv1.2"] {
                let err = resolve_smtp_encryption(Some(raw), true, is_production, false)
                    .expect_err(
                        "an unrecognised MAIL_SMTP_ENCRYPTION must fail rather \
                         than degrade to a default",
                    );
                let msg = format!("{err}");
                assert!(
                    msg.contains(raw),
                    "the error must quote the offending value — it is usually \
                     the typo itself: {msg}"
                );
                assert!(
                    msg.contains("starttls") && msg.contains("tls") && msg.contains("none"),
                    "and list what is accepted: {msg}"
                );
            }
        }
    }

    /// A blank value is how `MAIL_SMTP_ENCRYPTION=` in a `.env` file and an
    /// unsubstituted template variable both arrive. Treat it as unset
    /// rather than as an unrecognised value.
    #[test]
    fn a_blank_value_is_treated_as_unset() {
        assert_eq!(
            encryption(Some(""), false, false),
            SmtpEncryption::Plaintext
        );
        assert_eq!(
            encryption(Some("   "), true, false),
            SmtpEncryption::StartTls
        );
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
    fn file_driver_does_not_deliver() {
        assert!(!MailDriver::File.delivers());
    }

    #[test]
    fn file_driver_parses_and_round_trips() {
        assert_eq!(MailDriver::parse("file"), Some(MailDriver::File));
        assert_eq!(MailDriver::File.as_str(), "file");
    }

    #[test]
    fn production_refuses_the_file_driver_without_the_override() {
        let err = select_driver(Some("file"), true, false)
            .expect_err("production must refuse a non-delivering driver");
        assert!(
            err.to_string().contains("refusing to boot in production"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn production_accepts_the_file_driver_with_the_override() {
        let s = select_driver(Some("file"), true, true).expect("override acknowledges the risk");
        assert_eq!(s.driver, MailDriver::File);
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
