//! Auth errors must not become the legacy providerless ID-only mode.

use std::any::Any;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hyper::server::conn::http1::Builder;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use reqwest::Client;
use suprnova::http::text;
use suprnova::session::{
    new_pending_cookies_slot_for_test, new_session_slot_for_test, pending_cookies_scope_for_test,
    session_scope_for_test,
};
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, FrameworkError, Middleware,
    MiddlewareRegistry, Next, Request, Response, Router, UserProvider, handle_request,
};
use tokio::net::TcpListener;
use tokio::time::timeout;

struct User;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        "7".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

struct Provider {
    fail: bool,
}

#[async_trait]
impl UserProvider for Provider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        if self.fail {
            return Err(FrameworkError::internal(
                "No user provider configured: error returned by an actual provider",
            ));
        }
        Ok((id == "7").then(|| Arc::new(User) as Arc<dyn Authenticatable>))
    }

    async fn retrieve_by_credentials(
        &self,
        _credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }

    async fn validate_credentials(
        &self,
        _user: &dyn Authenticatable,
        _credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        Ok(false)
    }
}

struct PersistedIdentity;

#[async_trait]
impl Middleware for PersistedIdentity {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let session = new_session_slot_for_test();
        session
            .lock()
            .expect("session lock")
            .as_mut()
            .unwrap()
            .user_id = Some("7".into());
        session_scope_for_test(
            session,
            pending_cookies_scope_for_test(new_pending_cookies_slot_for_test(), next(request)),
        )
        .await
    }
}

async fn protected_status() -> u16 {
    let router: Router = Router::new()
        .get("/protected", |_request| async { text("reached") })
        .into();
    let router = Arc::new(router);
    let middleware = Arc::new(
        MiddlewareRegistry::new()
            .append(PersistedIdentity)
            .append(AuthMiddleware::new()),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = TestContainer::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, request).await) }
        });
        Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });
    let response = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .get(format!("http://{address}/protected"))
        .header("Connection", "close")
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.text().await.unwrap();
    timeout(Duration::from_secs(5), server)
        .await
        .expect("server timeout")
        .unwrap();
    assert_eq!(body == "reached", status == 200);
    status
}

#[tokio::test]
async fn undefined_default_guard_does_not_authorize_persisted_id() {
    TestContainer::scope(async {
        let manager = AuthManager::new(AuthConfig::new("missing"));
        manager.register_provider("users", Arc::new(Provider { fail: false }));
        TestContainer::singleton(manager);
        assert_eq!(protected_status().await, 500);
    })
    .await;
}

#[tokio::test]
async fn missing_configured_provider_does_not_authorize_persisted_id() {
    TestContainer::scope(async {
        TestContainer::singleton(AuthManager::new(AuthConfig::default()));
        assert_eq!(protected_status().await, 500);
    })
    .await;
}

#[tokio::test]
async fn undefined_default_guard_does_not_fall_back_to_legacy_provider() {
    TestContainer::scope(async {
        TestContainer::singleton(AuthManager::new(AuthConfig::new("missing")));
        TestContainer::bind::<dyn UserProvider>(Arc::new(Provider { fail: false }));
        assert_eq!(protected_status().await, 500);
    })
    .await;
}

#[tokio::test]
async fn named_provider_error_text_does_not_enable_providerless_mode() {
    TestContainer::scope(async {
        let manager = AuthManager::new(AuthConfig::default());
        manager.register_provider("users", Arc::new(Provider { fail: true }));
        TestContainer::singleton(manager);
        assert_eq!(protected_status().await, 500);
    })
    .await;
}

#[tokio::test]
async fn legacy_provider_error_text_does_not_enable_providerless_mode() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn UserProvider>(Arc::new(Provider { fail: true }));
        assert_eq!(protected_status().await, 500);
    })
    .await;
}

#[tokio::test]
async fn truly_providerless_persisted_id_still_reaches_handler() {
    TestContainer::scope(async {
        assert_eq!(protected_status().await, 200);
    })
    .await;
}

#[tokio::test]
async fn legacy_provider_valid_identity_still_reaches_handler() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn UserProvider>(Arc::new(Provider { fail: false }));
        assert_eq!(protected_status().await, 200);
    })
    .await;
}

#[tokio::test]
async fn providerless_login_id_remains_supported() {
    TestContainer::scope(async {
        session_scope_for_test(new_session_slot_for_test(), async {
            Auth::login_id("7").expect("providerless login_id");
            assert_eq!(Auth::id().as_deref(), Some("7"));
        })
        .await;
    })
    .await;
}
