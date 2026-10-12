//! HRD-006: `OriginPolicy::SameOrigin` compares the complete origin.
//!
//! The policy compares the `Origin` scheme, host and effective port with the
//! request's public scheme (`Request::scheme`, after trusted-proxy
//! processing), its validated host and its effective port, and refuses the
//! upgrade when the public scheme cannot be determined. Before this it
//! compared the host and port alone, so a page served over plain HTTP could
//! open a socket on the HTTPS application of the same host.
//!
//! Each test runs a loopback server that hands the framework a peer address,
//! as the production accept loop does. The process trusts `127.0.0.1` as a
//! proxy; a "direct" server reports a documentation address instead, which
//! is no trusted proxy, so its forwarded headers are not believed.

use async_trait::async_trait;
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use suprnova::http::{Request, TrustedProxiesConfig};
use suprnova::middleware::MiddlewareRegistry;
use suprnova::routing::Router;
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use suprnova::{AppConfig, Config, FrameworkError};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderName;

/// The host a proxy serves the application on.
const PUBLIC_HOST: &str = "app.example.com";

/// A peer address on no allowlist: a documentation address (RFC 5737).
const UNTRUSTED_PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9));

struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(text).await?;
        }
        Ok(())
    }
}

/// Who the server believes it is talking to.
#[derive(Clone, Copy)]
enum Peer {
    /// The real loopback peer, which the process trusts as a proxy.
    TrustedProxy,
    /// A peer the process does not trust: the browser itself, or a proxy
    /// the application has not named.
    Direct,
}

/// Trust the loopback peer, once for the process. The direct server never
/// reports the loopback address, so the one configuration serves both.
fn trust_loopback() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        Config::register(
            AppConfig::builder()
                .url("http://localhost")
                .debug(false)
                .trusted_proxies(TrustedProxiesConfig::with_ips([IpAddr::from([
                    127, 0, 0, 1,
                ])]))
                .build(),
        );
    });
}

/// Serve `/ws/echo` under the default `SameOrigin` policy and return the
/// port.
async fn spawn(peer: Peer) -> u16 {
    trust_loopback();
    let router = Arc::new(Router::new().ws_with_config(
        "/ws/echo",
        EchoHandler,
        WsConfig {
            origin_policy: OriginPolicy::SameOrigin,
            ..Default::default()
        },
    ));
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let port = listener.local_addr().expect("the listener address").port();

    tokio::spawn(async move {
        loop {
            let Ok((stream, address)) = listener.accept().await else {
                return;
            };
            let peer_ip = match peer {
                Peer::TrustedProxy => address.ip(),
                Peer::Direct => UNTRUSTED_PEER,
            };
            let io = hyper_util::rt::TokioIo::new(stream);
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            suprnova::handle_request_with_peer(
                                router,
                                middleware,
                                req,
                                Some(peer_ip),
                            )
                            .await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    port
}

/// Open `/ws/echo` with `headers` set on the handshake and return the
/// status the server answered: `101` for an upgrade, else the refusal.
async fn upgrade_status(port: u16, headers: &[(&str, &str)]) -> u16 {
    let mut request = format!("ws://127.0.0.1:{port}/ws/echo")
        .into_client_request()
        .expect("a valid WebSocket URL");
    for (name, value) in headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("a valid header name"),
            value.parse().expect("a valid header value"),
        );
    }
    let attempt = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("the handshake answers within five seconds");
    match attempt {
        Ok((mut socket, response)) => {
            let _ = socket.close(None).await;
            response.status().as_u16()
        }
        Err(WsError::Http(response)) => response.status().as_u16(),
        Err(other) => panic!("the handshake failed below HTTP: {other}"),
    }
}

/// A trusted proxy says the browser used HTTPS; an `http://` page of the
/// same host and port is another origin and must not open the socket.
#[tokio::test]
async fn hardening_an_http_origin_cannot_open_a_wss_socket() {
    let port = spawn(Peer::TrustedProxy).await;

    let status = upgrade_status(
        port,
        &[
            ("Host", PUBLIC_HOST),
            ("X-Forwarded-Proto", "https"),
            ("Origin", "http://app.example.com"),
        ],
    )
    .await;
    assert_eq!(
        status, 403,
        "an http origin opened a socket whose public scheme is https"
    );

    let status = upgrade_status(
        port,
        &[
            ("X-Forwarded-Proto", "https"),
            ("Origin", &format!("http://127.0.0.1:{port}")),
        ],
    )
    .await;
    assert_eq!(
        status, 403,
        "an http origin with the request's explicit port opened an https socket"
    );
}

/// The connection is the public hop and speaks plain HTTP; an `https://`
/// page of the same host and port is another origin.
#[tokio::test]
async fn hardening_an_https_origin_cannot_open_a_ws_socket() {
    let port = spawn(Peer::Direct).await;

    let status = upgrade_status(port, &[("Origin", &format!("https://127.0.0.1:{port}"))]).await;
    assert_eq!(
        status, 403,
        "an https origin opened a socket whose public scheme is http"
    );

    let status = upgrade_status(
        port,
        &[("Host", PUBLIC_HOST), ("Origin", "https://app.example.com")],
    )
    .await;
    assert_eq!(
        status, 403,
        "an https origin on the default port opened an http socket"
    );

    // The control: the same server admits its own http origin.
    let status = upgrade_status(port, &[("Origin", &format!("http://127.0.0.1:{port}"))]).await;
    assert_eq!(status, 101, "the request's own http origin was refused");
}

/// Behind a trusted TLS-terminating proxy the public origin is the one the
/// proxy names: `X-Forwarded-Proto` for the scheme and the `Host` it
/// passes on, or `X-Forwarded-Host`, for the host.
#[tokio::test]
async fn hardening_a_proxy_derived_https_origin_matches() {
    let port = spawn(Peer::TrustedProxy).await;

    let status = upgrade_status(
        port,
        &[
            ("Host", PUBLIC_HOST),
            ("X-Forwarded-Proto", "https"),
            ("Origin", "https://app.example.com"),
        ],
    )
    .await;
    assert_eq!(
        status, 101,
        "the https origin a trusted proxy names was refused"
    );

    let status = upgrade_status(
        port,
        &[
            ("X-Forwarded-Host", PUBLIC_HOST),
            ("X-Forwarded-Proto", "https"),
            ("Origin", "https://app.example.com"),
        ],
    )
    .await;
    assert_eq!(
        status, 101,
        "the host a trusted proxy names in X-Forwarded-Host was not used"
    );
}

/// One handshake that must be admitted: the server's port, the headers set
/// on the handshake, and what the case shows.
type PortCase = (u16, &'static [(&'static str, &'static str)], &'static str);

/// `:443` on https and `:80` on http name the default port, so the explicit
/// and the elided forms are one origin on either side; another port is not.
#[tokio::test]
async fn hardening_explicit_and_default_ports_compare_equal() {
    let proxied = spawn(Peer::TrustedProxy).await;
    let direct = spawn(Peer::Direct).await;

    let cases: [PortCase; 5] = [
        (
            proxied,
            &[
                ("Host", "app.example.com:443"),
                ("X-Forwarded-Proto", "https"),
                ("Origin", "https://app.example.com"),
            ],
            "Host :443 against an https origin with the port elided",
        ),
        (
            proxied,
            &[
                ("Host", PUBLIC_HOST),
                ("X-Forwarded-Proto", "https"),
                ("Origin", "https://app.example.com:443"),
            ],
            "Host with the port elided against an https origin :443",
        ),
        (
            proxied,
            &[
                ("Host", PUBLIC_HOST),
                ("X-Forwarded-Proto", "https"),
                ("X-Forwarded-Port", "8443"),
                ("Origin", "https://app.example.com:8443"),
            ],
            "the port a trusted proxy names in X-Forwarded-Port",
        ),
        (
            direct,
            &[
                ("Host", "app.example.com:80"),
                ("Origin", "http://app.example.com"),
            ],
            "Host :80 against an http origin with the port elided",
        ),
        (
            direct,
            &[
                ("Host", PUBLIC_HOST),
                ("Origin", "http://app.example.com:80"),
            ],
            "Host with the port elided against an http origin :80",
        ),
    ];
    for (port, headers, why) in cases {
        assert_eq!(upgrade_status(port, headers).await, 101, "{why}");
    }

    let status = upgrade_status(
        proxied,
        &[
            ("Host", "app.example.com:8443"),
            ("X-Forwarded-Proto", "https"),
            ("Origin", "https://app.example.com"),
        ],
    )
    .await;
    assert_eq!(
        status, 403,
        "an https origin on 443 matched a request on port 8443"
    );
}

/// A request that cannot say which scheme the browser used is refused,
/// whatever its host and port.
#[tokio::test]
async fn hardening_an_unknown_public_scheme_refuses() {
    let proxied = spawn(Peer::TrustedProxy).await;
    let direct = spawn(Peer::Direct).await;

    // A trusted proxy that names no scheme may have terminated TLS.
    let status = upgrade_status(
        proxied,
        &[("Host", PUBLIC_HOST), ("Origin", "http://app.example.com")],
    )
    .await;
    assert_eq!(
        status, 403,
        "a trusted proxy that names no scheme admitted an origin by host and port"
    );

    // A trusted proxy that names a scheme other than http or https.
    let status = upgrade_status(
        proxied,
        &[
            ("Host", PUBLIC_HOST),
            ("X-Forwarded-Proto", "ftp"),
            ("Origin", "http://app.example.com"),
        ],
    )
    .await;
    assert_eq!(
        status, 403,
        "a scheme that is neither http nor https admitted an origin"
    );

    // A forwarded scheme from a peer the application does not trust: a
    // proxy stands in front, and nothing it says can be believed.
    for origin in ["http://app.example.com", "https://app.example.com"] {
        let status = upgrade_status(
            direct,
            &[
                ("Host", PUBLIC_HOST),
                ("X-Forwarded-Proto", "https"),
                ("Origin", origin),
            ],
        )
        .await;
        assert_eq!(
            status, 403,
            "an untrusted peer's forwarded scheme admitted {origin}"
        );
    }
}
