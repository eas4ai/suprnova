use crate::config::env::{env, env_optional, env_strict};
use crate::error::FrameworkError;
use crate::http::body::DEFAULT_MAX_REQUEST_BODY_BYTES;

/// Default listen port when neither `SERVER_PORT` nor `PORT` is set.
///
/// Chosen to be distinctive - `8080`/`8000` collide with nearly every
/// other dev server and proxy on a typical machine. `8765` (an 8-7-6-5
/// countdown) is rarely squatted; the `8xxx` prefix keeps it readable as
/// a backend port. The matching Vite default is `5765`
/// ([`crate::inertia::config::DEFAULT_VITE_PORT`]).
pub const DEFAULT_SERVER_PORT: u16 = 8765;

/// Default header-read timeout (seconds) when `SERVER_HEADER_READ_TIMEOUT`
/// is unset, blank, zero, or unparseable.
///
/// Hyper documents a 30s default for this deadline, but - critically -
/// that default only arms when a [`Timer`](hyper::rt::Timer) is installed
/// on the connection builder. Before `Server::run` started installing
/// `hyper_util::rt::TokioTimer`, the "default" was silently inert: hyper
/// logged a warning and enforced no deadline at all, so a client that
/// opened a connection and sent an incomplete request head could hold it
/// (and, with `SERVER_MAX_CONNECTIONS` set, a semaphore permit) forever
/// (SEC-07, a slowloris-style exhaustion). This constant surfaces the
/// same 30s figure as an explicit, operator-configurable value instead
/// of leaving it as an implicit - and previously inactive - default.
pub const DEFAULT_HEADER_READ_TIMEOUT_SECS: u64 = 30;

/// Finite fallback for `SERVER_MAX_CONNECTIONS` when the env var is set
/// but invalid (unparseable) or zero.
///
/// An operator who sets this knob is explicitly asking for a connection
/// cap. Silently falling back to "unbounded" on a typo defeats the whole
/// point of the knob and, combined with SEC-07, means a handful of
/// stalled connections could exhaust file descriptors while the operator
/// believes a cap is in effect. Chosen high enough not to bite a
/// legitimate high-traffic deployment that meant to type a larger
/// number, low enough to still bound resource usage as a backstop.
pub const DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION: usize = 10_000;

/// Server configuration.
///
/// `max_body_size` is honoured: `Server::from_config` calls
/// [`crate::http::body::set_global_max_request_body_bytes`] with this
/// value during boot, so `SERVER_MAX_BODY_SIZE=...` in the env actually
/// changes the request body cap. Per-`FormRequest::max_body_bytes`
/// overrides still take precedence on individual endpoints.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server host address.
    pub host: String,
    /// Server port.
    pub port: u16,
    /// Maximum request body size in bytes.
    ///
    /// Defaults to [`DEFAULT_MAX_REQUEST_BODY_BYTES`] (8 MiB). Override
    /// via `SERVER_MAX_BODY_SIZE` in the environment. The configured
    /// value is wired into the process-global body cap at boot time;
    /// per-FormRequest `max_body_bytes` overrides still apply on
    /// individual endpoints.
    pub max_body_size: usize,
    /// Optional cap on the number of concurrently active TCP connections.
    ///
    /// When `Some(n)`, the server acquires a semaphore permit for each
    /// accepted connection and holds it until the connection closes;
    /// once all `n` permits are taken the accept loop blocks until an
    /// existing connection ends. When `None` (the default), behaviour is
    /// unchanged - connections are unbounded.
    ///
    /// Set via `SERVER_MAX_CONNECTIONS` in the environment. Blank or unset
    /// is treated as `None` (unbounded) - that's an intentional choice, not
    /// a misconfiguration. An unparseable or zero value, by contrast, means
    /// the operator DID ask for a cap and got it wrong; that falls back to
    /// [`DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION`] (with a
    /// `tracing::warn!`) rather than silently reverting to unbounded. This
    /// is a backstop against runaway connection accumulation; pair it with
    /// a reverse proxy and an appropriate `LimitNOFILE` for full protection.
    pub max_connections: Option<usize>,
    /// Deadline for reading a client's complete request head (start line +
    /// headers) before the connection is closed.
    ///
    /// Defaults to [`DEFAULT_HEADER_READ_TIMEOUT_SECS`] (30s). Override via
    /// `SERVER_HEADER_READ_TIMEOUT` (seconds) in the environment. This is
    /// the SEC-07 slowloris mitigation: `Server::run` installs a
    /// `hyper_util::rt::TokioTimer` on every connection and passes this
    /// value to hyper's `header_read_timeout`, so a client that opens a
    /// connection and never completes its request head is dropped instead
    /// of held indefinitely. Only bounds header parsing - it does not
    /// apply to already-established WebSocket/SSE connections.
    pub header_read_timeout: std::time::Duration,
    /// Optional shared secret required to reach the *readiness* half of
    /// the built-in health endpoint.
    ///
    /// Set via `SERVER_HEALTH_READINESS_TOKEN`; blank or unset is `None`.
    ///
    /// When `None` (the default), readiness is public - which is the
    /// behaviour every deployment guide in `manual/` documents, and the
    /// behaviour the generated Docker `HEALTHCHECK`, the Railway
    /// `healthcheckPath`, and the DigitalOcean app spec all depend on.
    /// Changing that default would break them, so it stays open unless an
    /// operator closes it.
    ///
    /// When `Some(token)`, any request that would probe a dependency
    /// (`/_suprnova/health/ready`, or `/_suprnova/health?db=true`) must
    /// carry `X-Suprnova-Health-Token: <token>` or it is answered 404 -
    /// not 401, so the readiness surface is invisible rather than merely
    /// closed. Liveness stays public either way: a probe that needs no
    /// secret is one less credential in a k8s manifest.
    ///
    /// Worth closing when the endpoint is internet-reachable. Readiness
    /// runs a database round trip for whoever asks, which makes it both a
    /// free liveness oracle for your dependencies and a small amount of
    /// work an anonymous caller can ask the process to do on demand.
    pub health_readiness_token: Option<String>,
}

impl ServerConfig {
    /// Build config from environment variables. The default for
    /// `max_body_size` is [`DEFAULT_MAX_REQUEST_BODY_BYTES`] so the
    /// "no env var set" case matches the compile-time fallback used
    /// by [`crate::http::body::global_max_request_body_bytes`] before
    /// boot wires the runtime value in.
    ///
    /// This helper is intentionally lenient - a typed env var that
    /// fails to parse falls back to the default (with a
    /// `tracing::warn!`). It is invoked from `impl Default` and other
    /// infallible paths. The strict, boot-failing variant is
    /// [`Self::try_from_env`]; `Config::init` calls that.
    pub fn from_env() -> Self {
        Self {
            host: env("SERVER_HOST", "127.0.0.1".to_string()),
            port: resolve_port_lenient(),
            max_body_size: env("SERVER_MAX_BODY_SIZE", DEFAULT_MAX_REQUEST_BODY_BYTES),
            max_connections: parse_max_connections(
                std::env::var("SERVER_MAX_CONNECTIONS").ok().as_deref(),
            ),
            header_read_timeout: resolve_header_read_timeout(
                std::env::var("SERVER_HEADER_READ_TIMEOUT").ok().as_deref(),
            ),
            health_readiness_token: parse_health_readiness_token(
                std::env::var("SERVER_HEALTH_READINESS_TOKEN")
                    .ok()
                    .as_deref(),
            ),
        }
    }

    /// Build config from environment variables, returning an error if
    /// any typed knob is set to a value that fails to parse. Used by
    /// `Config::init` so a typo in `SERVER_PORT` aborts boot instead
    /// of silently reverting to the default.
    pub fn try_from_env() -> Result<Self, FrameworkError> {
        let host = env_strict::<String>("SERVER_HOST")?.unwrap_or_else(|| "127.0.0.1".to_string());
        // Precedence: SERVER_PORT (explicit) > PORT (PaaS convention -
        // Heroku/Railway/Render/Fly inject it) > distinctive default.
        let port = match env_strict::<u16>("SERVER_PORT")? {
            Some(p) => p,
            None => env_strict::<u16>("PORT")?.unwrap_or(DEFAULT_SERVER_PORT),
        };
        let max_body_size =
            env_strict::<usize>("SERVER_MAX_BODY_SIZE")?.unwrap_or(DEFAULT_MAX_REQUEST_BODY_BYTES);
        // `SERVER_MAX_CONNECTIONS` is intentionally lenient even in the
        // strict path: a typo here should not abort boot, just log and
        // fall back to a finite safe default via parse_max_connections.
        let max_connections =
            parse_max_connections(std::env::var("SERVER_MAX_CONNECTIONS").ok().as_deref());
        // `SERVER_HEADER_READ_TIMEOUT` is likewise lenient: it's an
        // optional hardening knob, and an invalid value already degrades
        // to the SAFE default (30s) rather than to "no timeout" - there is
        // no unsafe fallback to guard against here, so a typo need not
        // abort boot.
        let header_read_timeout = resolve_header_read_timeout(
            std::env::var("SERVER_HEADER_READ_TIMEOUT").ok().as_deref(),
        );
        // Likewise lenient, and for the same reason as the two above: a
        // blank or absent secret means "readiness is public", which is the
        // documented default, not a misconfiguration to abort boot over.
        let health_readiness_token = parse_health_readiness_token(
            std::env::var("SERVER_HEALTH_READINESS_TOKEN")
                .ok()
                .as_deref(),
        );
        Ok(Self {
            host,
            port,
            max_body_size,
            max_connections,
            header_read_timeout,
            health_readiness_token,
        })
    }

    /// Create a builder for customizing config
    pub fn builder() -> ServerConfigBuilder {
        ServerConfigBuilder::default()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Resolve the listen port for the lenient (non-boot) path.
///
/// Precedence mirrors [`ServerConfig::try_from_env`]: `SERVER_PORT` >
/// `PORT` (PaaS convention) > [`DEFAULT_SERVER_PORT`]. Unparseable values
/// are treated as unset here (the strict `try_from_env` path is what
/// fails boot on a typo).
fn resolve_port_lenient() -> u16 {
    env_optional::<u16>("SERVER_PORT")
        .or_else(|| env_optional::<u16>("PORT"))
        .unwrap_or(DEFAULT_SERVER_PORT)
}

/// Parse the optional `SERVER_MAX_CONNECTIONS` cap from a raw env-var
/// string.
///
/// A blank or absent value is treated as unset (`None` = unbounded) -
/// that is the intentional, documented default; nobody asked for a cap.
/// An unparseable or zero value is different: the operator DID set this
/// knob and got it wrong, so falling back to `None` would silently
/// discard the cap they asked for. That case instead falls back to
/// [`DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION`] with a
/// `tracing::warn!`, rather than a boot error - it remains an optional
/// hardening knob, so a typo still doesn't prevent the server from
/// starting, it just doesn't silently disappear either.
pub(crate) fn parse_max_connections(raw: Option<&str>) -> Option<usize> {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty())?;
    match trimmed.parse::<usize>() {
        Ok(n) if n > 0 => Some(n),
        Ok(_) | Err(_) => {
            tracing::warn!(
                raw_value = trimmed,
                fallback = DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION,
                "SERVER_MAX_CONNECTIONS is set but invalid or zero; falling back to a finite \
                 safe default instead of leaving connections unbounded - unbounded is not a \
                 safe interpretation of a misconfigured limit."
            );
            Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
        }
    }
}

/// Resolve `SERVER_HEADER_READ_TIMEOUT` (seconds) from a raw env-var
/// string into a [`Duration`](std::time::Duration).
///
/// Unlike `SERVER_MAX_CONNECTIONS`, there is no "unset" special case to
/// preserve here: a blank/absent value falls back to
/// [`DEFAULT_HEADER_READ_TIMEOUT_SECS`], and so does an unparseable or
/// zero value - zero would mean "no timeout," which is exactly the
/// SEC-07 hole this knob exists to close, so it is treated the same as
/// any other invalid input rather than honored as "disable."
pub(crate) fn resolve_header_read_timeout(raw: Option<&str>) -> std::time::Duration {
    let trimmed = raw.map(str::trim).filter(|s| !s.is_empty());
    let secs = match trimmed {
        None => DEFAULT_HEADER_READ_TIMEOUT_SECS,
        Some(s) => match s.parse::<u64>() {
            Ok(n) if n > 0 => n,
            Ok(_) => {
                tracing::warn!(
                    "SERVER_HEADER_READ_TIMEOUT=0 would disable the SEC-07 header-read \
                     deadline entirely; falling back to the {DEFAULT_HEADER_READ_TIMEOUT_SECS}s \
                     default."
                );
                DEFAULT_HEADER_READ_TIMEOUT_SECS
            }
            Err(_) => {
                tracing::warn!(
                    raw_value = s,
                    "SERVER_HEADER_READ_TIMEOUT is set but failed to parse; falling back to \
                     the {DEFAULT_HEADER_READ_TIMEOUT_SECS}s default."
                );
                DEFAULT_HEADER_READ_TIMEOUT_SECS
            }
        },
    };
    std::time::Duration::from_secs(secs)
}

/// Trim `SERVER_HEALTH_READINESS_TOKEN` into an optional secret.
///
/// Blank is `None` rather than `Some("")`: an empty secret would compare
/// equal to an empty header and gate nothing while *looking* configured,
/// which is the worst of both outcomes. `export TOKEN=` in a shell script
/// or an unsubstituted value in a k8s manifest both land here, so the
/// distinction is not hypothetical.
///
/// Surrounding whitespace is trimmed because these arrive through YAML
/// block scalars and `.env` files, where a trailing space is invisible and
/// would otherwise make every probe 404 with nothing to see in a diff.
pub(crate) fn parse_health_readiness_token(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Builder for ServerConfig
#[derive(Default)]
pub struct ServerConfigBuilder {
    host: Option<String>,
    port: Option<u16>,
    max_body_size: Option<usize>,
    max_connections: Option<usize>,
    header_read_timeout: Option<std::time::Duration>,
    health_readiness_token: Option<String>,
}

impl ServerConfigBuilder {
    /// Set the server host
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Set the server port
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set the maximum request body size in bytes
    pub fn max_body_size(mut self, size: usize) -> Self {
        self.max_body_size = Some(size);
        self
    }

    /// Cap the number of concurrently active connections.
    ///
    /// When set, the server will not accept new connections once this
    /// many are active; the accept loop blocks until an existing
    /// connection closes. When unset, connections are unbounded.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.max_connections = Some(n);
        self
    }

    /// Override the header-read timeout (see
    /// [`ServerConfig::header_read_timeout`] - the SEC-07 slowloris
    /// mitigation). Default: [`DEFAULT_HEADER_READ_TIMEOUT_SECS`] (30s).
    pub fn header_read_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.header_read_timeout = Some(timeout);
        self
    }

    /// Require a shared secret on the readiness half of the built-in
    /// health endpoint (see [`ServerConfig::health_readiness_token`]).
    ///
    /// Callers must then send `X-Suprnova-Health-Token: <token>` to reach
    /// `/_suprnova/health/ready` or `/_suprnova/health?db=true`; without
    /// it they get a 404. Liveness stays public.
    pub fn health_readiness_token(mut self, token: impl Into<String>) -> Self {
        self.health_readiness_token = Some(token.into());
        self
    }

    /// Build the ServerConfig
    pub fn build(self) -> ServerConfig {
        let default = ServerConfig::from_env();
        ServerConfig {
            host: self.host.unwrap_or(default.host),
            port: self.port.unwrap_or(default.port),
            max_body_size: self.max_body_size.unwrap_or(default.max_body_size),
            max_connections: self.max_connections.or(default.max_connections),
            header_read_timeout: self
                .header_read_timeout
                .unwrap_or(default.header_read_timeout),
            health_readiness_token: self
                .health_readiness_token
                .or(default.health_readiness_token),
        }
    }
}

#[cfg(test)]
mod tests {
    //! `ServerConfig::from_env`'s default for `max_body_size` must
    //! match the body-collector's compile-time default so a missing
    //! env var doesn't silently change the cap.
    //!
    //! Note: we don't assert on env-var-driven values here because tests
    //! share a process env and `SERVER_MAX_BODY_SIZE` could leak in from
    //! another test. The default-alignment invariant is what matters for
    //! the audit regression.

    use super::*;

    #[test]
    fn parses_max_connections_from_env() {
        // unset / blank → None (unbounded - the intentional default; no
        // cap was requested, so there's nothing to fall back "safely" from).
        assert_eq!(parse_max_connections(None), None);
        assert_eq!(parse_max_connections(Some("")), None);
        // set → Some(n)
        assert_eq!(parse_max_connections(Some("1024")), Some(1024));
        // unparseable / zero → an explicit cap was requested and botched,
        // so fall back to the finite safe default (SEC-07) instead of
        // silently reverting to unbounded.
        assert_eq!(
            parse_max_connections(Some("abc")),
            Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
        );
        assert_eq!(
            parse_max_connections(Some("0")),
            Some(DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION)
        );
        // whitespace-padded value is accepted
        assert_eq!(parse_max_connections(Some("  512  ")), Some(512));
    }

    #[test]
    fn parses_health_readiness_token_from_env() {
        // unset / blank → None → readiness stays public, which is what
        // every deployment guide in `manual/` documents.
        assert_eq!(parse_health_readiness_token(None), None);
        assert_eq!(parse_health_readiness_token(Some("")), None);
        // Whitespace-only is blank too. `TOKEN=" "` in a .env file must not
        // produce a secret that no probe can ever send.
        assert_eq!(parse_health_readiness_token(Some("   ")), None);
        // Set → Some, with surrounding whitespace stripped. A trailing
        // newline from a YAML block scalar is invisible in a diff and would
        // otherwise 404 every probe.
        assert_eq!(
            parse_health_readiness_token(Some("s3cret")),
            Some("s3cret".to_string())
        );
        assert_eq!(
            parse_health_readiness_token(Some("  s3cret\n")),
            Some("s3cret".to_string())
        );
    }

    #[test]
    fn resolves_header_read_timeout_from_env() {
        // unset / blank → the safe default (30s).
        assert_eq!(
            resolve_header_read_timeout(None),
            std::time::Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS)
        );
        assert_eq!(
            resolve_header_read_timeout(Some("")),
            std::time::Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS)
        );
        // set → the configured value.
        assert_eq!(
            resolve_header_read_timeout(Some("5")),
            std::time::Duration::from_secs(5)
        );
        // unparseable / zero → the safe default, NOT "no timeout". Zero
        // is not honored as "disable" because that would reopen SEC-07.
        assert_eq!(
            resolve_header_read_timeout(Some("abc")),
            std::time::Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS)
        );
        assert_eq!(
            resolve_header_read_timeout(Some("0")),
            std::time::Duration::from_secs(DEFAULT_HEADER_READ_TIMEOUT_SECS)
        );
        // whitespace-padded value is accepted
        assert_eq!(
            resolve_header_read_timeout(Some("  7  ")),
            std::time::Duration::from_secs(7)
        );
    }

    #[test]
    #[serial_test::serial(server_config_env)]
    fn try_from_env_rejects_unparseable_port() {
        // `SERVER_PORT=abc` must fail boot via `try_from_env`, not
        // silently fall back to the default the way the lenient
        // `from_env` path does. This is the boot-time fail-loud
        // guarantee `Config::init` relies on.
        let prior = std::env::var("SERVER_PORT").ok();
        // SAFETY: this test mutates a process-global env var. Other
        // tests in this crate use the same single-threaded pattern;
        // we restore the prior value at the end.
        unsafe {
            std::env::set_var("SERVER_PORT", "abc");
        }
        let err = ServerConfig::try_from_env().expect_err("unparseable port must error");
        let msg = format!("{}", err);
        assert!(
            msg.contains("SERVER_PORT"),
            "error should name the env var: {:?}",
            msg
        );
        unsafe {
            match prior {
                Some(v) => std::env::set_var("SERVER_PORT", v),
                None => std::env::remove_var("SERVER_PORT"),
            }
        }
    }

    #[test]
    #[serial_test::serial(server_config_env)]
    fn default_max_body_size_matches_body_module_default() {
        // Unset the var so from_env hits its default path. Use a unique
        // scope guard so we don't disturb other tests on the same
        // process: stash the prior value, clear, run assertion, restore.
        let prior = std::env::var("SERVER_MAX_BODY_SIZE").ok();
        // SAFETY: tests run single-threaded for this scope only because
        // we don't await across the modification; module-level config
        // env-var tests in the rest of the crate use the same pattern.
        unsafe {
            std::env::remove_var("SERVER_MAX_BODY_SIZE");
        }
        let config = ServerConfig::from_env();
        assert_eq!(
            config.max_body_size, DEFAULT_MAX_REQUEST_BODY_BYTES,
            "ServerConfig default must match the body collector's \
             DEFAULT_MAX_REQUEST_BODY_BYTES - divergent defaults caused \
             SERVER_MAX_BODY_SIZE to be a dead knob"
        );
        // Restore prior env state for sibling tests.
        if let Some(v) = prior {
            unsafe {
                std::env::set_var("SERVER_MAX_BODY_SIZE", v);
            }
        }
    }

    #[test]
    #[serial_test::serial(server_config_env)]
    fn port_precedence_server_port_then_port_then_default() {
        let prior_server = std::env::var("SERVER_PORT").ok();
        let prior_port = std::env::var("PORT").ok();

        // SAFETY: single-threaded scope (no await across the mutation),
        // restored at the end - same pattern as the sibling env tests.
        unsafe {
            std::env::remove_var("SERVER_PORT");
            std::env::remove_var("PORT");
        }

        // Neither set → distinctive default.
        assert_eq!(ServerConfig::from_env().port, DEFAULT_SERVER_PORT);
        assert_eq!(
            ServerConfig::try_from_env().unwrap().port,
            DEFAULT_SERVER_PORT
        );

        // Only PORT set (PaaS: Heroku/Railway/Render/Fly inject it).
        unsafe {
            std::env::set_var("PORT", "4321");
        }
        assert_eq!(ServerConfig::from_env().port, 4321);
        assert_eq!(ServerConfig::try_from_env().unwrap().port, 4321);

        // SERVER_PORT wins over PORT when both are set.
        unsafe {
            std::env::set_var("SERVER_PORT", "9999");
        }
        assert_eq!(ServerConfig::from_env().port, 9999);
        assert_eq!(ServerConfig::try_from_env().unwrap().port, 9999);

        unsafe {
            match prior_server {
                Some(v) => std::env::set_var("SERVER_PORT", v),
                None => std::env::remove_var("SERVER_PORT"),
            }
            match prior_port {
                Some(v) => std::env::set_var("PORT", v),
                None => std::env::remove_var("PORT"),
            }
        }
    }
}
