//! Phase 11 R1 - verify `SessionStore::destroy_for_user` revokes every
//! session row for a given user id, leaving rows belonging to other
//! users untouched. The wire-up from `PasswordReset::complete` →
//! `session::destroy_all_for_user` → `DatabaseSessionDriver::destroy_for_user`
//! is then trusted (single-line orchestration), and the surface contract
//! is exercised here.
//!
//! Boots a fresh in-memory SQLite via `TestDatabase::fresh::<M>` with a
//! migrator that ships the framework session schema inline. Mirrors the
//! app-level `m20251208_220000_create_sessions_table` migration so the
//! framework test doesn't need the example-app crate's migration
//! registry.

use sea_orm_migration::MigrationName;
use sea_orm_migration::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::TransactionTrait;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::session::{DatabaseSessionDriver, SessionData, SessionStore};
use suprnova::testing::{TestContainer, TestContainerGuard, TestDatabase};
use suprnova::{FrameworkError, SessionMigrationError};

/// Migrator containing just the sessions table - matches the schema
/// the example app installs in production via
/// `app/src/migrations/m20251208_220000_create_sessions_table.rs`.
struct TestMigrator;

async fn multi_connection_test_database() -> (TestContainerGuard, DbConnection, PathBuf) {
    let guard = TestContainer::fake();
    let target_dir = std::env::current_dir()
        .expect("test workspace")
        .join("target");
    std::fs::create_dir_all(&target_dir).expect("create target directory");
    let database_path = target_dir.join(format!("p3_07_{}.sqlite", uuid::Uuid::new_v4().simple()));
    let config = DatabaseConfig::builder()
        .url(format!("sqlite://{}?mode=rwc", database_path.display()))
        .max_connections(3)
        .min_connections(3)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config)
        .await
        .expect("connect shared-memory SQLite pool");
    TestMigrator::up(connection.inner(), None)
        .await
        .expect("migrate file-backed SQLite pool");
    TestContainer::singleton(connection.clone());
    (guard, connection, database_path)
}

#[async_trait::async_trait]
impl MigratorTrait for TestMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(SessionsMigration)]
    }
}

struct SessionsMigration;

impl MigrationName for SessionsMigration {
    fn name(&self) -> &str {
        "m20251208_220000_create_sessions_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for SessionsMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Sessions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Sessions::UserId).string().null())
                    .col(ColumnDef::new(Sessions::Payload).text().not_null())
                    .col(ColumnDef::new(Sessions::CsrfToken).string().not_null())
                    .col(
                        ColumnDef::new(Sessions::LastActivity)
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
            .drop_table(Table::drop().table(Sessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    UserId,
    Payload,
    CsrfToken,
    LastActivity,
}

struct LegacySessionStore {
    mutations: AtomicUsize,
}

#[async_trait::async_trait]
impl SessionStore for LegacySessionStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }

    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[tokio::test]
async fn legacy_store_atomic_migration_default_fails_before_mutation() {
    let store = LegacySessionStore {
        mutations: AtomicUsize::new(0),
    };
    let replacement = SessionData::new("replacement".into(), "csrf".into());

    let error = store
        .migrate_two_factor_session("pending", &replacement)
        .await
        .expect_err("legacy stores must fail closed instead of emulating atomic migration");

    assert!(matches!(&error, SessionMigrationError::RolledBack(_)));
    assert!(error.to_string().contains("does not support atomic"));
    assert_eq!(store.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn destroy_for_user_removes_only_that_users_rows() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    // Two sessions for alice, one for bob.
    let mut alice1 = SessionData::new("alice-sess-1".into(), "csrf1".into());
    alice1.user_id = Some("alice-uid".into());
    driver.write(&alice1).await.unwrap();

    let mut alice2 = SessionData::new("alice-sess-2".into(), "csrf2".into());
    alice2.user_id = Some("alice-uid".into());
    driver.write(&alice2).await.unwrap();

    let mut bob1 = SessionData::new("bob-sess-1".into(), "csrf3".into());
    bob1.user_id = Some("bob-uid".into());
    driver.write(&bob1).await.unwrap();

    // Preconditions: all three readable.
    assert!(driver.read("alice-sess-1").await.unwrap().is_some());
    assert!(driver.read("alice-sess-2").await.unwrap().is_some());
    assert!(driver.read("bob-sess-1").await.unwrap().is_some());

    // Destroy alice's sessions.
    let deleted = driver.destroy_for_user("alice-uid").await.unwrap();
    assert_eq!(deleted, 2, "destroy_for_user must return the row count");

    // Both alice rows gone, bob untouched.
    assert!(
        driver.read("alice-sess-1").await.unwrap().is_none(),
        "alice's first session must be revoked"
    );
    assert!(
        driver.read("alice-sess-2").await.unwrap().is_none(),
        "alice's second session must be revoked"
    );
    assert!(
        driver.read("bob-sess-1").await.unwrap().is_some(),
        "bob's session must not be touched when revoking alice"
    );
}

/// P4-02: a user signed in only through a non-default guard has a row
/// with `user_id = NULL`. User-wide revocation must still find the row
/// through the payload's guard identities and delete it, so the same
/// cookie cannot restore the named identity on replay.
#[tokio::test]
async fn destroy_for_user_revokes_named_only_guard_session() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let mut sess = SessionData::new("named-only-sess".into(), "csrf".into());
    assert!(sess.user_id.is_none());
    sess.data.insert(
        "_auth_guards".to_string(),
        serde_json::json!({ "admin": { "id": "admin-uid" } }),
    );
    driver.write(&sess).await.unwrap();
    assert!(
        driver.read("named-only-sess").await.unwrap().is_some(),
        "precondition: the named-only session must exist"
    );

    let deleted = driver.destroy_for_user("admin-uid").await.unwrap();
    assert_eq!(deleted, 1, "revocation must reach the named-only row");

    assert!(
        driver.read("named-only-sess").await.unwrap().is_none(),
        "the revoked row must be gone, so the cookie cannot replay the identity"
    );
}

/// P4-02: a session carrying several principals under different guards
/// must die when any one of them is revoked - the surviving row would
/// otherwise keep authenticating the revoked principal.
#[tokio::test]
async fn destroy_for_user_revokes_multi_principal_session_for_any_principal() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let mut sess = SessionData::new("multi-principal-sess".into(), "csrf".into());
    sess.user_id = Some("web-uid".into());
    sess.data.insert(
        "_auth_guards".to_string(),
        serde_json::json!({ "admin": { "id": "admin-uid" } }),
    );
    driver.write(&sess).await.unwrap();

    let deleted = driver.destroy_for_user("admin-uid").await.unwrap();
    assert_eq!(deleted, 1);
    assert!(
        driver.read("multi-principal-sess").await.unwrap().is_none(),
        "revoking any principal must remove the whole session row"
    );

    // The other principal's revocation now finds nothing left to do.
    let deleted = driver.destroy_for_user("web-uid").await.unwrap();
    assert_eq!(deleted, 0);
}

/// P4-02 negative: revoking one user must not touch sessions whose guard
/// identities belong to someone else.
#[tokio::test]
async fn destroy_for_user_leaves_other_guard_identities_alone() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let mut sess = SessionData::new("other-admin-sess".into(), "csrf".into());
    sess.data.insert(
        "_auth_guards".to_string(),
        serde_json::json!({ "admin": { "id": "other-admin-uid" } }),
    );
    driver.write(&sess).await.unwrap();

    let deleted = driver.destroy_for_user("admin-uid").await.unwrap();
    assert_eq!(deleted, 0);
    assert!(
        driver.read("other-admin-sess").await.unwrap().is_some(),
        "another user's named-guard session must survive"
    );
}

/// P4-09: a programmatically built driver with an absurd lifetime must
/// neither panic in deadline arithmetic nor mass-expire live sessions.
/// The `u64`->`i64` conversion caps, the deadline addition is checked,
/// and the GC threshold floors instead of deleting everything.
#[tokio::test]
async fn oversized_programmatic_lifetime_neither_panics_nor_mass_expires() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(u64::MAX));

    let mut sess = SessionData::new("huge-lifetime-sess".into(), "csrf".into());
    sess.user_id = Some("u-huge".into());
    driver.write(&sess).await.unwrap();

    assert!(
        driver
            .read("huge-lifetime-sess")
            .await
            .expect("read must not panic on an oversized lifetime")
            .is_some(),
        "a fresh session under a capped lifetime must read back as live"
    );
    assert_eq!(
        driver.gc().await.expect("gc must not panic"),
        0,
        "gc under an oversized lifetime must delete nothing, not mass-expire"
    );
    assert!(
        driver
            .read("huge-lifetime-sess")
            .await
            .expect("read after gc")
            .is_some(),
        "the session must survive the gc pass"
    );
}

#[tokio::test]
async fn destroy_for_user_returns_zero_when_no_matching_rows() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));
    let deleted = driver.destroy_for_user("ghost-uid").await.unwrap();
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn module_helper_destroy_all_for_user_delegates_to_driver() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let mut sess = SessionData::new("helper-sess".into(), "csrf".into());
    sess.user_id = Some("helper-uid".into());
    driver.write(&sess).await.unwrap();

    let deleted = suprnova::session::destroy_all_for_user("helper-uid")
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    assert!(driver.read("helper-sess").await.unwrap().is_none());
}

// ── SEC-02(c): a write for a session read as existing must not
// resurrect a row deleted out from under it (e.g. by a concurrent
// revocation) ───────────────────────────────────────────────────────

/// Primary SEC-02(c) regression test. Mirrors the exact race the finding
/// describes: a request reads an existing session (marking it
/// `loaded_from_store`), a concurrent revocation deletes the row, then
/// the original request's end-of-handler write must NOT recreate it.
#[tokio::test]
async fn write_does_not_resurrect_a_concurrently_deleted_row() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    // The victim's session already exists in the store.
    let mut sess = SessionData::new("resurrection-sess".into(), "csrf".into());
    sess.user_id = Some("victim-uid".into());
    driver.write(&sess).await.unwrap();

    // A request reads it - this is the point `loaded_from_store` gets
    // set, proving the row existed when THIS request observed it.
    let mut reloaded = driver
        .read("resurrection-sess")
        .await
        .unwrap()
        .expect("row must exist");
    assert!(
        reloaded.loaded_from_store,
        "read() must mark a row it actually found as loaded_from_store"
    );

    // Concurrently, a security-team forced reset (or password-reset
    // completion) revokes every session for this user - deleting the
    // row out from under the in-flight request above.
    let revoked = driver.destroy_for_user("victim-uid").await.unwrap();
    assert_eq!(revoked, 1);
    assert!(
        driver.read("resurrection-sess").await.unwrap().is_none(),
        "precondition: the row must actually be gone after revocation"
    );

    // The original request's handler mutates the (now stale, in-memory)
    // session and the middleware persists it at the end of the request -
    // exactly the sequence at session/middleware.rs's end-of-handle
    // persistence step.
    reloaded.put("touched", "yes");
    driver.write(&reloaded).await.unwrap();

    // SEC-02(c): the write must NOT have resurrected the row. Before the
    // fix, the unconditional `INSERT ... ON CONFLICT DO UPDATE` would
    // have recreated it here, carrying `user_id` (and every other
    // field) right back - undoing the revocation this same test just
    // performed.
    assert!(
        driver.read("resurrection-sess").await.unwrap().is_none(),
        "a write for a session read as existing must not resurrect a row \
         deleted out from under it by a concurrent revocation"
    );
}

/// Regression guard for the fix itself: an id-rotation (login, 2FA
/// promotion, remember-me hydration, manual regenerate,
/// `invalidate_session`) must still be able to CREATE its new row. If
/// `SessionData::rotate_id` failed to clear `loaded_from_store`, the
/// write-after-rotate would incorrectly take the update-only branch
/// against an id that was never persisted and silently drop the
/// regenerated session instead of creating it.
#[tokio::test]
async fn write_after_rotate_id_creates_the_new_row() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let mut sess = SessionData::new("old-id-session".into(), "csrf".into());
    sess.user_id = Some("rotator-uid".into());
    driver.write(&sess).await.unwrap();

    let mut reloaded = driver
        .read("old-id-session")
        .await
        .unwrap()
        .expect("row must exist");
    assert!(reloaded.loaded_from_store);

    reloaded.rotate_id("new-id-session");
    assert!(
        !reloaded.loaded_from_store,
        "rotate_id must clear loaded_from_store so the new id's row can be created"
    );

    driver.write(&reloaded).await.unwrap();

    assert!(
        driver.read("new-id-session").await.unwrap().is_some(),
        "write after rotate_id must CREATE the new row, not silently \
         no-op via the update-only branch"
    );
}

#[tokio::test]
async fn atomic_migration_replaces_the_old_row_with_one_authenticated_row() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let pending = SessionData::new("pending-two-factor".into(), "pending-csrf".into());
    driver.write(&pending).await.unwrap();

    let mut authenticated = pending.clone();
    authenticated.rotate_id("authenticated-two-factor");
    authenticated.user_id = Some("promoted-user".into());
    driver
        .migrate_two_factor_session("pending-two-factor", &authenticated)
        .await
        .expect("database driver must atomically migrate a 2FA session");

    assert!(driver.read("pending-two-factor").await.unwrap().is_none());
    let stored = driver
        .read("authenticated-two-factor")
        .await
        .unwrap()
        .expect("authenticated replacement must exist");
    assert_eq!(stored.user_id.as_deref(), Some("promoted-user"));
}

#[tokio::test]
async fn atomic_migration_insert_failure_rolls_back_the_old_row() {
    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let pending = SessionData::new("pending-two-factor".into(), "pending-csrf".into());
    driver.write(&pending).await.unwrap();
    let collision = SessionData::new("occupied-new-id".into(), "other-csrf".into());
    driver.write(&collision).await.unwrap();

    let mut authenticated = pending.clone();
    authenticated.rotate_id("occupied-new-id");
    authenticated.user_id = Some("promoted-user".into());
    let error = driver
        .migrate_two_factor_session("pending-two-factor", &authenticated)
        .await
        .expect_err("replacement insert collision must fail the whole migration");
    assert!(matches!(error, SessionMigrationError::RolledBack(_)));

    assert!(
        driver.read("pending-two-factor").await.unwrap().is_some(),
        "failed replacement insert must roll back deletion of the pending row"
    );
    let occupied = driver
        .read("occupied-new-id")
        .await
        .unwrap()
        .expect("pre-existing replacement id must remain untouched");
    assert!(occupied.user_id.is_none());
}

#[tokio::test]
async fn concurrent_atomic_migrations_from_one_pending_row_elect_one_winner() {
    let (guard, database, database_path) = multi_connection_test_database().await;
    let driver = DatabaseSessionDriver::new(Duration::from_secs(3600));

    let pending = SessionData::new("shared-pending-two-factor".into(), "pending-csrf".into());
    driver.write(&pending).await.unwrap();
    let mut first = pending.clone();
    first.rotate_id("first-authenticated-session");
    first.user_id = Some("promoted-user".into());
    let mut second = pending;
    second.rotate_id("second-authenticated-session");
    second.user_id = Some("promoted-user".into());

    // Hold SQLite's write lock before either real driver call reaches DELETE.
    // Both migrations can BEGIN on distinct pooled connections, but their
    // deletes remain blocked. Observing all three pool connections in use is
    // the deterministic overlap witness; only then do we release the writer.
    let blocker = database.inner().begin().await.unwrap();
    blocker
        .execute_unprepared(
            "UPDATE sessions SET csrf_token = csrf_token WHERE id = 'shared-pending-two-factor'",
        )
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let pool = database.inner().get_sqlite_connection_pool().clone();
    let (first_result, second_result, ()) = tokio::join!(
        async {
            barrier.wait().await;
            driver
                .migrate_two_factor_session("shared-pending-two-factor", &first)
                .await
        },
        async {
            barrier.wait().await;
            driver
                .migrate_two_factor_session("shared-pending-two-factor", &second)
                .await
        },
        async {
            barrier.wait().await;
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if pool.size() as usize - pool.num_idle() == 3 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("both migrations must hold distinct transaction connections");
            blocker.commit().await.unwrap();
        },
    );
    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1,
        "exactly one promotion may consume the pending session row"
    );
    assert!(
        driver
            .read("shared-pending-two-factor")
            .await
            .unwrap()
            .is_none()
    );
    let reachable = usize::from(
        driver
            .read("first-authenticated-session")
            .await
            .unwrap()
            .is_some(),
    ) + usize::from(
        driver
            .read("second-authenticated-session")
            .await
            .unwrap()
            .is_some(),
    );
    assert_eq!(reachable, 1, "only the winning auth row may exist");

    drop(guard);
    database.inner().clone().close().await.unwrap();
    std::fs::remove_file(database_path).expect("remove isolated SQLite test database");
}
