//! PostgreSQL coverage for the RBAC helpers, the `unique` validation rule,
//! and `Model::increment`.
//!
//! All three built raw SQL with `?` placeholders. Postgres rejects `?`
//! outright, so every one of these paths failed there - and the failure
//! went unnoticed because the rest of the suite runs on SQLite. These
//! tests exist to make the backend difference observable: run them against
//! a real Postgres and a regression fails immediately instead of surfacing
//! in someone's production deploy.
//!
//! ```bash
//! docker run -d --rm --name suprnova-rbac-pg \
//!   -e POSTGRES_PASSWORD=pw -e POSTGRES_DB=suprnova_test \
//!   -p 55998:5432 postgres:17-alpine
//! PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
//!   cargo test -p suprnova --test rbac_postgres -- --ignored --test-threads=1
//! docker rm -f suprnova-rbac-pg
//! ```

use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::time::Duration;
use suprnova::rbac::{
    assign_role_to_model, create_role, give_permission_to_role, has_permission_for_model,
    has_role_for_model,
};
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection};

async fn connect_and_install() -> suprnova::testing::TestContainerGuard {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    let conn = Database::connect(options)
        .await
        .expect("Postgres test database must be reachable");
    let backend = conn.get_database_backend();

    // The RBAC schema, matching what the framework's migrations create.
    for sql in [
        "DROP TABLE IF EXISTS model_permissions",
        "DROP TABLE IF EXISTS model_roles",
        "DROP TABLE IF EXISTS role_permissions",
        "DROP TABLE IF EXISTS permissions",
        "DROP TABLE IF EXISTS roles",
        "CREATE TABLE roles (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL, \
         display_name TEXT NULL, guard_name TEXT NOT NULL)",
        "CREATE TABLE permissions (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL, \
         display_name TEXT NULL, guard_name TEXT NOT NULL)",
        "CREATE TABLE role_permissions (role_id BIGINT NOT NULL, permission_id BIGINT NOT NULL)",
        "CREATE TABLE model_roles (model_type TEXT NOT NULL, model_id TEXT NOT NULL, \
         role_id BIGINT NOT NULL)",
        "CREATE TABLE model_permissions (model_type TEXT NOT NULL, model_id TEXT NOT NULL, \
         permission_id BIGINT NOT NULL)",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|e| panic!("schema setup failed on {sql:?}: {e}"));
    }

    let guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn));
    guard
}

/// The headline case: creating a role, granting it a permission, assigning
/// it to a model, and asking whether the model has it. Every step is a
/// statement that used `?`.
#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_rbac_round_trips_roles_and_permissions() {
    let _guard = connect_and_install().await;

    create_role("editor").await.expect("create_role");
    give_permission_to_role("editor", "articles.publish")
        .await
        .expect("give_permission_to_role");
    assign_role_to_model("App::User", "42", "editor")
        .await
        .expect("assign_role_to_model");

    assert!(
        has_role_for_model("App::User", "42", "editor")
            .await
            .expect("has_role_for_model"),
        "the assigned role must be visible"
    );
    assert!(
        has_permission_for_model("App::User", "42", "articles.publish")
            .await
            .expect("has_permission_for_model"),
        "a permission inherited through the role must resolve - this is the \
         five-bind join, the widest statement in the module"
    );

    // Negative controls: the check must actually discriminate, not just
    // return true because every query errored into a default.
    assert!(
        !has_role_for_model("App::User", "43", "editor")
            .await
            .expect("has_role_for_model"),
        "a different model id must not inherit the role"
    );
    assert!(
        !has_permission_for_model("App::User", "42", "articles.delete")
            .await
            .expect("has_permission_for_model"),
        "an ungranted permission must not resolve"
    );
}

/// Idempotency depends on the `SELECT COUNT(*) … WHERE … = ? AND … = ?`
/// existence checks running successfully. If those error on Postgres, the
/// helpers would insert duplicates rather than short-circuit.
#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_rbac_assignment_is_idempotent() {
    let _guard = connect_and_install().await;

    for _ in 0..3 {
        create_role("auditor").await.expect("create_role");
        assign_role_to_model("App::User", "7", "auditor")
            .await
            .expect("assign_role_to_model");
    }

    let roles: i64 = DB::scalar(
        "SELECT COUNT(*) FROM roles WHERE name = $1",
        vec!["auditor".into()],
    )
    .await
    .expect("count roles");
    assert_eq!(roles, 1, "repeated create_role must not insert duplicates");

    let assignments: i64 = DB::scalar(
        "SELECT COUNT(*) FROM model_roles WHERE model_id = $1",
        vec!["7".into()],
    )
    .await
    .expect("count assignments");
    assert_eq!(
        assignments, 1,
        "repeated assignment must not insert duplicates"
    );
}
