//! `suprnova ssr:check` — verify the Inertia SSR worker is healthy.
//!
//! HTTP check against the worker's own `/health` route. Every
//! `@inertiajs/{vue3,react,svelte}/server` `createServer()` bundle
//! answers `GET /health` with `{ status: 'OK', timestamp }` / 200 out of
//! the box (`@inertiajs/core/src/server.ts`) — no extra code needed in
//! the SSR entry. Verifying the *application* answered, not just that
//! some listener accepted a TCP handshake, is what Laravel's
//! `Inertia\Ssr\HttpGateway::isHealthy()` does too
//! (`Http::get($this->getUrl('/health'))->successful()`).
//!
//! Use this in CI or your deploy-pipeline smoke tests:
//!
//! ```bash
//! suprnova ssr:start &
//! ./wait-until.sh suprnova ssr:check
//! # ...run e2e tests...
//! ```

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Resolve the SSR worker URL from flag → env → default. Public for
/// test coverage of the precedence chain.
pub(crate) fn resolve_url(flag: Option<String>) -> String {
    flag.or_else(|| std::env::var("SUPRNOVA_SSR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:13714".to_string())
}

/// Parse a URL's host and port for a TCP probe. Returns
/// `Err(reason)` for inputs we can't make sense of. We don't depend on
/// the `url` crate in `suprnova-cli`, so this is a hand-rolled parser
/// targeting the narrow `http[s]://host[:port][/path]` shape.
pub(crate) fn parse_host_port(url: &str) -> Result<(String, u16), String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else {
        return Err("URL must start with http:// or https://".into());
    };
    // Trim trailing path so just "host[:port]" remains.
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.is_empty() {
        return Err("missing host".into());
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p.parse().map_err(|_| format!("invalid port: {p}"))?;
            (h.to_string(), port)
        }
        None => (
            host_port.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return Err("missing host".into());
    }
    Ok((host, port))
}

/// Resolve `addr` without letting a wedged resolver outlive the deadline.
///
/// `to_socket_addrs` is a blocking libc call with no timeout of its own,
/// and it used to run *outside* the probe's budget entirely: the
/// `--timeout` flag only ever bounded the connect. A host whose nameserver
/// blackholes rather than refuses would hang `ssr:check` indefinitely,
/// which is the worst possible behaviour for a command whose documented
/// use is a CI wait-loop.
///
/// There is no timeout-capable resolver in `std`, so the lookup runs on a
/// worker thread and we wait on a channel. If it blows the deadline the
/// thread is left to finish and be reaped at exit — leaking a thread is
/// acceptable for a short-lived CLI, and unavoidable without pulling in an
/// async resolver for one probe.
fn resolve_within(addr: &str, budget: Duration) -> Result<Vec<std::net::SocketAddr>, String> {
    use std::net::ToSocketAddrs;

    let (tx, rx) = std::sync::mpsc::channel();
    let owned = addr.to_string();
    std::thread::spawn(move || {
        let resolved = owned
            .to_socket_addrs()
            .map(|iter| iter.collect::<Vec<_>>())
            .map_err(|e| e.to_string());
        // The receiver may already have given up; that is not an error.
        let _ = tx.send(resolved);
    });

    match rx.recv_timeout(budget) {
        Ok(Ok(addrs)) if addrs.is_empty() => Err(format!("no addresses for {addr}")),
        Ok(Ok(addrs)) => Ok(addrs),
        Ok(Err(e)) => Err(format!("DNS resolution for {addr} failed: {e}")),
        Err(_) => Err(format!(
            "DNS resolution for {addr} did not finish within the timeout"
        )),
    }
}

/// Try every resolved address until one connects or the deadline passes.
///
/// The old code took `socket_addrs.next()` — the *first* address — and
/// reported the host unreachable if it failed. That is wrong for the most
/// ordinary dual-stack setup there is: a host with an AAAA record on a
/// machine with no IPv6 route resolves to the v6 address first, fails, and
/// `ssr:check` reports the worker down while it is listening happily on
/// v4. Trying each address is what every other client does.
///
/// Each attempt gets whatever is left of the budget, so N addresses cannot
/// multiply into N × timeout.
fn connect_within(
    addrs: &[std::net::SocketAddr],
    deadline: std::time::Instant,
) -> Result<std::net::SocketAddr, String> {
    let mut last_error = None;

    for socket in addrs {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(socket, remaining) {
            Ok(_) => return Ok(*socket),
            Err(e) => last_error = Some(format!("{socket}: {e}")),
        }
    }

    Err(match last_error {
        Some(e) => format!("no address accepted a connection (last: {e})"),
        None => "the timeout elapsed before any address could be tried".to_string(),
    })
}

/// Probe the worker, returning `Ok` with the address that answered.
///
/// Split out of [`run`] so the behaviour is testable: `run` itself calls
/// `std::process::exit`, which a test cannot observe.
pub(crate) fn probe(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<std::net::SocketAddr, String> {
    // One deadline for the whole operation — DNS and every connection
    // attempt inside it. Previously DNS was unbounded and each attempt got
    // a fresh full timeout.
    let deadline = std::time::Instant::now() + timeout;
    let addr = format!("{host}:{port}");

    let addrs = resolve_within(&addr, timeout)?;
    connect_within(&addrs, deadline)
}

pub fn run(url: Option<String>, timeout_ms: u64) {
    let url = resolve_url(url);
    let (host, port) = match parse_host_port(&url) {
        Ok(hp) => hp,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(2);
        }
    };

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let addr = match probe(&host, port, Duration::from_millis(timeout_ms)) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("FAIL: SSR worker not reachable at {url} ({e})");
            std::process::exit(1);
        }
    };

    match get_health(addr, &host, deadline) {
        Ok(()) => {
            println!("OK: SSR worker healthy ({url}/health)");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("FAIL: SSR worker at {url} is not healthy ({e})");
            std::process::exit(1);
        }
    }
}

/// Send `GET /health HTTP/1.1` to `addr` and report whether the
/// response status line is 2xx. Blocking, minimal HTTP/1.1 client —
/// `ssr:check` is a short-lived CLI invocation, and pulling in a full
/// HTTP client for one GET would be a heavier dependency than the check
/// warrants.
///
/// `addr` must already be known reachable (from [`probe`]) — this opens
/// a fresh connection to it rather than reusing the one `probe` made,
/// which costs one extra round trip but keeps `probe`'s address-
/// iteration logic untouched and independently testable.
fn get_health(addr: std::net::SocketAddr, host: &str, deadline: Instant) -> Result<(), String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("no time left in the budget for the health request".to_string());
    }

    let mut stream = TcpStream::connect_timeout(&addr, remaining)
        .map_err(|e| format!("connect for health check: {e}"))?;
    stream
        .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
        .map_err(|e| format!("set read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(deadline.saturating_duration_since(Instant::now())))
        .map_err(|e| format!("set write timeout: {e}"))?;

    let request = format!("GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write health request: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read health response: {e}"))?;

    let status_line = response
        .split(|&b| b == b'\n')
        .next()
        .map(|l| String::from_utf8_lossy(l).trim().to_string())
        .unwrap_or_default();

    // "HTTP/1.1 200 OK" -> take the 3-digit code, treat 2xx as healthy.
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());

    match code {
        Some(c) if (200..300).contains(&c) => Ok(()),
        Some(c) => Err(format!("SSR worker returned {c} for GET /health")),
        None => Err(format!(
            "could not parse a status line from: {status_line:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_url_prefers_flag() {
        let r = resolve_url(Some("http://example.com:9000".into()));
        assert_eq!(r, "http://example.com:9000");
    }

    #[test]
    fn resolve_url_falls_back_to_default() {
        if std::env::var("SUPRNOVA_SSR_URL").is_err() {
            let r = resolve_url(None);
            assert_eq!(r, "http://127.0.0.1:13714");
        }
    }

    #[test]
    fn parse_host_port_explicit_port() {
        let (h, p) = parse_host_port("http://127.0.0.1:13714").unwrap();
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 13714);
    }

    #[test]
    fn parse_host_port_https_default_443() {
        let (_, p) = parse_host_port("https://ssr.example.com").unwrap();
        assert_eq!(p, 443);
    }

    #[test]
    fn parse_host_port_http_default_80() {
        let (_, p) = parse_host_port("http://ssr.example.com").unwrap();
        assert_eq!(p, 80);
    }

    #[test]
    fn parse_host_port_rejects_garbage() {
        assert!(parse_host_port("not a url").is_err());
    }
}

#[cfg(test)]
mod probe_tests {
    //! P2-13. Two defects lived in this probe: DNS ran outside the
    //! timeout entirely, and only the first resolved address was tried.
    //!
    //! Addresses here come from RFC 5737 TEST-NET-1 (`192.0.2.0/24`),
    //! which is reserved for documentation and guaranteed not to be
    //! routed. Connecting to one either fails immediately (no route) or
    //! times out — never succeeds — so these tests exercise the failure
    //! paths without depending on the network or on any host being down.

    use super::{connect_within, probe, resolve_within};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::time::{Duration, Instant};

    /// A listener bound to an ephemeral port, kept alive by the caller.
    fn listening() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        (listener, addr)
    }

    /// A local port that nothing is listening on, so connecting to it is
    /// *refused immediately* rather than timing out.
    ///
    /// That distinction matters. TEST-NET-1 addresses are unroutable, so a
    /// connection to one hangs until the deadline — which is what the
    /// deadline test below wants, and exactly what the iteration test does
    /// not: a first address that eats the entire budget proves nothing
    /// about whether a second would have been tried.
    ///
    /// Binding an ephemeral port and dropping it is the obvious way to get
    /// a dead address, and it is the bug this replaces: `drop` hands the
    /// port straight back to the ephemeral pool, a `listening()` in another
    /// test — these run in parallel — is handed the same number, and the
    /// supposedly unreachable address is live. That is how
    /// `a_reachable_address_after_an_unreachable_one_is_still_found` failed
    /// a release gate, reporting the dead address as the one that accepted,
    /// which is precisely what it should do when the address is listening.
    ///
    /// Privileged ports sit outside the ephemeral range and need root to
    /// bind, so no test in this run can claim one. The probe steps over any
    /// a system daemon already holds — sshd on 22 being the obvious one —
    /// and the counter keeps two calls distinct, which
    /// `all_addresses_failing_reports_the_last_error` depends on.
    pub(super) fn refusing() -> SocketAddr {
        static NEXT: AtomicU16 = AtomicU16::new(1);
        loop {
            let port = NEXT.fetch_add(1, Ordering::Relaxed);
            assert!(port < 1024, "no unbound privileged port left to refuse on");
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err() {
                return addr;
            }
        }
    }

    /// The headline regression: a working address after a dead one must
    /// still be found. This is the dual-stack case — an AAAA record on a
    /// host with no IPv6 route — where the probe used to report the
    /// worker down while it was listening perfectly well.
    #[test]
    fn a_reachable_address_after_an_unreachable_one_is_still_found() {
        let (_listener, good) = listening();
        let bad = refusing();

        let deadline = Instant::now() + Duration::from_secs(5);
        let connected = connect_within(&[bad, good], deadline).unwrap_or_else(|e| {
            panic!(
                "the second address was listening and must have been tried; \
                 taking only the first is the defect: {e}"
            )
        });

        assert_eq!(
            connected, good,
            "must report the address that actually accepted"
        );
    }

    /// The single-address happy path still works.
    #[test]
    fn a_reachable_first_address_connects() {
        let (_listener, good) = listening();
        let deadline = Instant::now() + Duration::from_secs(5);

        assert_eq!(connect_within(&[good], deadline).expect("listening"), good);
    }

    /// Every address failing is an error, not a hang — and the message
    /// carries the last failure so an operator can see why.
    #[test]
    fn all_addresses_failing_reports_the_last_error() {
        let a = refusing();
        let b = refusing();

        let deadline = Instant::now() + Duration::from_secs(5);
        let err =
            connect_within(&[a, b], deadline).expect_err("nothing is listening on either port");

        assert!(
            err.contains("no address accepted"),
            "the error must say every address was tried: {err}"
        );
    }

    /// The budget covers the whole address list rather than resetting per
    /// address, so N dead addresses cannot multiply into N × timeout.
    #[test]
    fn the_deadline_bounds_the_whole_address_list() {
        let addrs: Vec<SocketAddr> = (1..=8)
            .map(|n| format!("192.0.2.{n}:9").parse().expect("test-net addr"))
            .collect();

        let budget = Duration::from_millis(500);
        let started = Instant::now();
        let _ = connect_within(&addrs, started + budget);
        let elapsed = started.elapsed();

        assert!(
            elapsed < budget * 3,
            "8 unreachable addresses took {elapsed:?} against a {budget:?} \
             budget; the deadline must span the whole list, not restart for \
             each address"
        );
    }

    /// An empty list must not report success by vacuous truth.
    #[test]
    fn an_empty_address_list_is_an_error() {
        let deadline = Instant::now() + Duration::from_secs(1);
        connect_within(&[], deadline).expect_err("no addresses means no connection");
    }

    /// A name that cannot resolve is an error rather than a hang. `.invalid`
    /// is reserved by RFC 2606 precisely so it never resolves.
    #[test]
    fn an_unresolvable_host_is_reported_not_hung() {
        let err = resolve_within("nonexistent.invalid:80", Duration::from_secs(5))
            .expect_err("`.invalid` is guaranteed never to resolve");

        assert!(
            err.contains("nonexistent.invalid"),
            "the error must name what failed to resolve: {err}"
        );
    }

    /// A literal address resolves without touching a nameserver, so this
    /// pins that the resolve step passes IPs straight through.
    #[test]
    fn a_literal_address_resolves_to_itself() {
        let addrs =
            resolve_within("127.0.0.1:13714", Duration::from_secs(5)).expect("literal address");

        assert_eq!(
            addrs,
            vec!["127.0.0.1:13714".parse::<SocketAddr>().unwrap()]
        );
    }

    /// End to end through `probe`, which is what `run` calls.
    #[test]
    fn probe_reaches_a_listening_worker() {
        let (_listener, addr) = listening();

        probe("127.0.0.1", addr.port(), Duration::from_secs(5))
            .expect("a listening worker must probe OK");
    }

    #[test]
    fn probe_fails_on_a_closed_port_within_the_budget() {
        // `refusing()`, not bind-then-drop. This test used the latter and
        // flaked roughly once in sixty runs of the suite: `drop` returns the
        // port to the ephemeral pool, a `listening()` in a sibling test —
        // these run in parallel — is handed the same number, and the probe
        // connects to a port this test believes it closed. `is_err` then
        // fails, which is the correct answer to the question the test
        // accidentally asked. See `refusing()` above, which exists because
        // the identical race already failed a release gate once.
        let addr = refusing();

        let started = Instant::now();
        let result = probe("127.0.0.1", addr.port(), Duration::from_millis(800));

        assert!(result.is_err(), "a closed port must not probe OK");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "must fail within the budget, not hang"
        );
    }
}

#[cfg(test)]
mod health_tests {
    //! T31. `ssr:check` upgrades from "something answered on the port"
    //! to "the SSR worker's own `/health` route said OK" — mirroring
    //! Laravel's `HttpGateway::isHealthy()`.

    use super::get_health;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::time::{Duration, Instant};

    /// Spawn a one-shot fake HTTP server that reads a request off the
    /// first accepted connection (discarding it) and writes back
    /// `response` verbatim. Returns its address.
    fn fake_http_server(response: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf); // drain the request; content unchecked
                let _ = stream.write_all(response.as_bytes());
            }
        });
        addr
    }

    #[test]
    fn a_200_response_is_healthy() {
        let addr =
            fake_http_server("HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"OK\"}");
        let deadline = Instant::now() + Duration::from_secs(5);
        get_health(addr, "127.0.0.1", deadline).expect("2xx must report healthy");
    }

    #[test]
    fn a_500_response_is_unhealthy() {
        let addr =
            fake_http_server("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
        let deadline = Instant::now() + Duration::from_secs(5);
        let err = get_health(addr, "127.0.0.1", deadline).expect_err("5xx must not report healthy");
        assert!(err.contains("500"), "error names the status: {err}");
    }

    #[test]
    fn garbage_is_reported_not_panicked() {
        let addr = fake_http_server("not an http response at all");
        let deadline = Instant::now() + Duration::from_secs(5);
        get_health(addr, "127.0.0.1", deadline)
            .expect_err("an unparseable status line must be a clean error, not a panic");
    }

    #[test]
    fn nothing_listening_is_a_connect_error() {
        // Reuses `probe_tests::refusing()` rather than bind-then-drop:
        // an ephemeral port freed by `drop` can be handed straight back
        // out to a `listening()` in a sibling test running in parallel —
        // the exact race `refusing()` exists to avoid (see its doc
        // comment in `probe_tests`).
        let addr = super::probe_tests::refusing();
        let deadline = Instant::now() + Duration::from_secs(5);
        get_health(addr, "127.0.0.1", deadline).expect_err("connecting to a dead port must error");
    }
}
