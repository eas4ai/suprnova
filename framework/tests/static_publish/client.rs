//! The cookie used by a published-page mutation follows the real CSRF path.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, HttpResponse, MiddlewareRegistry,
    Request, Router, SessionConfig, SessionData, SessionMiddleware, SessionStore,
};

#[derive(Default)]
struct Sessions(Mutex<HashMap<String, SessionData>>);
#[suprnova::async_trait]
impl SessionStore for Sessions {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }
    async fn write(&self, data: &SessionData) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().insert(data.id.clone(), data.clone());
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, _: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[tokio::test]
async fn pub_005_cookie_bootstrap_allows_mutation_and_private_get_is_no_store() {
    let _container = TestContainer::fake();
    Crypt::init(EncryptionKey::generate());
    let router: Router = Router::new()
        .get("/form", |_: Request| async {
            Ok(HttpResponse::html("<form></form>"))
        })
        .post("/save", |_: Request| async {
            Ok(HttpResponse::text("saved"))
        })
        .into();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            config.clone(),
            Arc::new(Sessions::default()),
        ))
        .append(CsrfMiddleware::new().with_session_config(&config));
    let client = TestClient::new(router, registry);
    let bootstrap = client
        .get("/form?__suprnova_origin=1")
        .header("Cache-Control", "no-store")
        .send()
        .await;
    bootstrap.assert_ok();
    assert_eq!(bootstrap.header("Cache-Control"), Some("private, no-store"));
    let token = bootstrap
        .cookie("XSRF-TOKEN")
        .expect("private GET issues same-origin CSRF cookie");
    assert!(!bootstrap.body_text().contains(&token));
    client
        .post("/save")
        .header("X-XSRF-TOKEN", &token)
        .send()
        .await
        .assert_ok()
        .assert_see("saved");
    client
        .post("/save")
        .header("X-XSRF-TOKEN", "wrong")
        .send()
        .await
        .assert_status(419);
}
