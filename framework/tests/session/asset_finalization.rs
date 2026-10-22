use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{FrameworkError, HttpResponse, Request};

#[derive(Default)]
struct Store {
    sessions: Mutex<HashMap<String, SessionData>>,
    writes: AtomicUsize,
}
#[async_trait]
impl SessionStore for Store {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let mut session = session.clone();
        session.mark_clean();
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session);
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, _: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}
fn config() -> SessionConfig {
    suprnova::testing::install_test_encryption_key();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}
async fn serve(
    middleware: &SessionMiddleware,
    config: &SessionConfig,
    cookie: Option<&str>,
    next: Next,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>> {
    let headers = cookie
        .map(|value| {
            vec![(
                "cookie".to_owned(),
                format!("{}={value}", config.cookie_name),
            )]
        })
        .unwrap_or_default();
    let response = middleware
        .handle(
            Request::for_test_with_headers(
                "GET",
                "/",
                headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
            next,
        )
        .await;
    match response {
        Ok(response) | Err(response) => response.into_hyper(),
    }
}
fn static_next(root: &std::path::Path, issue_token: bool) -> Next {
    let files = suprnova::StaticFiles::from_dir(root).handler();
    Arc::new(move |_| {
        let files = files.clone();
        Box::pin(async move {
            let token = issue_token.then(|| suprnova::session::get_csrf_token().unwrap());
            let response = files(Request::for_test_with_headers(
                "GET",
                "/asset.js",
                Vec::<(&str, &str)>::new(),
            ))
            .await?;
            Ok(if let Some(token) = token {
                response.header("X-Session-Token", token)
            } else {
                response
            })
        })
    })
}
#[tokio::test]
async fn budget_002_static_token_issuance_is_private() {
    let config = config();
    let store = Arc::new(Store::default());
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("asset.js"), "export default 1").unwrap();
    let response = serve(&middleware, &config, None, static_next(dir.path(), true)).await;
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "private, no-store"
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn budget_002_static_flash_aging_does_not_consume_redirect_flash() {
    let config = config();
    let store = Arc::new(Store::default());
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
    let redirect: Next = Arc::new(|_| {
        Box::pin(async {
            suprnova::session::session_mut(|session| session.flash("notice", "saved"));
            Ok(HttpResponse::text("")
                .status(302)
                .header("Location", "/done"))
        })
    });
    let response = serve(&middleware, &config, None, redirect).await;
    let prefix = format!("{}=", config.cookie_name);
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            value
                .to_str()
                .ok()?
                .strip_prefix(&prefix)?
                .split(';')
                .next()
        })
        .unwrap()
        .to_owned();
    let before = store
        .sessions
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .data
        .clone();
    let writes = store.writes.load(Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("asset.js"), "export default 1").unwrap();
    let asset = serve(
        &middleware,
        &config,
        Some(&cookie),
        static_next(dir.path(), false),
    )
    .await;
    assert_eq!(
        store.writes.load(Ordering::SeqCst),
        writes,
        "asset must not persist automatic flash aging"
    );
    assert!(
        asset
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none()
    );
    assert_eq!(
        store.sessions.lock().unwrap().values().next().unwrap().data,
        before
    );
    let page: Next = Arc::new(|_| {
        Box::pin(async {
            assert_eq!(
                suprnova::session::session_mut(|session| session.get_flash::<String>("notice"))
                    .flatten()
                    .as_deref(),
                Some("saved")
            );
            Ok(HttpResponse::text("page"))
        })
    });
    serve(&middleware, &config, Some(&cookie), page).await;
}
