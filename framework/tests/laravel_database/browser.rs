//! A browser for the tests: a router served through `handle_request` on a
//! loopback socket, behind the framework's `SessionMiddleware`, and a
//! cookie jar that keeps what the responses set.
//!
//! The server runs on the test's own runtime and thread, so it sees the
//! container [`crate::support::bind`] installed for the test.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::session::SessionConfig;
use suprnova::{MiddlewareRegistry, Router, SessionMiddleware, handle_request};

/// The session cookie's name in [`SessionConfig::default`].
pub const SESSION_COOKIE: &str = "suprnova_session";

pub struct Browser {
    addr: SocketAddr,
    cookies: HashMap<String, String>,
}

impl Browser {
    /// Serve `router` behind the session middleware, with remember-me
    /// cookies that live a day.
    pub async fn serve(router: Router) -> Self {
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime = Duration::from_secs(60 * 60 * 24);
        let registry = Arc::new(MiddlewareRegistry::new().append(SessionMiddleware::new(config)));
        let router = Arc::new(router);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("the listener's address");
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let router = router.clone();
                let registry = registry.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: hyper::Request<Incoming>| {
                        let router = router.clone();
                        let registry = registry.clone();
                        async move {
                            Ok::<_, Infallible>(handle_request(router, registry, request).await)
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            addr,
            cookies: HashMap::new(),
        }
    }

    /// The jar's value for `name`.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.get(name).cloned()
    }

    /// Put `value` in the jar under `name`.
    pub fn set_cookie(&mut self, name: &str, value: &str) {
        self.cookies.insert(name.to_owned(), value.to_owned());
    }

    /// Drop `name` from the jar, as a browser drops an expired cookie.
    pub fn forget(&mut self, name: &str) {
        self.cookies.remove(name);
    }

    /// Send a GET with the jar's cookies and `headers`; keep what the
    /// response sets and return `(status, body)`.
    pub async fn get(&mut self, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
        let stream = tokio::net::TcpStream::connect(self.addr)
            .await
            .expect("connect to the test server");
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut builder = hyper::Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", "localhost");
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header("Cookie", jar);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            sender.send_request(builder.body(Full::new(Bytes::new())).expect("a request")),
        )
        .await
        .expect("a response in time")
        .expect("a response");
        let (parts, body) = response.into_parts();
        for set_cookie in parts.headers.get_all("set-cookie") {
            let set_cookie = set_cookie.to_str().expect("an ASCII set-cookie");
            let pair = set_cookie.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let expired = set_cookie.to_ascii_lowercase().contains("max-age=0");
            if expired || value.is_empty() {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
        let body = body.collect().await.expect("the body").to_bytes();
        (
            parts.status.as_u16(),
            String::from_utf8_lossy(&body).into_owned(),
        )
    }
}

/// The value of the request header `name`, or `""`.
pub fn header(request: &suprnova::Request, name: &str) -> String {
    request.header(name).unwrap_or_default().to_owned()
}
