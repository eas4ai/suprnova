//! `SESSION_COOKIE_PREFIX` routes session cookies by wire name while the
//! encryption layer continues binding the logical name.

use bytes::Bytes;

use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use suprnova::Middleware;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{CookiePrefix, Crypt, EncryptionKey, FrameworkError};

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

#[derive(Default)]
struct MemoryStore {
    session: Mutex<Option<SessionData>>,
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.session.lock().unwrap().clone())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        *self.session.lock().unwrap() = Some(session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        let mut session = self.session.lock().unwrap();
        if session.as_ref().is_some_and(|session| session.id == id) {
            *session = None;
        }
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

fn percent_encode_cookie_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'=' => encoded.push_str("%3D"),
            b'+' => encoded.push_str("%2B"),
            b'/' => encoded.push_str("%2F"),
            b';' => encoded.push_str("%3B"),
            b' ' => encoded.push_str("%20"),
            b',' => encoded.push_str("%2C"),
            _ => encoded.push(byte as char),
        }
    }
    encoded
}

async fn post_request(cookie: Option<(&str, &str)>) -> suprnova::Request {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use suprnova::Request;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie_header = cookie
        .map(|(name, value)| format!("Cookie: {name}={}\r\n", percent_encode_cookie_value(value)))
        .unwrap_or_default();
    let http_bytes = format!(
        "POST /api/health/live HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\n{cookie_header}Content-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (request_tx, request_rx) = oneshot::channel::<Request>();
    let request_tx = Mutex::new(Some(request_tx));
    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let wrapped = Request::new(request);
            if let Ok(mut guard) = request_tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(wrapped);
            }
            async {
                Ok::<_, Infallible>(hyper::Response::new(
                    http_body_util::Full::new(Bytes::new()),
                ))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });

    let mut client = client_io;
    client.write_all(&http_bytes).await.unwrap();
    drop(client);
    request_rx.await.expect("request captured")
}

fn set_cookie_parts<B>(response: &hyper::Response<B>) -> (String, String) {
    let header = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("session response must set a cookie");
    let (name, value) = header
        .split_once('=')
        .expect("Set-Cookie must contain a name and value");
    let value = value.split(';').next().expect("cookie value segment");
    (name.to_string(), value.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefixed_session_cookie_round_trips() {
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.cookie_prefix = CookiePrefix::Host;
    let middleware = SessionMiddleware::with_store(config, store);

    let first_next: suprnova::middleware::Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| session.put("marker", true));
            Ok(suprnova::HttpResponse::text("stored"))
        })
    });
    let first = match middleware
        .handle(post_request(None).await, first_next)
        .await
    {
        Ok(response) | Err(response) => response.into_hyper(),
    };
    let (wire_name, wire_value) = set_cookie_parts(&first);
    assert_eq!(wire_name, "__Host-suprnova_session");

    let observed = Arc::new(Mutex::new(None));
    let observed_for_handler = observed.clone();
    let second_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_for_handler.clone();
        Box::pin(async move {
            *observed.lock().unwrap() =
                suprnova::session::session().and_then(|session| session.get::<bool>("marker"));
            Ok(suprnova::HttpResponse::text("read"))
        })
    });
    match middleware
        .handle(
            post_request(Some((&wire_name, &wire_value))).await,
            second_next,
        )
        .await
    {
        Ok(_) => {}
        Err(_) => panic!("second request failed"),
    }
    assert_eq!(*observed.lock().unwrap(), Some(true));
}

#[test]
fn prefixed_forget_targets_the_wire_name() {
    let mut config = SessionConfig::default();
    config.cookie_prefix = CookiePrefix::Host;
    let header =
        suprnova::session::middleware::create_forget_remember_cookie(&config).to_header_value();
    assert!(header.starts_with("__Host-remember_me="), "{header}");
    assert!(header.contains("Max-Age=0"), "{header}");
}
