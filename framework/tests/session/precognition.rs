//! PAR-085: live validation reads sessions without saving state or navigation.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    Crypt, EncryptionKey, FrameworkError, HttpResponse, InertiaConfig, InertiaHeadersMiddleware,
    Middleware, MiddlewareRegistry, Next, Precognitive, Redirect, Request, Response, Router,
    async_trait,
};

#[derive(Default)]
struct StoredRows {
    rows: HashMap<String, SessionData>,
    revision: u64,
    destroys: u64,
}

/// A real in-memory store whose revision records every write, including touches.
#[derive(Default)]
struct Store {
    stored: Mutex<StoredRows>,
    refuse_writes: bool,
}

impl Store {
    fn seeded(session: SessionData) -> Self {
        Self {
            stored: Mutex::new(StoredRows {
                rows: HashMap::from([(session.id.clone(), session)]),
                ..StoredRows::default()
            }),
            refuse_writes: false,
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        let stored = self.stored.lock().expect("store lock");
        let rows: serde_json::Map<String, serde_json::Value> = stored
            .rows
            .iter()
            .map(|(id, session)| {
                (
                    id.clone(),
                    serde_json::json!({
                        "data": session.data,
                        "user_id": session.user_id,
                        "csrf_token": session.csrf_token,
                    }),
                )
            })
            .collect();
        serde_json::json!({
            "rows": rows,
            "updated_revision": stored.revision,
            "destroys": stored.destroys,
        })
    }
}

#[async_trait]
impl SessionStore for Store {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self
            .stored
            .lock()
            .expect("store lock")
            .rows
            .get(id)
            .cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let mut stored = self.stored.lock().expect("store lock");
        stored.revision += 1;
        if self.refuse_writes {
            return Err(FrameworkError::internal("session store is read-only"));
        }
        let mut session = session.clone();
        session.mark_clean();
        stored.rows.insert(session.id.clone(), session);
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        let mut stored = self.stored.lock().expect("store lock");
        stored.destroys += 1;
        stored.rows.remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut stored = self.stored.lock().expect("store lock");
        let before = stored.rows.len();
        stored
            .rows
            .retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - stored.rows.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Exercise session reads and writes in route middleware, which runs on marked requests.
struct SessionEffects;

#[async_trait]
impl Middleware for SessionEffects {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let path = request.path().to_owned();
        if path == "/read" {
            assert!(request.is_precognitive());
            assert_eq!(suprnova::session::auth_user_id().as_deref(), Some("7"));
            assert_eq!(
                suprnova::session::session().and_then(|session| session.get::<String>("name")),
                Some("Shawn".to_owned())
            );
        }
        if matches!(path.as_str(), "/mutate" | "/denied") {
            assert!(request.is_precognitive());
            suprnova::session::session_mut(|session| {
                let _: Option<String> = session.get_flash("toast");
                session.put("name", "Changed");
                session.flash("extra", "Not saved");
            });
            suprnova::session::regenerate_session_id();
        }
        if path == "/denied" {
            return Err(HttpResponse::text("Validation failed").status(422));
        }
        // The previous URL must also remain absent inside the active session.
        let response = next(request).await;
        if path == "/inertia" {
            assert_eq!(
                suprnova::session::session().and_then(|session| session.previous_url()),
                None
            );
        }
        response
    }
}

fn router() -> Router {
    Router::new()
        .post("/flash", |_request: Request| async {
            suprnova::session::session_mut(|session| session.flash("toast", "Saved"));
            suprnova::text("flashed")
        })
        .get("/page", |_request: Request| async {
            let toast =
                suprnova::session::session_mut(|session| session.get_flash::<String>("toast"))
                    .flatten();
            Ok(HttpResponse::json(serde_json::json!({"toast": toast})))
        })
        .post("/back", |_request: Request| async {
            Redirect::back("/fallback").into()
        })
        .get("/validate", |_request: Request| async {
            suprnova::text("real page")
        })
        .middleware(Precognitive)
        .get("/read", |_request: Request| async {
            suprnova::text("read")
        })
        .middleware(Precognitive)
        .middleware(SessionEffects)
        .get("/mutate", |_request: Request| async {
            suprnova::text("mutated")
        })
        .middleware(Precognitive)
        .middleware(SessionEffects)
        .get("/denied", |_request: Request| async {
            suprnova::text("unreachable")
        })
        .middleware(Precognitive)
        .middleware(SessionEffects)
        .get("/inertia", |_request: Request| async {
            suprnova::text("inertia page")
        })
        .middleware(Precognitive)
        .get("/unmarked", |_request: Request| async {
            suprnova::text("ordinary page")
        })
        .into()
}

fn stack(store: Arc<Store>, inertia: bool) -> MiddlewareRegistry {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let mut registry =
        MiddlewareRegistry::new().append(SessionMiddleware::with_store(config, store));
    if inertia {
        // Observe the session after the Inertia store runs, while its scope is active.
        registry = registry
            .append(SessionEffects)
            .append(InertiaHeadersMiddleware::from_config(
                &InertiaConfig::new().store_previous_url(true),
            ));
    }
    registry
}

/// A legacy cookie has no touch stamp, so every ordinary request must refresh it.
fn cookie(id: &str) -> String {
    let config = SessionConfig::default();
    let cookie = suprnova::http::cookie::Cookie::encrypted(&config.cookie_name, id)
        .expect("encrypted cookie");
    cookie
        .to_header_value()
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned()
}

async fn send(
    registry: &Arc<MiddlewareRegistry>,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> hyper::Response<Bytes> {
    let router = Arc::new(router());
    let registry = registry.clone();
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |request| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, request).await) }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .await;
    });
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client))
            .await
            .expect("client handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        sender.send_request(request.body(Full::new(Bytes::new())).expect("request")),
    )
    .await
    .expect("response timeout")
    .expect("response");
    let (parts, body) = response.into_parts();
    hyper::Response::from_parts(
        parts,
        body.collect().await.expect("response body").to_bytes(),
    )
}

fn seeded() -> (Arc<Store>, Arc<MiddlewareRegistry>, String) {
    let mut session = SessionData::new("a".repeat(40), "b".repeat(40));
    session.user_id = Some("7".to_owned());
    session.put("name", "Shawn");
    session.mark_clean();
    let store = Arc::new(Store::seeded(session));
    let registry = Arc::new(stack(store.clone(), false));
    (store, registry, cookie(&"a".repeat(40)))
}

#[tokio::test]
async fn precognition_keeps_a_toast_for_the_next_real_page() {
    let store = Arc::new(Store::default());
    let registry = Arc::new(stack(store.clone(), false));
    let flash = send(&registry, "POST", "/flash", &[]).await;
    assert_eq!(flash.status(), 200);
    let cookie = flash
        .headers()
        .get("set-cookie")
        .expect("session cookie")
        .to_str()
        .expect("cookie text")
        .split(';')
        .next()
        .expect("cookie pair");
    let before = store.snapshot();
    for path in ["/validate", "/mutate", "/denied"] {
        let response = send(
            &registry,
            "GET",
            path,
            &[("Cookie", cookie), ("Precognition", "true")],
        )
        .await;
        assert_eq!(
            response.status().as_u16(),
            if path == "/denied" { 422 } else { 204 }
        );
        assert_eq!(
            store.snapshot(),
            before,
            "{path} must not age or consume stored flash"
        );
    }
    let page = send(&registry, "GET", "/page", &[("Cookie", cookie)]).await;
    assert_eq!(page.status(), 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(page.body()).expect("page JSON")["toast"],
        "Saved"
    );
    let next_page = send(&registry, "GET", "/page", &[("Cookie", cookie)]).await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(next_page.body()).expect("page JSON")["toast"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn precognition_reads_identity_without_changing_payload_or_touch_revision() {
    let (store, registry, cookie) = seeded();
    let before = store.snapshot();
    let response = send(
        &registry,
        "GET",
        "/read",
        &[("Cookie", &cookie), ("Precognition", "true")],
    )
    .await;
    assert_eq!(response.status(), 204);
    assert_eq!(store.snapshot(), before);
    assert!(response.headers().get("set-cookie").is_none());
}

#[tokio::test]
async fn precognition_does_not_persist_mutations_or_rotate_the_stored_id() {
    let (store, registry, cookie) = seeded();
    let before = store.snapshot();
    for path in ["/mutate", "/denied"] {
        let response = send(
            &registry,
            "GET",
            path,
            &[("Cookie", &cookie), ("Precognition", "true")],
        )
        .await;
        assert_eq!(
            response.status().as_u16(),
            if path == "/denied" { 422 } else { 204 }
        );
        assert_eq!(store.snapshot(), before);
        assert!(response.headers().get("set-cookie").is_none());
    }
}

#[tokio::test]
async fn precognition_get_does_not_become_the_previous_url() {
    let (store, registry, cookie) = seeded();
    let response = send(
        &registry,
        "GET",
        "/validate?email=",
        &[("Cookie", &cookie), ("Precognition", "true")],
    )
    .await;
    assert_eq!(response.status(), 204);
    let back = send(&registry, "POST", "/back", &[("Cookie", &cookie)]).await;
    assert_eq!(back.status(), 302);
    assert_eq!(
        back.headers().get("Location").expect("redirect target"),
        "/fallback"
    );
    assert_eq!(
        store.snapshot()["rows"]["a".repeat(40)]["data"]["_previous.url"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn precognition_inertia_get_does_not_become_the_previous_url() {
    let (store, _, cookie) = seeded();
    let registry = Arc::new(stack(store.clone(), true));
    let response = send(
        &registry,
        "GET",
        "/inertia",
        &[
            ("Cookie", &cookie),
            ("Precognition", "true"),
            ("X-Inertia", "true"),
        ],
    )
    .await;
    assert_eq!(response.status(), 204);
    let back = send(&registry, "POST", "/back", &[("Cookie", &cookie)]).await;
    assert_eq!(
        back.headers().get("Location").expect("redirect target"),
        "/fallback"
    );
}

#[tokio::test]
async fn precognition_header_alone_does_not_suppress_a_real_navigation() {
    let (store, registry, cookie) = seeded();
    let response = send(
        &registry,
        "GET",
        "/unmarked",
        &[("Cookie", &cookie), ("Precognition", "true")],
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        store.snapshot()["rows"]["a".repeat(40)]["data"]["_previous.url"],
        "/unmarked"
    );
    let back = send(&registry, "POST", "/back", &[("Cookie", &cookie)]).await;
    assert_eq!(
        back.headers().get("Location").expect("redirect target"),
        "/unmarked"
    );
}

#[tokio::test]
async fn precognition_changes_preserve_prefetch_previous_url_exclusions() {
    for header in ["Purpose", "Sec-Purpose", "X-Moz"] {
        let (store, registry, cookie) = seeded();
        let response = send(
            &registry,
            "GET",
            "/validate",
            &[("Cookie", &cookie), (header, "PrEfEtCh")],
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            store.snapshot()["rows"]["a".repeat(40)]["data"]["_previous.url"],
            serde_json::Value::Null
        );
        let back = send(&registry, "POST", "/back", &[("Cookie", &cookie)]).await;
        assert_eq!(
            back.headers().get("Location").expect("redirect target"),
            "/fallback"
        );
    }
}

#[tokio::test]
async fn precognition_cookieless_mutation_never_attempts_a_failing_store_write() {
    let store = Arc::new(Store {
        refuse_writes: true,
        ..Store::default()
    });
    let registry = Arc::new(stack(store.clone(), false));
    let before = store.snapshot();
    let response = send(&registry, "GET", "/mutate", &[("Precognition", "true")]).await;
    assert_eq!(response.status(), 204);
    assert_eq!(store.snapshot(), before);
    assert!(response.headers().get("set-cookie").is_none());
}
