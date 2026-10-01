//! #132: `SessionConfig.table_name` reaches the database session driver.
//!
//! Every test here runs on a database whose only session table is
//! `app_sessions`. A driver that still wrote to `sessions` would fail on
//! the missing table, so a passing test proves the configured name is
//! the one the driver reads and writes.

use std::sync::Arc;
use std::time::Duration;

use sea_orm_migration::MigrationName;
use sea_orm_migration::prelude::*;
use suprnova::session::{DatabaseSessionDriver, SessionConfig, SessionData, SessionMiddleware};
use suprnova::testing::TestDatabase;
use suprnova::{Crypt, EncryptionKey, SessionStore};

const TABLE: &str = "app_sessions";

/// Migrator that creates `app_sessions` with the default table's shape and
/// no `sessions` table at all. `destroy_for_user.rs` uses it too.
pub(crate) struct AppSessionsMigrator;

#[async_trait::async_trait]
impl MigratorTrait for AppSessionsMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(AppSessionsMigration)]
    }
}

struct AppSessionsMigration;

impl MigrationName for AppSessionsMigration {
    fn name(&self) -> &str {
        "m20260930_000001_create_app_sessions_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AppSessionsMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(AppSessions::Table)
                    .col(
                        ColumnDef::new(AppSessions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AppSessions::UserId).string().null())
                    .col(ColumnDef::new(AppSessions::Payload).text().not_null())
                    .col(ColumnDef::new(AppSessions::CsrfToken).string().not_null())
                    .col(
                        ColumnDef::new(AppSessions::LastActivity)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AppSessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum AppSessions {
    Table,
    Id,
    UserId,
    Payload,
    CsrfToken,
    LastActivity,
}

fn driver() -> DatabaseSessionDriver {
    DatabaseSessionDriver::with_table(Duration::from_secs(3600), TABLE)
        .expect("app_sessions is a valid table name")
}

async fn app_session_rows(db: &TestDatabase) -> i64 {
    db.fetch_one("SELECT COUNT(*) AS n FROM app_sessions", vec![])
        .await
        .expect("count app_sessions rows")
        .try_get("", "n")
        .expect("count column")
}

async fn assert_no_sessions_table(db: &TestDatabase) {
    let manager = SchemaManager::new(db.conn());
    assert!(
        !manager.has_table("sessions").await.unwrap(),
        "the fixture must not have a `sessions` table, or a driver writing there could pass"
    );
}

#[tokio::test]
async fn with_table_writes_reads_updates_and_destroys_in_the_named_table() {
    let db = TestDatabase::fresh::<AppSessionsMigrator>().await.unwrap();
    assert_no_sessions_table(&db).await;
    let driver = driver();

    let mut session = SessionData::new("custom-table-sess".into(), "csrf".into());
    session.user_id = Some("u-1".into());
    session.put("cart_items", 3);
    driver.write(&session).await.unwrap();
    assert_eq!(app_session_rows(&db).await, 1);

    let mut loaded = driver
        .read("custom-table-sess")
        .await
        .unwrap()
        .expect("the session written to app_sessions reads back");
    assert!(loaded.loaded_from_store);
    assert_eq!(loaded.user_id.as_deref(), Some("u-1"));
    assert_eq!(loaded.csrf_token, "csrf");
    assert_eq!(loaded.get::<i64>("cart_items"), Some(3));

    // A session read from the store writes back through the update-only
    // branch, which must target the same table.
    loaded.put("cart_items", 4);
    driver.write(&loaded).await.unwrap();
    let reloaded = driver.read("custom-table-sess").await.unwrap().unwrap();
    assert_eq!(reloaded.get::<i64>("cart_items"), Some(4));
    assert_eq!(app_session_rows(&db).await, 1);

    driver.destroy("custom-table-sess").await.unwrap();
    assert!(driver.read("custom-table-sess").await.unwrap().is_none());
    assert_eq!(app_session_rows(&db).await, 0);
}

#[tokio::test]
async fn destroy_for_user_and_gc_act_on_the_named_table() {
    let db = TestDatabase::fresh::<AppSessionsMigrator>().await.unwrap();
    let driver = driver();

    let mut alice = SessionData::new("alice-sess".into(), "csrf1".into());
    alice.user_id = Some("alice-uid".into());
    driver.write(&alice).await.unwrap();
    // A named-guard-only session carries the principal in its payload, so
    // the second revocation phase has to scan the named table too.
    let mut alice_admin = SessionData::new("alice-admin-sess".into(), "csrf2".into());
    alice_admin.data.insert(
        "_auth_guards".to_string(),
        serde_json::json!({ "admin": { "id": "alice-uid" } }),
    );
    driver.write(&alice_admin).await.unwrap();
    let mut bob = SessionData::new("bob-sess".into(), "csrf3".into());
    bob.user_id = Some("bob-uid".into());
    driver.write(&bob).await.unwrap();

    assert_eq!(driver.destroy_for_user("alice-uid").await.unwrap(), 2);
    assert!(driver.read("alice-sess").await.unwrap().is_none());
    assert!(driver.read("alice-admin-sess").await.unwrap().is_none());
    assert!(driver.read("bob-sess").await.unwrap().is_some());

    let fresh = SessionData::new("fresh-sess".into(), "csrf4".into());
    driver.write(&fresh).await.unwrap();
    db.execute_unprepared(
        "UPDATE app_sessions SET last_activity = '2000-01-01 00:00:00' WHERE id = 'bob-sess'",
    )
    .await
    .unwrap();

    assert_eq!(driver.gc().await.unwrap(), 1, "gc collects the stale row");
    assert!(driver.read("bob-sess").await.unwrap().is_none());
    assert!(driver.read("fresh-sess").await.unwrap().is_some());
    assert_eq!(app_session_rows(&db).await, 1);
}

#[tokio::test]
async fn two_factor_migration_replaces_the_row_in_the_named_table() {
    let db = TestDatabase::fresh::<AppSessionsMigrator>().await.unwrap();
    let driver = driver();

    let pending = SessionData::new("pending-two-factor".into(), "pending-csrf".into());
    driver.write(&pending).await.unwrap();
    let mut authenticated = pending.clone();
    authenticated.rotate_id("authenticated-two-factor");
    authenticated.user_id = Some("promoted-user".into());

    driver
        .migrate_two_factor_session("pending-two-factor", &authenticated)
        .await
        .expect("the migration runs against app_sessions");

    assert!(driver.read("pending-two-factor").await.unwrap().is_none());
    let stored = driver
        .read("authenticated-two-factor")
        .await
        .unwrap()
        .expect("the authenticated row exists");
    assert_eq!(stored.user_id.as_deref(), Some("promoted-user"));
    assert_eq!(app_session_rows(&db).await, 1);
}

#[test]
fn with_table_rejects_names_that_are_not_plain_identifiers() {
    let too_long = "a".repeat(64);
    for bad in ["", "1abc", "bad name", "x;drop", too_long.as_str()] {
        let error = match DatabaseSessionDriver::with_table(Duration::from_secs(60), bad) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("{bad:?} must be rejected"),
        };
        assert!(
            error.contains(&format!("{bad:?}")),
            "the error names the rejected value {bad:?}: {error}"
        );
    }

    let longest = "a".repeat(63);
    for good in [
        "sessions",
        "app_sessions",
        "_sessions",
        "Sessions2",
        longest.as_str(),
    ] {
        assert!(
            DatabaseSessionDriver::with_table(Duration::from_secs(60), good).is_ok(),
            "{good:?} is a valid table name"
        );
    }
}

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

#[tokio::test]
async fn session_middleware_persists_to_the_configured_table() {
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let db = TestDatabase::fresh::<AppSessionsMigrator>().await.unwrap();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.table_name = TABLE.to_string();
    let middleware = SessionMiddleware::new(config);

    let next: Next = Arc::new(|_request| {
        Box::pin(async {
            suprnova::session::session_mut(|session| session.put("visited", "yes"));
            Ok(suprnova::HttpResponse::text("ok"))
        })
    });
    let response = middleware
        .handle(
            crate::cookie_prefix_roundtrip::post_request(None).await,
            next,
        )
        .await;
    assert!(
        response.is_ok(),
        "the session persists without error into the configured table"
    );

    assert_eq!(app_session_rows(&db).await, 1);
    let row = db
        .fetch_one("SELECT payload FROM app_sessions", vec![])
        .await
        .unwrap();
    let payload: String = row.try_get("", "payload").unwrap();
    assert!(payload.contains("visited"), "{payload}");
}
