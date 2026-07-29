//! `suprnova ssr:check` — verify the Inertia SSR worker is reachable.
//!
//! TCP-level reachability ping. Either the worker is listening on the
//! configured URL's host:port (exit 0), or it isn't (exit 1). The
//! check is deliberately protocol-agnostic — POSTing a fake page to
//! `/render` would surface false negatives when a real page renderer
//! errors on the dummy input. We just verify the worker is up.
//!
//! Use this in CI or your deploy-pipeline smoke tests:
//!
//! ```bash
//! suprnova ssr:start &
//! ./wait-until.sh suprnova ssr:check
//! # ...run e2e tests...
//! ```

use std::net::TcpStream;
use std::time::Duration;

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

    match probe(&host, port, Duration::from_millis(timeout_ms)) {
        Ok(_) => {
            println!("OK: SSR worker reachable at {}", url);
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("FAIL: SSR worker not reachable at {url} ({e})");
            std::process::exit(1);
        }
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
    use std::net::{SocketAddr, TcpListener};
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
    fn refusing() -> SocketAddr {
        let (listener, addr) = listening();
        drop(listener);
        addr
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
        // Bind then drop, so the port is almost certainly closed.
        let (listener, addr) = listening();
        drop(listener);

        let started = Instant::now();
        let result = probe("127.0.0.1", addr.port(), Duration::from_millis(800));

        assert!(result.is_err(), "a closed port must not probe OK");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "must fail within the budget, not hang"
        );
    }
}
