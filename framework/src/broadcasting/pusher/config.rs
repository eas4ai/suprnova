//! Configuration for the Pusher-protocol driver.
//!
//! One shape serves Pusher Channels, Soketi and Laravel Reverb: they
//! speak the same protocol and differ only in where they live.

use crate::FrameworkError;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::env::VarError;
use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

/// The transport scheme of a self-hosted Pusher-protocol server.
///
/// Only consulted when the `host` of a [`PusherConfig`] is set. Pusher
/// Channels itself is always reached over HTTPS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PusherScheme {
    /// Plain HTTP. The port defaults to 80.
    Http,
    /// HTTPS. The default; the port defaults to 443.
    Https,
}

/// Credentials and endpoint of a Pusher-protocol service: Pusher
/// Channels, Soketi or Laravel Reverb.
///
/// The secret and the encryption master key are private fields so they
/// can only leave this type through the signing and encryption code,
/// and `Debug` prints both as `"[redacted]"`.
///
/// The setters accept any value; [`PusherBroadcastHub::new`](crate::PusherBroadcastHub::new)
/// checks them, so a bad value fails at boot with an error that names
/// the field: `app_id`, `key` and `cluster` must be letters, digits,
/// `_` or `-`; `host` must be a plain hostname or IP address; the
/// secret must not be empty; and `timeout` must be greater than zero.
///
/// ```rust,no_run
/// use suprnova::{PusherConfig, PusherScheme};
/// # fn ex() -> Result<(), suprnova::FrameworkError> {
/// // Pusher Channels, from PUSHER_* variables.
/// let pusher = PusherConfig::from_env()?;
///
/// // A local Soketi server, built by hand.
/// let soketi = PusherConfig::new("app-id", "app-key", "app-secret")
///     .host("127.0.0.1")
///     .port(6001)
///     .scheme(PusherScheme::Http);
/// # let _ = (pusher, soketi);
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct PusherConfig {
    /// The app id, as the service issued it. It is part of the REST
    /// path `/apps/{app_id}/events`.
    pub app_id: String,
    /// The public app key. Clients connect with it, and every
    /// authorization string starts with it.
    pub key: String,
    /// The app secret. It signs every request and every authorization.
    secret: String,
    /// The host of a self-hosted server (Soketi, Reverb). When unset,
    /// the driver talks to Pusher Channels in `cluster`.
    pub host: Option<String>,
    /// The port of a self-hosted server. When unset it follows
    /// `scheme`: 443 for HTTPS, 80 for HTTP.
    pub port: Option<u16>,
    /// The scheme of a self-hosted server. Ignored without a host.
    pub scheme: PusherScheme,
    /// The Pusher Channels cluster, `mt1` by default. Ignored when a
    /// host is set.
    pub cluster: String,
    /// The key that end-to-end encrypted channels derive their
    /// per-channel secrets from. Without it, publishing to an encrypted
    /// channel fails instead of sending plaintext.
    encryption_master_key: Option<[u8; 32]>,
    /// How long one REST call may take before publishing fails, so a
    /// stalled service cannot hold a dispatch open.
    pub timeout: Duration,
}

impl PusherConfig {
    /// A configuration for one app, with every other setting at its
    /// default: Pusher Channels cluster `mt1` over HTTPS, no encryption
    /// master key, and a 5 second timeout.
    pub fn new(
        app_id: impl Into<String>,
        key: impl Into<String>,
        secret: impl Into<String>,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            key: key.into(),
            secret: secret.into(),
            host: None,
            port: None,
            scheme: PusherScheme::Https,
            cluster: "mt1".to_string(),
            encryption_master_key: None,
            timeout: Duration::from_secs(5),
        }
    }

    /// Point the driver at a self-hosted server instead of Pusher
    /// Channels.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Set the port of a self-hosted server.
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the scheme of a self-hosted server.
    pub fn scheme(mut self, scheme: PusherScheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// Set the Pusher Channels cluster.
    pub fn cluster(mut self, cluster: impl Into<String>) -> Self {
        self.cluster = cluster.into();
        self
    }

    /// Set how long one REST call may take. A zero timeout would fail
    /// every request, so building the hub refuses it.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the encryption master key from its base64 form, the form
    /// Pusher's dashboard and Laravel's `.env` use.
    ///
    /// # Errors
    ///
    /// The key must decode to exactly 32 bytes. The error never quotes
    /// the input, because any part of it is key material.
    pub fn encryption_master_key_base64(mut self, encoded: &str) -> Result<Self, FrameworkError> {
        self.encryption_master_key = Some(decode_master_key(encoded).ok_or_else(|| {
            FrameworkError::internal(
                "the Pusher encryption master key must be base64 that decodes to exactly 32 bytes",
            )
        })?);
        Ok(self)
    }

    /// Read the configuration from the `PUSHER_*` variables Laravel
    /// uses. Same as [`from_env_prefix("PUSHER")`](Self::from_env_prefix).
    ///
    /// # Errors
    ///
    /// See [`from_env_prefix`](Self::from_env_prefix).
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_env_prefix("PUSHER")
    }

    /// Read the configuration from variables named `{prefix}_*`, so
    /// `from_env_prefix("REVERB")` reads `REVERB_APP_ID` and so on.
    ///
    /// Required, and must not be empty: `{prefix}_APP_ID`,
    /// `{prefix}_APP_KEY`, `{prefix}_APP_SECRET`. Optional:
    /// `{prefix}_HOST`, `{prefix}_PORT`, `{prefix}_SCHEME` (`http` or
    /// `https`), `{prefix}_APP_CLUSTER`,
    /// `{prefix}_ENCRYPTION_MASTER_KEY_BASE64` and
    /// `{prefix}_TIMEOUT_SECS`. An optional variable that is set but
    /// empty counts as unset, because Laravel's `.env.example` ships
    /// them that way.
    ///
    /// # Errors
    ///
    /// A missing required variable or an invalid value is an error that
    /// names the variable. It never quotes the value, so neither the
    /// secret nor the master key can reach a log through it.
    pub fn from_env_prefix(prefix: &str) -> Result<Self, FrameworkError> {
        Self::from_lookup(prefix, |name| std::env::var(name))
    }

    /// Build the configuration from a variable lookup shaped like
    /// [`std::env::var`]. The seam lets the tests cover every variable
    /// without mutating the process environment.
    fn from_lookup(
        prefix: &str,
        lookup: impl Fn(&str) -> Result<String, VarError>,
    ) -> Result<Self, FrameworkError> {
        let read = |suffix: &str| -> Result<(String, Option<String>), FrameworkError> {
            let name = format!("{prefix}_{suffix}");
            match lookup(&name) {
                Ok(value) if value.is_empty() => Ok((name, None)),
                Ok(value) => Ok((name, Some(value))),
                Err(VarError::NotPresent) => Ok((name, None)),
                // `VarError`'s own message quotes the value, so it is
                // never formatted here.
                Err(VarError::NotUnicode(_)) => Err(FrameworkError::internal(format!(
                    "{name} is not valid UTF-8"
                ))),
            }
        };
        let required = |suffix: &str| -> Result<String, FrameworkError> {
            match read(suffix)? {
                (_, Some(value)) => Ok(value),
                (name, None) => Err(FrameworkError::internal(format!(
                    "{name} is not set; the Pusher driver needs it"
                ))),
            }
        };

        let mut config = Self::new(
            required("APP_ID")?,
            required("APP_KEY")?,
            required("APP_SECRET")?,
        );
        if let (_, Some(host)) = read("HOST")? {
            config.host = Some(host);
        }
        if let (name, Some(port)) = read("PORT")? {
            config.port = Some(
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| {
                        FrameworkError::internal(format!(
                            "{name} must be a port number from 1 to 65535"
                        ))
                    })?,
            );
        }
        if let (name, Some(scheme)) = read("SCHEME")? {
            config.scheme = match scheme.to_ascii_lowercase().as_str() {
                "http" => PusherScheme::Http,
                "https" => PusherScheme::Https,
                _ => {
                    return Err(FrameworkError::internal(format!(
                        "{name} must be `http` or `https`"
                    )));
                }
            };
        }
        if let (_, Some(cluster)) = read("APP_CLUSTER")? {
            config.cluster = cluster;
        }
        if let (name, Some(encoded)) = read("ENCRYPTION_MASTER_KEY_BASE64")? {
            config.encryption_master_key = Some(decode_master_key(&encoded).ok_or_else(|| {
                FrameworkError::internal(format!(
                    "{name} must be base64 that decodes to exactly 32 bytes"
                ))
            })?);
        }
        if let (name, Some(timeout)) = read("TIMEOUT_SECS")? {
            let secs = timeout
                .parse::<u64>()
                .ok()
                .filter(|secs| *secs != 0)
                .ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "{name} must be a whole number of seconds greater than 0"
                    ))
                })?;
            config.timeout = Duration::from_secs(secs);
        }
        Ok(config)
    }

    /// The base URL of the REST API: the self-hosted server when a host
    /// is set, otherwise Pusher Channels in the configured cluster.
    pub(crate) fn base_url(&self) -> String {
        match &self.host {
            Some(host) => {
                let (scheme, default_port) = match self.scheme {
                    PusherScheme::Http => ("http", 80),
                    PusherScheme::Https => ("https", 443),
                };
                let port = self.port.unwrap_or(default_port);
                // A bare IPv6 address needs brackets inside a URL.
                if host.parse::<Ipv6Addr>().is_ok() {
                    format!("{scheme}://[{host}]:{port}")
                } else {
                    format!("{scheme}://{host}:{port}")
                }
            }
            None => format!("https://api-{}.pusher.com", self.cluster),
        }
    }

    /// Check every value the driver puts into a URL or a signature, so a
    /// bad one fails when the hub is built instead of on the first
    /// publish.
    ///
    /// `app_id`, `key` and `cluster` must be `[A-Za-z0-9_-]+`: they are
    /// spliced into the REST URL, the signed query and the
    /// `{key}:{signature}` string unescaped, so a `/`, `?`, `&` or `:`
    /// would change what is requested or signed. `host` must be a plain
    /// hostname or IP address for the same reason. The builder setters
    /// stay infallible, so a zero `timeout` (which would fail every
    /// request) and an empty secret (which anyone could sign with) are
    /// refused here too.
    ///
    /// # Errors
    ///
    /// The first invalid field, named. The value is never quoted.
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        let token = |field: &str, value: &str| {
            if !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                Ok(())
            } else {
                Err(FrameworkError::internal(format!(
                    "the Pusher {field} must be one or more letters, digits, `_` or `-`"
                )))
            }
        };
        token("app_id", &self.app_id)?;
        token("key", &self.key)?;
        token("cluster", &self.cluster)?;
        if self.secret.is_empty() {
            return Err(FrameworkError::internal(
                "the Pusher secret must not be empty",
            ));
        }
        if let Some(host) = &self.host
            && !is_valid_host(host)
        {
            return Err(FrameworkError::internal(
                "the Pusher host must be a hostname or an IP address, with no scheme, port, \
                 path, query, fragment or user info",
            ));
        }
        if self.timeout.is_zero() {
            return Err(FrameworkError::internal(
                "the Pusher timeout must be greater than zero",
            ));
        }
        Ok(())
    }

    /// The app secret, for the signing code only.
    pub(crate) fn secret(&self) -> &str {
        &self.secret
    }

    /// The encryption master key, when one is configured.
    pub(crate) fn encryption_master_key(&self) -> Option<&[u8; 32]> {
        self.encryption_master_key.as_ref()
    }
}

/// Whether `host` is an IP address (IPv6 bare or in brackets) or a
/// hostname: dot-separated labels of 1 to 63 letters, digits, `-` or
/// `_` (container names use `_`), with no `-` at either end, 253 bytes
/// at most. Anything else, such as a `/`, `?`, `#`, `@` or `:port`,
/// would change the URL the driver builds.
fn is_valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().is_ok();
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

/// Decode a base64 master key, or `None` unless it is exactly 32 bytes.
///
/// Returns `Option` rather than the decoder's error on purpose: that
/// error quotes the offending byte, which is key material.
fn decode_master_key(encoded: &str) -> Option<[u8; 32]> {
    STANDARD.decode(encoded).ok()?.try_into().ok()
}

impl std::fmt::Debug for PusherConfig {
    /// Hand-written so the secret and the master key print as
    /// `"[redacted]"`: a config logged with `{:?}` must not leak them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PusherConfig")
            .field("app_id", &self.app_id)
            .field("key", &self.key)
            .field("secret", &"[redacted]")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("scheme", &self.scheme)
            .field("cluster", &self.cluster)
            .field(
                "encryption_master_key",
                &self.encryption_master_key.as_ref().map(|_| "[redacted]"),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SECRET: &str = "7ad3773142a6692b25b8";
    const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn lookup_from(vars: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, VarError> + use<> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| vars.get(name).cloned().ok_or(VarError::NotPresent)
    }

    #[test]
    fn pusher_config_from_env_prefix_reads_every_variable() {
        let lookup = lookup_from(&[
            ("REVERB_APP_ID", "3"),
            ("REVERB_APP_KEY", "app-key"),
            ("REVERB_APP_SECRET", SECRET),
            ("REVERB_HOST", "ws.example.test"),
            ("REVERB_PORT", "8080"),
            ("REVERB_SCHEME", "http"),
            ("REVERB_APP_CLUSTER", "eu"),
            ("REVERB_ENCRYPTION_MASTER_KEY_BASE64", MASTER_KEY_B64),
            ("REVERB_TIMEOUT_SECS", "9"),
        ]);
        let config = PusherConfig::from_lookup("REVERB", lookup).expect("valid config");
        assert_eq!(config.app_id, "3");
        assert_eq!(config.key, "app-key");
        assert_eq!(config.secret(), SECRET);
        assert_eq!(config.host.as_deref(), Some("ws.example.test"));
        assert_eq!(config.port, Some(8080));
        assert_eq!(config.scheme, PusherScheme::Http);
        assert_eq!(config.cluster, "eu");
        let expected: Vec<u8> = (0u8..32).collect();
        assert_eq!(
            config.encryption_master_key().map(|k| k.to_vec()),
            Some(expected)
        );
        assert_eq!(config.timeout, Duration::from_secs(9));
        assert_eq!(config.base_url(), "http://ws.example.test:8080");
    }

    #[test]
    fn pusher_config_defaults_apply_when_optional_variables_are_unset_or_empty() {
        let lookup = lookup_from(&[
            ("PUSHER_APP_ID", "3"),
            ("PUSHER_APP_KEY", "app-key"),
            ("PUSHER_APP_SECRET", SECRET),
            // Laravel's `.env.example` ships optional entries empty.
            ("PUSHER_HOST", ""),
            ("PUSHER_PORT", ""),
            ("PUSHER_SCHEME", ""),
        ]);
        let config = PusherConfig::from_lookup("PUSHER", lookup).expect("valid config");
        assert_eq!(config.host, None);
        assert_eq!(config.port, None);
        assert_eq!(config.scheme, PusherScheme::Https);
        assert_eq!(config.cluster, "mt1");
        assert!(config.encryption_master_key().is_none());
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.base_url(), "https://api-mt1.pusher.com");
    }

    #[test]
    fn pusher_config_from_env_prefix_reads_the_process_environment() {
        // No test sets this prefix, so the first required variable is
        // missing. This drives the real `std::env` path without
        // mutating the environment.
        let err = PusherConfig::from_env_prefix("SUPRNOVA_PUSHER_UNSET_PREFIX")
            .expect_err("nothing is set under this prefix");
        assert!(
            err.to_string()
                .contains("SUPRNOVA_PUSHER_UNSET_PREFIX_APP_ID"),
            "the error names the missing variable: {err}"
        );
    }

    #[test]
    fn pusher_config_missing_required_variable_is_named() {
        let lookup = lookup_from(&[("PUSHER_APP_ID", "3"), ("PUSHER_APP_KEY", "app-key")]);
        let err = PusherConfig::from_lookup("PUSHER", lookup).expect_err("secret is missing");
        assert!(
            err.to_string().contains("PUSHER_APP_SECRET"),
            "the error names the variable: {err}"
        );

        let lookup = lookup_from(&[
            ("PUSHER_APP_ID", "3"),
            ("PUSHER_APP_KEY", ""),
            ("PUSHER_APP_SECRET", SECRET),
        ]);
        let err = PusherConfig::from_lookup("PUSHER", lookup).expect_err("key is empty");
        assert!(
            err.to_string().contains("PUSHER_APP_KEY"),
            "an empty required variable is named too: {err}"
        );
    }

    #[test]
    fn pusher_config_rejects_bad_scheme_port_and_timeout() {
        for (name, value) in [
            ("PUSHER_SCHEME", "ftp"),
            ("PUSHER_PORT", "eighty"),
            ("PUSHER_PORT", "0"),
            ("PUSHER_TIMEOUT_SECS", "soon"),
            ("PUSHER_TIMEOUT_SECS", "0"),
        ] {
            let lookup = lookup_from(&[
                ("PUSHER_APP_ID", "3"),
                ("PUSHER_APP_KEY", "app-key"),
                ("PUSHER_APP_SECRET", SECRET),
                (name, value),
            ]);
            let err = PusherConfig::from_lookup("PUSHER", lookup)
                .expect_err("an invalid value is refused");
            let message = err.to_string();
            assert!(message.contains(name), "the error names {name}: {message}");
            assert!(!message.contains(SECRET), "never the secret: {message}");
        }
    }

    #[test]
    fn pusher_config_rejects_a_master_key_that_is_not_32_bytes() {
        // 31 bytes, valid base64.
        let short = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHg==";
        let err = PusherConfig::new("3", "app-key", SECRET)
            .encryption_master_key_base64(short)
            .expect_err("31 bytes is refused");
        assert!(!err.to_string().contains(short), "never the key: {err}");

        // Not base64 at all. The decoder's own error would quote the
        // offending byte, which is key material.
        let garbage = "not*base64*at*all";
        let err = PusherConfig::new("3", "app-key", SECRET)
            .encryption_master_key_base64(garbage)
            .expect_err("garbage is refused");
        let message = err.to_string();
        assert!(!message.contains(garbage), "never the key: {message}");
        assert!(
            !message.contains("42"),
            "never a byte of the key: {message}"
        );

        let lookup = lookup_from(&[
            ("PUSHER_APP_ID", "3"),
            ("PUSHER_APP_KEY", "app-key"),
            ("PUSHER_APP_SECRET", SECRET),
            ("PUSHER_ENCRYPTION_MASTER_KEY_BASE64", short),
        ]);
        let err = PusherConfig::from_lookup("PUSHER", lookup).expect_err("31 bytes is refused");
        let message = err.to_string();
        assert!(
            message.contains("PUSHER_ENCRYPTION_MASTER_KEY_BASE64"),
            "the error names the variable: {message}"
        );
        assert!(!message.contains(short), "never the key: {message}");
    }

    #[test]
    fn pusher_config_debug_redacts_secret_and_master_key() {
        let config = PusherConfig::new("3", "app-key", SECRET)
            .encryption_master_key_base64(MASTER_KEY_B64)
            .expect("32 bytes");
        let debug = format!("{config:?}");
        assert!(!debug.contains(SECRET), "secret leaked: {debug}");
        assert!(
            !debug.contains(MASTER_KEY_B64),
            "master key leaked: {debug}"
        );
        // The decoded bytes 0..32 would print as a list of small integers.
        assert!(!debug.contains("31"), "master key bytes leaked: {debug}");
        assert!(debug.contains("[redacted]"), "{debug}");
        assert!(
            debug.contains("app-key"),
            "the public key stays visible: {debug}"
        );
    }

    #[test]
    fn pusher_config_base_url_defaults_the_port_by_scheme() {
        let https = PusherConfig::new("3", "k", SECRET).host("soketi.test");
        assert_eq!(https.base_url(), "https://soketi.test:443");
        let http = https.clone().scheme(PusherScheme::Http);
        assert_eq!(http.base_url(), "http://soketi.test:80");
        let pusher = PusherConfig::new("3", "k", SECRET).cluster("ap1");
        assert_eq!(pusher.base_url(), "https://api-ap1.pusher.com");
    }

    #[test]
    fn pusher_config_base_url_brackets_a_bare_ipv6_host() {
        let config = PusherConfig::new("3", "k", SECRET)
            .host("::1")
            .port(6001)
            .scheme(PusherScheme::Http);
        assert_eq!(config.base_url(), "http://[::1]:6001");
        let bracketed = config.clone().host("[::1]");
        assert_eq!(bracketed.base_url(), "http://[::1]:6001");
    }

    #[test]
    fn pusher_config_validate_accepts_real_values() {
        for config in [
            PusherConfig::new("3", "278d425bdf160c739803", SECRET),
            PusherConfig::new("my-app_1", "app-key", SECRET).cluster("eu"),
            PusherConfig::new("3", "k", SECRET).host("soketi.internal"),
            PusherConfig::new("3", "k", SECRET).host("soketi_server"),
            PusherConfig::new("3", "k", SECRET).host("127.0.0.1"),
            PusherConfig::new("3", "k", SECRET).host("::1"),
            PusherConfig::new("3", "k", SECRET).host("[2001:db8::1]"),
        ] {
            assert!(config.validate().is_ok(), "{config:?}");
        }
    }

    #[test]
    fn pusher_config_validate_refuses_bad_values_naming_the_field() {
        let base = || PusherConfig::new("3", "k", SECRET);
        let cases = [
            ("app_id", PusherConfig::new("", "k", SECRET)),
            ("app_id", PusherConfig::new("3/../4", "k", SECRET)),
            ("app_id", PusherConfig::new("3?x=1", "k", SECRET)),
            ("key", PusherConfig::new("3", "", SECRET)),
            ("key", PusherConfig::new("3", "k:forged", SECRET)),
            ("key", PusherConfig::new("3", "k&auth_key=x", SECRET)),
            ("cluster", base().cluster("")),
            ("cluster", base().cluster("evil.example/x")),
            ("cluster", base().cluster("mt1.attacker")),
            ("host", base().host("")),
            ("host", base().host("evil.example/path")),
            ("host", base().host("evil.example?x")),
            ("host", base().host("evil.example#x")),
            ("host", base().host("user@evil.example")),
            ("host", base().host("evil.example:8080")),
            ("host", base().host("https://evil.example")),
            ("host", base().host("-bad.example")),
            ("host", base().host("bad..example")),
            ("host", base().host("[not-an-ip]")),
            ("timeout", base().timeout(Duration::ZERO)),
            ("secret", PusherConfig::new("3", "k", "")),
        ];
        for (field, config) in cases {
            let err = config
                .validate()
                .expect_err(&format!("{field} is refused: {config:?}"));
            let message = err.to_string();
            assert!(message.contains(field), "names {field}: {message}");
            assert!(!message.contains(SECRET), "never the secret: {message}");
        }
    }
}
