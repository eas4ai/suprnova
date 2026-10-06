//! A loopback server with a trusted peer, a raw client, and the
//! process-wide configuration the tests install.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::http::TrustedProxiesConfig;
use suprnova::{
    AppConfig, Config, Crypt, EncryptionKey, FrameworkError, MiddlewareRegistry, Router,
    SessionData, SessionStore, async_trait, handle_request_with_peer,
};

/// The prefix a trusted proxy names in these tests.
pub const PREFIX: &str = "/billing";

/// Register an `AppConfig` with `app_url` that trusts the loopback peer
/// every test connects from.
pub fn install(app_url: &str) {
    install_with(app_url, true);
}

/// Register an `AppConfig` with `app_url` that trusts no proxy.
pub fn install_untrusted(app_url: &str) {
    install_with(app_url, false);
}

fn install_with(app_url: &str, trust_loopback: bool) {
    let proxies = if trust_loopback {
        TrustedProxiesConfig::with_ips([IpAddr::from([127, 0, 0, 1])])
    } else {
        TrustedProxiesConfig::empty()
    };
    Config::register(
        AppConfig::builder()
            .url(app_url)
            .debug(false)
            .trusted_proxies(proxies)
            .build(),
    );
}

/// Install the process-wide encryption key once.
pub fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

/// Serve `router` with `middleware` on a loopback socket, every connection
/// handed to `handle_request_with_peer` with the peer's address, as the
/// production accept loop does.
pub async fn serve(router: Router, middleware: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(middleware);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the test listener");
    let address = listener.local_addr().expect("the listener address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                return;
            };
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            tokio::spawn(async move {
                let peer_ip = Some(peer.ip());
                let service = service_fn(move |request| {
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            handle_request_with_peer(router, middleware, request, peer_ip).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    address
}

/// One response: status, headers, body.
pub struct Reply {
    pub status: u16,
    pub headers: hyper::HeaderMap,
    pub body: String,
}

impl Reply {
    /// The only value of header `name`, or `None`.
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|value| value.to_str().expect("an ASCII header").to_owned())
    }

    /// Every `Set-Cookie` value whose cookie is named `name`.
    pub fn set_cookies(&self, name: &str) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter(|value| value.starts_with(&format!("{name}=")))
            .map(str::to_owned)
            .collect()
    }

    /// The `Path` attribute of the one `Set-Cookie` named `name`.
    pub fn cookie_path(&self, name: &str) -> String {
        let cookies = self.set_cookies(name);
        assert_eq!(cookies.len(), 1, "one `{name}` cookie: {cookies:?}");
        cookies[0]
            .split(';')
            .map(str::trim)
            .find_map(|attribute| attribute.strip_prefix("Path="))
            .unwrap_or_else(|| panic!("no Path on {}", cookies[0]))
            .to_owned()
    }

    /// The `name=value` pair of the one `Set-Cookie` named `name`.
    pub fn cookie_pair(&self, name: &str) -> String {
        let cookies = self.set_cookies(name);
        assert_eq!(cookies.len(), 1, "one `{name}` cookie: {cookies:?}");
        cookies[0].split(';').next().expect("a pair").to_owned()
    }

    /// The body parsed as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("JSON body ({error}): {}", self.body))
    }
}

/// Send one request. Header values are bytes, so a test can send one no
/// `&str` can hold, and a name may repeat to send it on several lines.
pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &[u8])],
    body: &[u8],
) -> Reply {
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect to the test server");
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("the HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .body(Full::new(Bytes::copy_from_slice(body)))
        .expect("a request");
    let map = request.headers_mut();
    map.insert("host", HeaderValue::from_static("app.test"));
    for (name, value) in headers {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
            HeaderValue::from_bytes(value).expect("a header value"),
        );
    }
    let response = tokio::time::timeout(Duration::from_secs(10), sender.send_request(request))
        .await
        .expect("the response within ten seconds")
        .expect("a response");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// `GET path` with `headers`.
pub async fn get(address: SocketAddr, path: &str, headers: &[(&str, &str)]) -> Reply {
    let headers: Vec<(&str, &[u8])> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_bytes()))
        .collect();
    send(address, "GET", path, &headers, b"").await
}

/// `GET path` behind a trusted proxy that serves the application under
/// [`PREFIX`].
pub async fn get_prefixed(address: SocketAddr, path: &str) -> Reply {
    get(address, path, &[("x-forwarded-prefix", PREFIX)]).await
}

/// `GET path` behind [`PREFIX`], sending `cookie` as the `Cookie` header.
pub async fn get_prefixed_with_cookie(address: SocketAddr, path: &str, cookie: &str) -> Reply {
    get(
        address,
        path,
        &[("x-forwarded-prefix", PREFIX), ("cookie", cookie)],
    )
    .await
}

/// A session store in memory, shared by every request of a test.
#[derive(Default)]
pub struct MemorySessionStore(pub Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().expect("the store lock").get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .expect("the store lock")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().expect("the store lock").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().expect("the store lock");
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}
