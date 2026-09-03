//! Full-middleware regressions for one-shot Inertia session delivery.

#![cfg(feature = "testing")]

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::json;
use suprnova::middleware::{Middleware, MiddlewareRegistry, Next};
use suprnova::serde::ser::Error as _;
use suprnova::serde::{Deserialize, Deserializer, Serialize, Serializer};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    FrameworkError, HttpResponse, Inertia, InertiaConfig, InertiaResponse, Request, Response,
    Router, handle_request,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Default, Clone)]
struct BoomSerialize;

impl Serialize for BoomSerialize {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom("boom: serialize always fails"))
    }
}

impl<'de> Deserialize<'de> for BoomSerialize {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Ok(Self)
    }
}

#[derive(suprnova::Data, validator::Validate)]
struct BoomDto {
    ok: i32,
    bad: BoomSerialize,
}

#[derive(Default)]
struct MemoryStore {
    sessions: Mutex<HashMap<String, SessionData>>,
    fail_next_write: AtomicBool,
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        if self.fail_next_write.swap(false, Ordering::SeqCst) {
            return Err(FrameworkError::internal("injected session write failure"));
        }
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    });
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

async fn request(cookie: Option<&str>, inertia: bool, error_bag: Option<&str>) -> Request {
    let cookie_header = cookie
        .map(|value| {
            format!(
                "Cookie: suprnova_session={}\r\n",
                percent_encode_cookie_value(value)
            )
        })
        .unwrap_or_default();
    let inertia_header = if inertia { "X-Inertia: true\r\n" } else { "" };
    let error_bag_header = error_bag
        .map(|bag| format!("X-Inertia-Error-Bag: {bag}\r\n"))
        .unwrap_or_default();
    let http_bytes = format!(
        "GET /form HTTP/1.1\r\nHost: localhost\r\n{cookie_header}{inertia_header}{error_bag_header}Content-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (request_tx, request_rx) = oneshot::channel::<Request>();
    let request_tx = Mutex::new(Some(request_tx));
    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            if let Some(tx) = request_tx.lock().unwrap().take() {
                let _ = tx.send(Request::new(request));
            }
            async { Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::new()))) }
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

fn config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

type HyperResponse = hyper::Response<http_body_util::combinators::BoxBody<Bytes, Infallible>>;

fn into_hyper(response: Response) -> HyperResponse {
    match response {
        Ok(response) | Err(response) => response.into_hyper(),
    }
}

fn session_cookie<B>(response: &hyper::Response<B>) -> Option<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|header| {
            header
                .strip_prefix("suprnova_session=")
                .and_then(|rest| rest.split(';').next())
                .map(ToOwned::to_owned)
        })
}

async fn spawn_panic_server(store: Arc<MemoryStore>, accepts: usize) -> SocketAddr {
    let router: Router = Router::new()
        .get("/seed-builder", |_request: Request| async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.panic_builder",
                    json!({"field": ["Builder panic must retry."]}),
                );
                session.flash("notice", json!({"message": "builder panic"}));
                session.flash("_inertia.preserve_fragment", true);
                session.flash("_inertia.clear_history", true);
            });
            Ok(HttpResponse::text("seeded"))
        })
        .get("/panic-builder", |_request: Request| async {
            let _response = InertiaResponse::new("Form").with("bad", BoomSerialize);
            Ok(HttpResponse::text("unreachable"))
        })
        .get("/seed-data", |_request: Request| async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.panic_data",
                    json!({"field": ["Data panic must retry."]}),
                );
                session.flash("notice", json!({"message": "data panic"}));
                session.flash("_inertia.preserve_fragment", true);
                session.flash("_inertia.clear_history", true);
            });
            Ok(HttpResponse::text("seeded"))
        })
        .get("/panic-data", |_request: Request| async {
            let _response = Inertia::data(
                "Form",
                BoomDto {
                    ok: 1,
                    bad: BoomSerialize,
                },
            );
            Ok(HttpResponse::text("unreachable"))
        })
        .get("/render", |request: Request| async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let router = Arc::new(router);
    let middleware =
        Arc::new(MiddlewareRegistry::new().append(SessionMiddleware::with_store(config(), store)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, request).await) }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    address
}

async fn send_get(
    address: SocketAddr,
    path: &str,
    cookie: Option<&str>,
    error_bag: Option<&str>,
) -> (u16, Option<String>, String) {
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    if let Some(cookie) = cookie {
        builder = builder.header(
            "Cookie",
            format!("suprnova_session={}", percent_encode_cookie_value(cookie)),
        );
    }
    if let Some(error_bag) = error_bag {
        builder = builder
            .header("X-Inertia", "true")
            .header("X-Inertia-Error-Bag", error_bag);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        sender.send_request(builder.body(Full::new(Bytes::new())).unwrap()),
    )
    .await
    .expect("request timeout")
    .expect("send request");
    let status = response.status().as_u16();
    let cookie = session_cookie(&response);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, cookie, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn body(response: HyperResponse) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn lazy_failure_reflashes_aged_named_errors_until_successful_response() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store.clone());

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.login",
                    json!({"email": ["The email field is required."]}),
                );
                session.flash("toast", json!({"message": "try again"}));
                session.flash("_old_input", json!({"email": "person@example.test"}));
                session.flash("_inertia.preserve_fragment", true);
                session.flash("_inertia.clear_history", true);
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let failing: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .lazy("boom", || async {
                    Err::<serde_json::Value, _>(FrameworkError::internal("lazy failed"))
                })
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let failed = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("login")).await, failing)
            .await,
    );
    assert_eq!(failed.status(), 500);
    let cookie = session_cookie(&failed).unwrap_or(cookie);
    let failed_body = body(failed).await;
    assert!(!failed_body.contains("The email field is required."));
    {
        let sessions = store.sessions.lock().unwrap();
        let persisted = sessions.values().next().expect("session remains persisted");
        assert_eq!(
            persisted.get::<serde_json::Value>("_flash.old._old_input"),
            Some(json!({"email": "person@example.test"})),
            "the Inertia retry guard must not reflash unrelated session data"
        );
    }

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let retried = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("login")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(retried.status(), 200);
    let cookie = session_cookie(&retried).unwrap_or(cookie);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["login"]["email"],
        "The email field is required."
    );
    assert_eq!(retry_page["flash"]["toast"]["message"], "try again");
    assert_eq!(retry_page["preserveFragment"], true);
    assert_eq!(retry_page["clearHistory"], true);

    let final_response = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("login")).await, render)
            .await,
    );
    assert_eq!(final_response.status(), 200);
    let final_page: serde_json::Value = serde_json::from_str(&body(final_response).await).unwrap();
    assert_eq!(final_page["props"]["errors"], json!({"login": {}}));
    assert!(final_page.get("flash").is_none());
    assert!(final_page.get("preserveFragment").is_none());
    assert!(final_page.get("clearHistory").is_none());

    let persisted = store
        .sessions
        .lock()
        .unwrap()
        .values()
        .next()
        .cloned()
        .expect("session remains persisted");
    assert!(!persisted.has("_flash.old._old_input"));
    assert!(!persisted.has("_flash.new._old_input"));
}

#[tokio::test]
async fn ssr_failure_reflashes_aged_errors_until_successful_response() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store);

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.profile",
                    json!({"name": ["The name field is required."]}),
                );
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let failing: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .with_config(
                    InertiaConfig::new()
                        .ssr("http://127.0.0.1:1")
                        .ssr_throw_on_error(true),
                )
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let failed = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), false, Some("profile")).await,
                failing,
            )
            .await,
    );
    assert_eq!(failed.status(), 500);
    let cookie = session_cookie(&failed).unwrap_or(cookie);

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let retried = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("profile")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(retried.status(), 200);
    let cookie = session_cookie(&retried).unwrap_or(cookie);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["profile"]["name"],
        "The name field is required."
    );

    let final_response = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("profile")).await, render)
            .await,
    );
    assert_eq!(final_response.status(), 200);
    let final_page: serde_json::Value = serde_json::from_str(&body(final_response).await).unwrap();
    assert_eq!(final_page["props"]["errors"], json!({"profile": {}}));
}

#[tokio::test]
async fn cancelled_resolution_reflashes_staged_errors() {
    let _guard = TEST_LOCK.lock().await;
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock().unwrap().as_mut().unwrap().put(
        "_flash.old.errors.cancelled",
        json!({"email": ["Try again."]}),
    );
    let started = Arc::new(tokio::sync::Notify::new());
    let started_for_resolver = started.clone();
    let request = request(None, true, Some("cancelled")).await;

    {
        let resolving = suprnova::session::session_scope_for_test(slot.clone(), async move {
            InertiaResponse::new("Form")
                .lazy("blocked", move || {
                    let started = started_for_resolver.clone();
                    async move {
                        started.notify_one();
                        std::future::pending::<Result<serde_json::Value, FrameworkError>>().await
                    }
                })
                .resolve(&request)
                .await
        });
        tokio::pin!(resolving);
        tokio::select! {
            _ = &mut resolving => panic!("resolver unexpectedly completed"),
            () = started.notified() => {}
        }
    }

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(!session.has("_flash.old.errors.cancelled"));
    assert_eq!(
        session.get::<serde_json::Value>("_flash.new.errors.cancelled"),
        Some(json!({"email": ["Try again."]}))
    );
}

#[tokio::test]
async fn session_cookie_failure_keeps_staged_errors_in_the_persisted_session() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store);

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.cookie",
                    json!({"email": ["The cookie could not be built."]}),
                );
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    suprnova::session::middleware::fail_next_session_cookie_construction_for_test();
    let failed = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("cookie")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(failed.status(), 500);
    assert!(
        session_cookie(&failed).is_none(),
        "failed construction must not attach a replacement session cookie"
    );

    let retried = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("cookie")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(retried.status(), 200);
    let cookie = session_cookie(&retried).unwrap_or(cookie);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["cookie"]["email"],
        "The cookie could not be built."
    );

    let final_response = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("cookie")).await, render)
            .await,
    );
    assert_eq!(final_response.status(), 200);
    let final_page: serde_json::Value = serde_json::from_str(&body(final_response).await).unwrap();
    assert_eq!(final_page["props"]["errors"], json!({"cookie": {}}));
}

#[tokio::test]
async fn session_write_failure_keeps_staged_errors_in_the_persisted_session() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store.clone());

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.write",
                    json!({"email": ["The session could not be written."]}),
                );
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    store.fail_next_write.store(true, Ordering::SeqCst);
    let failed = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("write")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(failed.status(), 500);
    assert!(session_cookie(&failed).is_none());

    let retried = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("write")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(retried.status(), 200);
    let cookie = session_cookie(&retried).unwrap_or(cookie);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["write"]["email"],
        "The session could not be written."
    );

    let final_response = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("write")).await, render)
            .await,
    );
    assert_eq!(final_response.status(), 200);
    let final_page: serde_json::Value = serde_json::from_str(&body(final_response).await).unwrap();
    assert_eq!(final_page["props"]["errors"], json!({"write": {}}));
}

#[tokio::test]
async fn try_data_failure_reflashes_aged_session_values_before_resolve() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store);

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.eager",
                    json!({"email": ["Eager serialization failed."]}),
                );
                session.flash("notice", json!({"message": "retry eager response"}));
                session.flash("_inertia.preserve_fragment", true);
                session.flash("_inertia.clear_history", true);
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let failing: Next = Arc::new(|_request| {
        Box::pin(async {
            match Inertia::try_data(
                "Form",
                BoomDto {
                    ok: 1,
                    bad: BoomSerialize,
                },
            ) {
                Ok(_) => panic!("serialization unexpectedly succeeded"),
                Err(error) => Err(HttpResponse::from(error)),
            }
        })
    });
    let failed = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("eager")).await, failing)
            .await,
    );
    assert_eq!(failed.status(), 500);
    let cookie = session_cookie(&failed).unwrap_or(cookie);

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let retried = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("eager")).await,
                render.clone(),
            )
            .await,
    );
    assert_eq!(retried.status(), 200);
    let cookie = session_cookie(&retried).unwrap_or(cookie);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["eager"]["email"],
        "Eager serialization failed."
    );
    assert_eq!(
        retry_page["flash"]["notice"]["message"],
        "retry eager response"
    );
    assert_eq!(retry_page["preserveFragment"], true);
    assert_eq!(retry_page["clearHistory"], true);

    let final_response = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("eager")).await, render)
            .await,
    );
    assert_eq!(final_response.status(), 200);
    let final_page: serde_json::Value = serde_json::from_str(&body(final_response).await).unwrap();
    assert_eq!(final_page["props"]["errors"], json!({"eager": {}}));
    assert!(final_page.get("flash").is_none());
    assert!(final_page.get("preserveFragment").is_none());
    assert!(final_page.get("clearHistory").is_none());
}

#[tokio::test]
async fn try_with_failure_reflashes_aged_errors_before_resolve() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let middleware = SessionMiddleware::with_store(config(), store);

    let seed: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| {
                session.flash(
                    "errors.try_with",
                    json!({"field": ["Try-with serialization failed."]}),
                );
            });
            Ok(HttpResponse::text("seeded"))
        })
    });
    let seeded = into_hyper(
        middleware
            .handle(request(None, false, None).await, seed)
            .await,
    );
    let cookie = session_cookie(&seeded).expect("seed response must attach a session cookie");

    let failing: Next = Arc::new(|_request| {
        Box::pin(async {
            match InertiaResponse::new("Form").try_with("bad", BoomSerialize) {
                Ok(_) => panic!("serialization unexpectedly succeeded"),
                Err(error) => Err(HttpResponse::from(error)),
            }
        })
    });
    let failed = into_hyper(
        middleware
            .handle(
                request(Some(&cookie), true, Some("try_with")).await,
                failing,
            )
            .await,
    );
    assert_eq!(failed.status(), 500);
    let cookie = session_cookie(&failed).unwrap_or(cookie);

    let render: Next = Arc::new(|request| {
        Box::pin(async move {
            InertiaResponse::new("Form")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
    });
    let retried = into_hyper(
        middleware
            .handle(request(Some(&cookie), true, Some("try_with")).await, render)
            .await,
    );
    assert_eq!(retried.status(), 200);
    let retry_page: serde_json::Value = serde_json::from_str(&body(retried).await).unwrap();
    assert_eq!(
        retry_page["props"]["errors"]["try_with"]["field"],
        "Try-with serialization failed."
    );
}

#[tokio::test]
async fn failed_resolution_does_not_resurrect_handler_deleted_old_value() {
    let _guard = TEST_LOCK.lock().await;
    let slot = suprnova::session::new_session_slot_for_test();
    {
        let mut guard = slot.lock().unwrap();
        let session = guard.as_mut().unwrap();
        session.put(
            "_flash.old.errors.deleted",
            json!({"field": ["staged deleted value"]}),
        );
    }
    let request = request(None, true, None).await;

    let result = suprnova::session::session_scope_for_test(slot.clone(), async move {
        InertiaResponse::new("Form")
            .lazy("mutate", || async {
                suprnova::session::session_mut(|session| {
                    session.forget("_flash.old.errors.deleted");
                });
                Err::<serde_json::Value, _>(FrameworkError::internal("resolver failed"))
            })
            .resolve(&request)
            .await
    })
    .await;
    assert!(result.is_err());

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(!session.has("_flash.old.errors.deleted"));
    assert!(!session.has("_flash.new.errors.deleted"));
}

#[tokio::test]
async fn failed_resolution_does_not_move_handler_replaced_old_value() {
    let _guard = TEST_LOCK.lock().await;
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock().unwrap().as_mut().unwrap().put(
        "_flash.old.errors.replaced",
        json!({"field": ["staged replaced value"]}),
    );
    let request = request(None, true, None).await;

    let result = suprnova::session::session_scope_for_test(slot.clone(), async move {
        InertiaResponse::new("Form")
            .lazy("mutate", || async {
                suprnova::session::session_mut(|session| {
                    session.put(
                        "_flash.old.errors.replaced",
                        json!({"field": ["handler replacement"]}),
                    );
                });
                Err::<serde_json::Value, _>(FrameworkError::internal("resolver failed"))
            })
            .resolve(&request)
            .await
    })
    .await;
    assert!(result.is_err());

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert_eq!(
        session.get::<serde_json::Value>("_flash.old.errors.replaced"),
        Some(json!({"field": ["handler replacement"]}))
    );
    assert!(!session.has("_flash.new.errors.replaced"));
}

#[tokio::test]
async fn failed_resolution_does_not_undo_handler_flush() {
    let _guard = TEST_LOCK.lock().await;
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock().unwrap().as_mut().unwrap().put(
        "_flash.old.errors.flushed",
        json!({"field": ["must stay flushed"]}),
    );
    let request = request(None, true, None).await;

    let result = suprnova::session::session_scope_for_test(slot.clone(), async move {
        InertiaResponse::new("Form")
            .lazy("flush", || async {
                suprnova::session::session_mut(|session| session.flush());
                Err::<serde_json::Value, _>(FrameworkError::internal("resolver failed"))
            })
            .resolve(&request)
            .await
    })
    .await;
    assert!(result.is_err());

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(session.data.is_empty(), "flush must remain authoritative");
}

#[tokio::test]
async fn successful_eager_builder_leaves_staged_values_for_resolve_to_commit() {
    let _guard = TEST_LOCK.lock().await;
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock().unwrap().as_mut().unwrap().put(
        "_flash.old.errors.success",
        json!({"field": ["deliver once"]}),
    );
    let request = request(None, true, Some("success")).await;

    let response = suprnova::session::session_scope_for_test(slot.clone(), async move {
        let response = InertiaResponse::new("Form")
            .try_with("ok", 1)
            .expect("clean eager value must serialize");
        assert!(
            suprnova::session::session()
                .unwrap()
                .has("_flash.old.errors.success"),
            "builder creation must not consume or reflash staged session values"
        );
        response.resolve(&request).await
    })
    .await
    .expect("response resolves");
    let page: serde_json::Value = serde_json::from_str(&body(response.into_hyper()).await).unwrap();
    assert_eq!(page["props"]["errors"]["success"]["field"], "deliver once");

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(!session.has("_flash.old.errors.success"));
    assert!(!session.has("_flash.new.errors.success"));
}

#[tokio::test]
async fn infallible_builder_panic_recovery_preserves_persisted_session_values() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let address = spawn_panic_server(Arc::new(MemoryStore::default()), 4).await;

    let (status, cookie, _) = send_get(address, "/seed-builder", None, None).await;
    assert_eq!(status, 200);
    let cookie = cookie.expect("seed response must attach a session cookie");

    let (status, replacement, _) = send_get(address, "/panic-builder", Some(&cookie), None).await;
    assert_eq!(status, 500, "request-level recovery must catch the panic");
    let cookie = replacement.unwrap_or(cookie);

    let (status, replacement, body) =
        send_get(address, "/render", Some(&cookie), Some("panic_builder")).await;
    assert_eq!(status, 200);
    let cookie = replacement.unwrap_or(cookie);
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["props"]["errors"]["panic_builder"]["field"],
        "Builder panic must retry."
    );
    assert_eq!(page["flash"]["notice"]["message"], "builder panic");
    assert_eq!(page["preserveFragment"], true);
    assert_eq!(page["clearHistory"], true);

    let (status, _, body) =
        send_get(address, "/render", Some(&cookie), Some("panic_builder")).await;
    assert_eq!(status, 200);
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["errors"], json!({"panic_builder": {}}));
    assert!(page.get("flash").is_none());
    assert!(page.get("preserveFragment").is_none());
    assert!(page.get("clearHistory").is_none());
}

#[tokio::test]
async fn infallible_data_panic_recovery_preserves_persisted_session_values() {
    let _guard = TEST_LOCK.lock().await;
    ensure_crypt();
    let address = spawn_panic_server(Arc::new(MemoryStore::default()), 4).await;

    let (status, cookie, _) = send_get(address, "/seed-data", None, None).await;
    assert_eq!(status, 200);
    let cookie = cookie.expect("seed response must attach a session cookie");

    let (status, replacement, _) = send_get(address, "/panic-data", Some(&cookie), None).await;
    assert_eq!(status, 500, "request-level recovery must catch the panic");
    let cookie = replacement.unwrap_or(cookie);

    let (status, replacement, body) =
        send_get(address, "/render", Some(&cookie), Some("panic_data")).await;
    assert_eq!(status, 200);
    let cookie = replacement.unwrap_or(cookie);
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        page["props"]["errors"]["panic_data"]["field"],
        "Data panic must retry."
    );
    assert_eq!(page["flash"]["notice"]["message"], "data panic");
    assert_eq!(page["preserveFragment"], true);
    assert_eq!(page["clearHistory"], true);

    let (status, _, body) = send_get(address, "/render", Some(&cookie), Some("panic_data")).await;
    assert_eq!(status, 200);
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["props"]["errors"], json!({"panic_data": {}}));
    assert!(page.get("flash").is_none());
    assert!(page.get("preserveFragment").is_none());
    assert!(page.get("clearHistory").is_none());
}
