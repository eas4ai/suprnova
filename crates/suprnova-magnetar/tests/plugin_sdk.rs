//! Contract tests for registry, wire dispatch, effects, and feature absence.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use magnetar::Result;
use magnetar::plugin::*;
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata, StoredSession,
};
use magnetar::storage::{
    AuthTransaction, CeremonyRecord, CeremonyStore, IssueToken, IssuedToken, NewCeremony,
    PresentedToken, TokenStore,
};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::json;
use sha2::{Digest, Sha256};

#[path = "fixtures/storage_schema.rs"]
mod fixture;
use fixture::StorageSchema;
use fixture::sql_stores::SqlSessionStore;

struct NullStorage;
#[async_trait]
impl TokenStore for NullStorage {
    async fn issue(&self, _input: IssueToken) -> Result<IssuedToken> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }
    async fn consume(
        &self,
        _token: PresentedToken,
        _purpose: &str,
    ) -> Result<magnetar::storage::ConsumedToken> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }
    async fn consume_in(
        &self,
        _tx: &mut AuthTransaction<'_>,
        _token: PresentedToken,
        _purpose: &str,
    ) -> Result<magnetar::storage::ConsumedToken> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }
    async fn check(&self, _token: PresentedToken, _purpose: &str) -> Result<bool> {
        Ok(false)
    }
}
#[async_trait]
impl CeremonyStore for NullStorage {
    async fn create(&self, _input: NewCeremony) -> Result<CeremonyRecord> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }
    async fn consume(&self, _selector: &str, _kind: &str) -> Result<Option<CeremonyRecord>> {
        Ok(None)
    }
    async fn peek(&self, _selector: &str, _kind: &str) -> Result<Option<CeremonyRecord>> {
        Ok(None)
    }
    async fn transition(
        &self,
        _selector: &str,
        _kind: &str,
        _expected: &str,
        _next: &str,
    ) -> Result<bool> {
        Ok(false)
    }
}
struct Allow;
#[async_trait]
impl FactorGate for Allow {
    async fn complete_sign_in(
        &self,
        _principal: magnetar::auth::VerifiedPrincipal,
        _context: magnetar::auth::AuthenticationContext,
    ) -> Result<magnetar::auth::SignInDecision> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }

    async fn complete_challenge(
        &self,
        _selector: &str,
        _code: &str,
    ) -> Result<magnetar::sessions::SessionGrant> {
        Err(magnetar::Error::Internal {
            message: "unused".into(),
        })
    }
}
#[async_trait]
impl Encryptor for Allow {
    async fn encrypt(&self, value: &[u8]) -> Result<Vec<u8>> {
        Ok(value.to_vec())
    }
    async fn decrypt(&self, value: &[u8]) -> Result<Vec<u8>> {
        Ok(value.to_vec())
    }
}
#[async_trait]
impl AbuseLimiter for Allow {
    async fn acquire(
        &self,
        _key: &str,
        _policy: magnetar::abuse::AbusePolicy,
    ) -> Result<magnetar::abuse::Permit> {
        Ok(magnetar::abuse::Permit::Allowed { retry_after: None })
    }
}
#[async_trait]
impl MailDriver for Allow {
    async fn send(&self, _message: MailMessage) -> Result<()> {
        Ok(())
    }
}
#[async_trait]
impl HttpTransport for Allow {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: vec![],
        })
    }
}
#[async_trait]
impl LinkGenerator for Allow {
    async fn url_for(&self, _route_name: &str, _params: &[(String, String)]) -> Result<String> {
        Ok("https://example.test".into())
    }
}

async fn context() -> PluginContext<StorageSchema> {
    let database = fixture::database().await;
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO storage_users (id, email, auth_epoch)
             VALUES (2, 'web@example.test', 0)"
                .to_owned(),
        ))
        .await
        .expect("seed session users");
    let store = Arc::new(SqlSessionStore(database));
    let bearer_digest: [u8; 32] = Sha256::digest(b"bearer").into();
    for session in [
        StoredSession {
            session_id: "session".into(),
            user_id: "1".into(),
            auth_epoch: 0,
            token_hash: bearer_digest,
            token_digest: bearer_digest,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            revoked_at: None,
            metadata: SessionMetadata::default(),
        },
        StoredSession {
            session_id: "web-session".into(),
            user_id: "2".into(),
            auth_epoch: 0,
            token_hash: [5; 32],
            token_digest: [5; 32],
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            revoked_at: None,
            metadata: SessionMetadata::default(),
        },
    ] {
        store
            .insert_session_if_epoch_current(session)
            .await
            .expect("seed live session");
    }
    let sessions = Arc::new(OpaqueSessionProvider::new(store, OpaqueConfig::default()));
    PluginContext::new(
        Arc::new(NullStorage),
        sessions,
        Arc::new(Allow),
        Arc::new(Allow),
        Arc::new(Allow),
        Arc::new(Allow),
        Arc::new(Allow),
        Arc::new(Allow),
    )
}

struct ContractPlugin;
#[async_trait]
impl Plugin<StorageSchema> for ContractPlugin {
    fn name(&self) -> &str {
        "contract"
    }
    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![RouteDescriptor::new(
            Method::Post,
            "/contract/{id}",
            "contract.run",
        )]
    }
    async fn init(&self, _context: InitContext<'_, StorageSchema>) -> PluginResult<()> {
        Ok(())
    }
    async fn before_request(
        &self,
        context: RequestContext<'_, StorageSchema>,
    ) -> PluginResult<BeforeRequest> {
        if context.request.headers.contains_key("x-short-circuit") {
            Ok(BeforeRequest::Respond(WireResponse::json(
                json!({"short": true}),
            )))
        } else {
            Ok(BeforeRequest::Continue)
        }
    }
    async fn handle(
        &self,
        context: RequestContext<'_, StorageSchema>,
    ) -> PluginResult<WireResponse> {
        Ok(WireResponse::json(json!({
            "id": context.request.path_params.get("id"),
            "user_id": context.session.map(|session| session.user_id()),
        })))
    }
}
struct CountingHook {
    calls: Arc<AtomicUsize>,
    panic: bool,
}

#[async_trait]
impl LifecycleHook<StorageSchema> for CountingHook {
    async fn on_event(
        &self,
        _context: HookContext<'_, StorageSchema>,
        _event: LifecycleEvent,
    ) -> PluginResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic, "intentional lifecycle panic");
        Ok(())
    }
}

struct HookPlugin {
    calls: Arc<AtomicUsize>,
    panic: bool,
}

#[async_trait]
impl Plugin<StorageSchema> for HookPlugin {
    fn name(&self) -> &str {
        "hook"
    }
    fn routes(&self) -> Vec<RouteDescriptor> {
        Vec::new()
    }
    async fn handle(
        &self,
        _context: RequestContext<'_, StorageSchema>,
    ) -> PluginResult<WireResponse> {
        Ok(WireResponse::ok())
    }
    fn lifecycle_hooks(&self) -> Vec<Arc<dyn LifecycleHook<StorageSchema>>> {
        vec![Arc::new(CountingHook {
            calls: Arc::clone(&self.calls),
            panic: self.panic,
        })]
    }
}

#[tokio::test]
async fn duplicate_lifecycle_delivery_reaches_idempotent_hook() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = PluginRegistry::new(context().await)
        .register(HookPlugin {
            calls: Arc::clone(&calls),
            panic: false,
        })
        .build()
        .await
        .unwrap();
    let event = LifecycleEvent::new("duplicate", LifecycleEventKind::UserCreated, "u");
    registry.dispatch_lifecycle(event.clone()).await.unwrap();
    registry.dispatch_lifecycle(event).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_panics_are_recorded_without_unwinding() {
    let calls = Arc::new(AtomicUsize::new(0));
    let registry = PluginRegistry::new(context().await)
        .register(HookPlugin { calls, panic: true })
        .build()
        .await
        .unwrap();
    assert!(
        registry
            .dispatch_lifecycle(LifecycleEvent::new(
                "panic",
                LifecycleEventKind::UserCreated,
                "u",
            ))
            .await
            .is_err()
    );
    assert!(matches!(
        registry.take_lifecycle_errors().as_slice(),
        [PluginError::LifecyclePanic { .. }]
    ));
}
#[tokio::test]
async fn registry_dispatches_init_before_request_and_handle() {
    let registry = PluginRegistry::new(context().await)
        .register(ContractPlugin)
        .build()
        .await
        .unwrap();
    registry.init().await.unwrap();
    let mut request = WireRequest::new(Method::Post, "/contract/abc");
    let response = registry
        .handle(request.clone())
        .await
        .unwrap()
        .into_effects();
    assert_eq!(response.body, Some(json!({"id": "abc", "user_id": null})));
    request.headers.insert("x-short-circuit".into(), "1".into());
    assert!(matches!(
        registry.before_request(&request).await.unwrap(),
        BeforeRequest::Respond(_)
    ));
}

#[tokio::test]
async fn erased_facade_forwards_bound_credential() {
    let registry = PluginRegistry::new(context().await)
        .register(ContractPlugin)
        .build()
        .await
        .unwrap();
    let facade: &dyn ErasedPluginFacade = &registry;
    let request = WireRequest::new(Method::Post, "/contract/abc");
    let response = facade
        .handle_bound(request, Some(BearerCredential::new("bearer")))
        .await
        .unwrap()
        .into_effects();
    assert_eq!(response.body, Some(json!({"id": "abc", "user_id": "1"})));
}

#[tokio::test]
async fn erased_facade_forwards_web_binding() {
    let registry = PluginRegistry::new(context().await)
        .register(ContractPlugin)
        .build()
        .await
        .unwrap();
    let facade: &dyn ErasedPluginFacade = &registry;
    let response = facade
        .handle_web_binding(
            WireRequest::new(Method::Post, "/contract/abc"),
            &magnetar::sessions::WebSessionBinding {
                session_id: "web-session".into(),
                token_digest: [5; 32],
            },
        )
        .await
        .unwrap()
        .into_effects();
    assert_eq!(response.body, Some(json!({"id": "abc", "user_id": "2"})));
}

#[tokio::test]
async fn disabled_route_is_absent_and_collisions_are_rejected() {
    struct Disabled;
    #[async_trait]
    impl Plugin<StorageSchema> for Disabled {
        fn name(&self) -> &str {
            "disabled"
        }
        fn routes(&self) -> Vec<RouteDescriptor> {
            vec![RouteDescriptor::new(Method::Get, "/off", "off").disabled()]
        }
        async fn handle(
            &self,
            _context: RequestContext<'_, StorageSchema>,
        ) -> PluginResult<WireResponse> {
            Ok(WireResponse::ok())
        }
    }
    let registry = PluginRegistry::new(context().await)
        .register(Disabled)
        .build()
        .await
        .unwrap();
    assert!(registry.route_names().is_empty());
    assert!(matches!(
        registry.handle(WireRequest::new(Method::Get, "/off")).await,
        Err(PluginError::RouteNotFound { .. })
    ));
    struct EmptyPlugin;
    #[async_trait]
    impl Plugin<StorageSchema> for EmptyPlugin {
        fn name(&self) -> &str {
            "  "
        }
        fn routes(&self) -> Vec<RouteDescriptor> {
            Vec::new()
        }
        async fn handle(
            &self,
            _context: RequestContext<'_, StorageSchema>,
        ) -> PluginResult<WireResponse> {
            Ok(WireResponse::ok())
        }
    }
    assert!(matches!(
        PluginRegistry::new(context().await)
            .register(EmptyPlugin)
            .build()
            .await,
        Err(PluginError::InvalidComposition { plugin, message })
            if plugin == "  " && message.contains("plugin name")
    ));
    struct EmptyRoute;
    #[async_trait]
    impl Plugin<StorageSchema> for EmptyRoute {
        fn name(&self) -> &str {
            "empty-route"
        }
        fn routes(&self) -> Vec<RouteDescriptor> {
            vec![RouteDescriptor::new(Method::Get, "/empty", "   ")]
        }
        async fn handle(
            &self,
            _context: RequestContext<'_, StorageSchema>,
        ) -> PluginResult<WireResponse> {
            Ok(WireResponse::ok())
        }
    }
    assert!(matches!(
        PluginRegistry::new(context().await)
            .register(EmptyRoute)
            .build()
            .await,
        Err(PluginError::InvalidComposition { plugin, message })
            if plugin == "empty-route" && message.contains("route name")
    ));
    struct DuplicateRoutes;
    #[async_trait]
    impl Plugin<StorageSchema> for DuplicateRoutes {
        fn name(&self) -> &str {
            "duplicate-routes"
        }
        fn routes(&self) -> Vec<RouteDescriptor> {
            vec![
                RouteDescriptor::new(Method::Get, "/same", "same-a"),
                RouteDescriptor::new(Method::Post, "/other", "same-a"),
            ]
        }
        async fn handle(
            &self,
            _context: RequestContext<'_, StorageSchema>,
        ) -> PluginResult<WireResponse> {
            Ok(WireResponse::ok())
        }
    }
    assert!(matches!(
        PluginRegistry::new(context().await)
            .register(DuplicateRoutes)
            .build()
            .await,
        Err(PluginError::InvalidComposition { message, .. }) if message.contains("duplicate route name")
    ));

    struct OverlapRoutes;
    #[async_trait]
    impl Plugin<StorageSchema> for OverlapRoutes {
        fn name(&self) -> &str {
            "overlap-routes"
        }
        fn routes(&self) -> Vec<RouteDescriptor> {
            vec![
                RouteDescriptor::new(Method::Get, "/users/{id}", "users.by_id"),
                RouteDescriptor::new(Method::Get, "/users/me", "users.me"),
            ]
        }
        async fn handle(
            &self,
            _context: RequestContext<'_, StorageSchema>,
        ) -> PluginResult<WireResponse> {
            Ok(WireResponse::ok())
        }
    }
    assert!(matches!(
        PluginRegistry::new(context().await)
            .register(OverlapRoutes)
            .build()
            .await,
        Err(PluginError::InvalidComposition { message, .. }) if message.contains("overlapping route")
    ));

    assert!(matches!(
        PluginRegistry::new(context().await)
            .register(ContractPlugin)
            .register(ContractPlugin)
            .build()
            .await,
        Err(PluginError::InvalidComposition { message, .. }) if message.contains("duplicate plugin")
    ));
}
