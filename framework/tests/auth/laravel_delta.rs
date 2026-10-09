use std::sync::{Arc, Mutex};
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm::{EntityTrait, QueryOrder};
use sea_orm_migration::SchemaManager;
use suprnova::auth::remember::entity;
use suprnova::auth::request_state;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::session::{DatabaseSessionDriver, SessionData, SessionStore};
use suprnova::testing::TestContainer;
use suprnova::{App, Auth, AuthConfig, AuthManager, DatabaseUserProvider, FrameworkError};

async fn database() -> DbConnection {
    let config = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let db = DbConnection::connect(&config).await.expect("database");
    suprnova::session::migrations::create_sessions_table(
        &SchemaManager::new(db.inner()),
        "d4_sessions",
        suprnova::session::migrations::SessionUserKey::Integer,
    )
    .await
    .expect("sessions table");
    db.inner()
        .execute_unprepared(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT, password TEXT)",
        )
        .await
        .expect("users table");
    let hash = bcrypt::hash("secret", 4).expect("password hash");
    db.inner().execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO users (id, email, password) VALUES (7, 'alice@example.com', ?), (8, 'bob@example.com', ?)",
        [hash.clone().into(), hash.into()])).await.expect("users");
    db.inner().execute_unprepared("CREATE TABLE remember_tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id TEXT NOT NULL, selector TEXT NOT NULL, token_hash TEXT NOT NULL, expires_at DATETIME NOT NULL, created_at DATETIME NOT NULL, last_used_at DATETIME)")
        .await.expect("remember table");
    db.inner().execute_unprepared("INSERT INTO remember_tokens (user_id, selector, token_hash, expires_at, created_at) VALUES ('7','current','hash','2099-01-01 00:00:00','2026-01-01 00:00:00'), ('7','other','hash','2099-01-01 00:00:00','2026-01-01 00:00:00'), ('8','bob','hash','2099-01-01 00:00:00','2026-01-01 00:00:00')")
        .await.expect("remember rows");
    db
}

fn configure(db: DbConnection) -> Arc<DatabaseSessionDriver> {
    TestContainer::singleton(db);
    let manager = AuthManager::new(AuthConfig::default());
    manager.register_provider("users", Arc::new(DatabaseUserProvider::new("users")));
    TestContainer::singleton(manager);
    let store = Arc::new(
        DatabaseSessionDriver::with_table(Duration::from_secs(3600), "d4_sessions")
            .expect("custom session table"),
    );
    App::bind::<dyn SessionStore>(store.clone());
    store
}

fn signed_in(id: &str, user: &str) -> SessionData {
    let mut session = SessionData::new(id.into(), "csrf-preserved".into());
    session.user_id = Some(user.into());
    session.set_auth_guard_for_test("web", user, None);
    session.put("cart", "keep");
    session
}

fn current_session(remember: bool) -> SessionData {
    let mut current = signed_in("current-session", "7");
    if remember {
        current.put(
            "_auth_guards",
            serde_json::json!({"web": {"id": "7", "remember_selector": "current"}}),
        );
    }
    current
}

async fn rows(db: &DbConnection) -> Vec<entity::Model> {
    entity::Entity::find()
        .order_by_asc(entity::Column::Id)
        .all(db.inner())
        .await
        .expect("remember rows")
}

async fn request<F: std::future::Future>(current: SessionData, body: F) -> F::Output {
    let slot = Arc::new(Mutex::new(Some(current)));
    suprnova::session::session_scope_for_test(
        slot,
        request_state::request_state_scope_for_test(body),
    )
    .await
}

#[tokio::test]
async fn logout_preserves_current_session_and_remember_but_revokes_other_devices() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(true);
        store.write(&current).await.expect("current stored");
        store
            .write(&signed_in("other-session", "7"))
            .await
            .expect("other stored");
        store
            .write(&signed_in("bob-session", "8"))
            .await
            .expect("bob stored");
        let mut admin = SessionData::new("admin-session".into(), "csrf".into());
        admin.set_auth_guard_for_test("admin", "7", None);
        store.write(&admin).await.expect("admin stored");
        request(current.clone(), async {
            Auth::logout_other_devices("secret")
                .await
                .expect("logout other devices");
            assert_eq!(Auth::id().as_deref(), Some("7"));
            assert_eq!(
                Auth::user_or_fail()
                    .await
                    .expect("still signed in")
                    .get_auth_identifier(),
                "7"
            );
            let after = suprnova::session::session().expect("session retained");
            assert_eq!(after.id, current.id);
            assert_eq!(after.csrf_token, current.csrf_token);
            assert_eq!(after.data, current.data);
        })
        .await;
        assert!(
            store
                .read("other-session")
                .await
                .expect("other read")
                .is_none()
        );
        assert!(store.read("bob-session").await.expect("bob read").is_some());
        assert!(
            store
                .read("admin-session")
                .await
                .expect("admin read")
                .is_some()
        );
        let persisted = store
            .read("current-session")
            .await
            .expect("read current")
            .expect("current retained");
        assert_eq!(persisted.data, current.data);
        request(persisted, async {
            assert_eq!(
                Auth::user_or_fail()
                    .await
                    .expect("next request signed in")
                    .get_auth_identifier(),
                "7"
            );
        })
        .await;
        let selectors: Vec<_> = rows(&db).await.into_iter().map(|r| r.selector).collect();
        assert_eq!(selectors, ["current", "bob"]);
    })
    .await;
}

#[tokio::test]
async fn logout_other_devices_uses_column_identity_before_payload_identity() {
    use base64::Engine as _;

    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(true);
        store.write(&current).await.expect("current stored");
        for (id, user, payload) in [
            ("legacy-alice", 7, serde_json::json!({"cart": "keep"})),
            ("legacy-bob", 8, serde_json::json!({"cart": "keep"})),
            ("column-alice", 7, serde_json::json!({"_suprnova_user": "8"})),
            ("column-bob", 8, serde_json::json!({"_suprnova_user": "7"})),
        ] {
            let payload = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
            db.inner()
                .execute_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "INSERT INTO d4_sessions (id, user_id, payload, last_activity) VALUES (?, ?, ?, ?)",
                    [
                        id.into(),
                        user.into(),
                        payload.into(),
                        suprnova::clock::now().timestamp().into(),
                    ],
                ))
                .await
                .expect("legacy session stored");
            let persisted = store
                .read(id)
                .await
                .expect("read legacy")
                .expect("legacy present");
            request(persisted, async {
                assert_eq!(
                    Auth::user_or_fail()
                        .await
                        .expect("column user signed in")
                        .get_auth_identifier(),
                    user.to_string()
                );
            })
            .await;
        }
        request(current.clone(), async {
            Auth::logout_other_devices("secret")
                .await
                .expect("logout other devices");
            assert_eq!(Auth::id().as_deref(), Some("7"));
            assert_eq!(
                suprnova::session::session().expect("current retained").data,
                current.data
            );
        })
        .await;
        for id in ["legacy-alice", "column-alice"] {
            assert!(
                store.read(id).await.expect("other read").is_none(),
                "other device {id} must be deleted"
            );
        }
        for id in ["legacy-bob", "column-bob", "current-session"] {
            assert!(
                store.read(id).await.expect("retained read").is_some(),
                "session {id} must survive"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn guard_session_revocation_uses_column_identity_before_payload_identity() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let mut bob = SessionData::new("column-bob".into(), "csrf".into());
        bob.user_id = Some("7".into());
        store.write(&bob).await.expect("payload user stored");
        db.inner()
            .execute_unprepared("UPDATE d4_sessions SET user_id = 8 WHERE id = 'column-bob'")
            .await
            .expect("column user stored");
        let destroyed = store
            .destroy_guard_sessions("web", "7")
            .await
            .expect("revoke guard sessions");
        assert_eq!(destroyed.count, 0);
        assert!(destroyed.ids.is_empty());
        assert_eq!(
            store
                .read("column-bob")
                .await
                .expect("read bob")
                .expect("bob retained")
                .user_id
                .as_deref(),
            Some("8")
        );
    })
    .await;
}

#[tokio::test]
async fn wrong_password_returns_password_validation_and_changes_nothing() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(true);
        let other = signed_in("other-session", "7");
        store.write(&current).await.expect("current");
        store.write(&other).await.expect("other");
        let before = rows(&db).await;
        request(current.clone(), async {
            let error = Auth::logout_other_devices("wrong")
                .await
                .expect_err("wrong password");
            match error {
                FrameworkError::Validation(errors) => {
                    assert_eq!(errors.errors.len(), 1);
                    assert!(errors.errors.contains_key("password"));
                }
                other => panic!("expected password validation, got {other:?}"),
            }
            assert_eq!(
                suprnova::session::session().expect("unchanged").data,
                current.data
            );
            assert_eq!(Auth::id().as_deref(), Some("7"));
        })
        .await;
        assert_eq!(rows(&db).await, before);
        assert_eq!(
            store
                .read("current-session")
                .await
                .expect("current")
                .expect("present")
                .data,
            current.data
        );
        assert_eq!(
            store
                .read("other-session")
                .await
                .expect("other")
                .expect("present")
                .data,
            other.data
        );
    })
    .await;
}

#[tokio::test]
async fn without_current_remember_token_all_user_tokens_end() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(false);
        store.write(&current).await.expect("current");
        request(current, async {
            Auth::logout_other_devices("secret")
                .await
                .expect("only device");
            assert!(Auth::check());
            Auth::logout_other_devices("secret")
                .await
                .expect("repeat is harmless");
        })
        .await;
        assert_eq!(
            rows(&db)
                .await
                .iter()
                .map(|r| r.selector.as_str())
                .collect::<Vec<_>>(),
            ["bob"]
        );
        assert!(
            store
                .read("current-session")
                .await
                .expect("current")
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn guests_cannot_revoke_sessions_or_remember_tokens() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        store
            .write(&signed_in("other-session", "7"))
            .await
            .expect("other");
        let before = rows(&db).await;
        request(SessionData::new("guest".into(), "csrf".into()), async {
            assert!(matches!(
                Auth::logout_other_devices("secret").await,
                Err(FrameworkError::Unauthorized)
            ));
        })
        .await;
        assert!(store.read("other-session").await.expect("other").is_some());
        assert_eq!(rows(&db).await, before);
    })
    .await;
}

struct UnsupportedStore;
#[async_trait::async_trait]
impl SessionStore for UnsupportedStore {
    async fn read(&self, _: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }
    async fn write(&self, _: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy(&self, _: &str) -> Result<(), FrameworkError> {
        panic!("current session must never be destroyed")
    }
    async fn destroy_for_user(&self, _: &str) -> Result<u64, FrameworkError> {
        panic!("all-session revocation is forbidden")
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[tokio::test]
async fn configured_store_without_selective_revocation_returns_an_error() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(true);
        store.write(&current).await.expect("current");
        store
            .write(&signed_in("other-session", "7"))
            .await
            .expect("other");
        App::bind::<dyn SessionStore>(Arc::new(UnsupportedStore));
        let before = rows(&db).await;
        request(current, async {
            let error = Auth::logout_other_devices("secret")
                .await
                .expect_err("unsupported custom backend");
            assert!(error.to_string().contains("preserve the current session"));
            assert_eq!(Auth::id().as_deref(), Some("7"));
        })
        .await;
        assert!(store.read("other-session").await.expect("other").is_some());
        assert_eq!(rows(&db).await, before);
    })
    .await;
}

#[tokio::test]
async fn remember_storage_failure_is_reported_and_current_session_survives() {
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let current = current_session(true);
        store.write(&current).await.expect("current");
        db.inner()
            .execute_unprepared("DROP TABLE remember_tokens")
            .await
            .expect("simulate failed storage");
        request(current, async {
            assert!(Auth::logout_other_devices("secret").await.is_err());
            assert_eq!(Auth::id().as_deref(), Some("7"));
        })
        .await;
        assert!(
            store
                .read("current-session")
                .await
                .expect("current")
                .is_some()
        );
    })
    .await;
}

struct AllowingLimiter;
#[async_trait::async_trait]
impl suprnova::RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        Ok(true)
    }
    async fn retry_after(
        &self,
        _: &str,
        _: &suprnova::SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        Ok(None)
    }
}

async fn magnetar_device(email: &str) -> SessionData {
    let slot = suprnova::session::new_session_slot_for_test();
    let cookies = suprnova::session::new_pending_cookies_slot_for_test();
    suprnova::session::session_scope_for_test(
        slot.clone(),
        suprnova::session::pending_cookies_scope_for_test(
            cookies,
            request_state::request_state_scope_for_test(async {
                let (user, _) = Auth::password()
                    .authenticate(email, "secret", None, None)
                    .await
                    .expect("Magnetar sign-in");
                Auth::issue_remember_cookie(&user.id.to_string(), 60)
                    .await
                    .expect("remember device");
            }),
        ),
    )
    .await;
    slot.lock()
        .expect("session slot")
        .clone()
        .expect("device session")
}

#[tokio::test]
async fn magnetar_sessions_and_remember_credentials_preserve_only_the_current_device() {
    if crate::own_process_async::delegate(
        module_path!(),
        "magnetar_sessions_and_remember_credentials_preserve_only_the_current_device",
    )
    .await
    {
        return;
    }
    TestContainer::scope(async {
        let db = database().await;
        let store = configure(db.clone());
        let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
        App::bind::<dyn suprnova::RateLimiterDriver>(Arc::new(AllowingLimiter));
        suprnova::init_magnetar(suprnova::MagnetarConfig::from_sea_orm(db.inner().clone())).await.expect("default Magnetar engine");
        db.inner().execute_unprepared("INSERT INTO app_users (id, email, name, password_hash, auth_epoch, email_verified_at) SELECT id, email, 'Imported', password, 0, '2026-01-01 00:00:00' FROM users")
            .await.expect("import users with the same credentials");
        let other = magnetar_device("alice@example.com").await;
        let current = magnetar_device("alice@example.com").await;
        let bob = magnetar_device("bob@example.com").await;
        for session in [&other, &current, &bob] { store.write(session).await.expect("store framework session"); }
        let binding = current.magnetar_web_binding().expect("current binding");
        let remembered = magnetar::default_schema::remembers::Entity::find()
            .all(db.inner()).await.expect("remember credentials");
        assert_eq!(remembered.len(), 3);
        request(current.clone(), async {
            assert!(matches!(Auth::logout_other_devices("wrong").await, Err(FrameworkError::Validation(_))));
            assert_eq!(suprnova::magnetar_integration::list_sessions("7").await.expect("unchanged sessions").len(), 2);
            assert_eq!(magnetar::default_schema::remembers::Entity::find().all(db.inner()).await.expect("unchanged remember credentials"), remembered);
            Auth::logout_other_devices("secret").await.expect("logout other Magnetar devices");
            assert_eq!(Auth::id().as_deref(), Some("7"));
            assert_eq!(suprnova::session::session().expect("current session").data, current.data);
        }).await;
        let active = suprnova::magnetar_integration::list_sessions("7").await.expect("active current session");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, binding.session_id);
        assert_eq!(suprnova::magnetar_integration::list_sessions("8").await.expect("bob survives").len(), 1);
        assert!(store.read(&other.id).await.expect("other framework session").is_none());
        assert!(store.read(&current.id).await.expect("current framework session").is_some());
        assert!(store.read(&bob.id).await.expect("bob framework session").is_some());
        let remaining = magnetar::default_schema::remembers::Entity::find().all(db.inner()).await.expect("remaining remember credentials");
        assert_eq!(remaining.len(), 2);
        let selector = current.data["_auth_guards"]["web"]["remember_selector"].as_str().expect("current selector");
        assert!(remaining.iter().any(|row| row.user_id == "7" && row.selector == selector));
        assert!(remaining.iter().any(|row| row.user_id == "8"));
    }).await;
}
