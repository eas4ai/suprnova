//! Email-verification and password-reset tokens work on the
//! `auth_flow_tokens` table a scaffolded app creates, on every engine, in
//! today's shape and in the one older scaffolds created.
//!
//! Older scaffolds created `expires_at`, `used_at` and `created_at` with
//! `.timestamp()`: `TIMESTAMP` on MySQL and MariaDB. `TokenStore` read whole
//! rows into an entity with `NaiveDateTime` fields, which the MySQL driver
//! decodes only from `DATETIME`, so `check`, `owner` and `consume` failed
//! there and no verification or reset link could be used. The table builder
//! now creates them with `.date_time()`, `DATETIME` on MySQL; tables an
//! older migration created keep `TIMESTAMP`.
//!
//! The builder's `TEXT` hash with a UNIQUE key failed on MySQL 8.4 (error
//! 1170), so a scaffolded app stopped at its fourth migration there. The
//! hash is now `VARCHAR(64)`, the length of a SHA-256 hex digest.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth_flows -- --ignored scaffold_token_table::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test auth_flows -- --ignored scaffold_token_table::postgres_
//! ```

// `#[rustfmt::skip]`: the template is scaffold output, not workspace
// source, so `cargo fmt` must not rewrite it.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_auth_flow_tokens_table.rs.tpl"]
mod scaffold_auth_flow_tokens;

use sea_orm::sea_query::{ColumnDef, Table, TableCreateStatement};
use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, DeriveIden, EntityTrait, Statement};
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::auth_flows::token_store::{TokenPurpose, TokenStore};
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::TestContainer;

async fn connect(url: &str) -> DbConnection {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    DbConnection::connect(&config)
        .await
        .expect("connect test database")
}

#[derive(DeriveIden)]
enum AuthFlowTokens {
    Table,
    Id,
    UserId,
    TokenHash,
    Purpose,
    ExpiresAt,
    UsedAt,
    CreatedAt,
}

/// Which `auth_flow_tokens` a test creates.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// The scaffold's migration, which applies the framework's builder.
    Current,
    /// The table older builders created: a `TEXT` hash and every time
    /// column `.timestamp()`.
    Legacy,
}

/// The table older builders created, column for column.
fn legacy_table() -> TableCreateStatement {
    Table::create()
        .table(AuthFlowTokens::Table)
        .col(
            ColumnDef::new(AuthFlowTokens::Id)
                .big_integer()
                .not_null()
                .auto_increment()
                .primary_key(),
        )
        .col(ColumnDef::new(AuthFlowTokens::UserId).text().not_null())
        .col(
            ColumnDef::new(AuthFlowTokens::TokenHash)
                .text()
                .not_null()
                .unique_key(),
        )
        .col(ColumnDef::new(AuthFlowTokens::Purpose).text().not_null())
        .col(
            ColumnDef::new(AuthFlowTokens::ExpiresAt)
                .timestamp()
                .not_null(),
        )
        .col(ColumnDef::new(AuthFlowTokens::UsedAt).timestamp().null())
        .col(
            ColumnDef::new(AuthFlowTokens::CreatedAt)
                .timestamp()
                .not_null(),
        )
        .to_owned()
}

async fn create_table(database: &DbConnection, shape: Shape) {
    let manager = SchemaManager::new(database.inner());
    let created: Result<(), DbErr> = match shape {
        Shape::Current => scaffold_auth_flow_tokens::Migration.up(&manager).await,
        Shape::Legacy => manager.create_table(legacy_table()).await,
    };
    if let Err(error) = created {
        panic!("{shape:?} auth_flow_tokens: {error}");
    }
}

/// Whether `database` is MySQL rather than MariaDB. MySQL refuses the
/// legacy table's UNIQUE key on a `TEXT` column (error 1170), so no older
/// migration ever created that table there.
async fn is_mysql_proper(database: &DbConnection) -> bool {
    if database.inner().get_database_backend() != DatabaseBackend::MySql {
        return false;
    }
    let row = database
        .inner()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::MySql,
            "SELECT VERSION() AS version",
        ))
        .await
        .expect("read the server version")
        .expect("a version row");
    let version: String = row.try_get("", "version").expect("version text");
    !version.contains("MariaDB")
}

async fn issue_check_consume_and_prune(url: &str, shape: Shape) {
    let database = connect(url).await;
    database
        .inner()
        .execute_unprepared("DROP TABLE IF EXISTS auth_flow_tokens")
        .await
        .expect("drop auth_flow_tokens");
    create_table(&database, shape).await;
    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());

    let verification = TokenPurpose::EmailVerification;
    let token = TokenStore::issue("7", verification, chrono::Duration::minutes(60))
        .await
        .expect("issue a verification token");
    let reset = TokenStore::issue(
        "7",
        TokenPurpose::PasswordReset,
        chrono::Duration::minutes(60),
    )
    .await
    .expect("issue a reset token");

    let check = TokenStore::check(&token, verification).await;
    let owner = TokenStore::owner(&token, verification).await;
    let consumed = TokenStore::consume(&token, verification).await;
    let check_after = TokenStore::check(&token, verification).await;
    let reset_consumed = TokenStore::consume(&reset, TokenPurpose::PasswordReset).await;
    // One assertion over every call, so a failure shows each of them.
    assert_eq!(
        format!("{check:?} {owner:?} {consumed:?} {check_after:?} {reset_consumed:?}"),
        r#"Ok(true) Ok(Some("7")) Ok(Some("7")) Ok(false) Ok(Some("7"))"#,
        "{shape:?}: check, owner, consume, check after consume, consume a reset token"
    );

    // An expiry past 2038-01-19, the end of MySQL's `TIMESTAMP` range,
    // still issues a usable token.
    let long_lived = TokenStore::issue("8", verification, chrono::Duration::days(20 * 365))
        .await
        .expect("issue a token that outlives 2038");
    assert!(
        TokenStore::check(&long_lived, verification)
            .await
            .expect("check the long-lived token")
    );

    // Whole rows read through the public entity, whatever type the
    // migration gave the time columns.
    let rows = suprnova::auth_flows::token_store::entity::Entity::find()
        .all(database.inner())
        .await
        .map(|rows| rows.len());
    assert_eq!(
        format!("{rows:?}"),
        "Ok(3)",
        "{shape:?}: whole rows through token_store::entity::Entity"
    );

    // An already-expired token is pruned; the live ones stay.
    TokenStore::issue("9", verification, chrono::Duration::minutes(-5))
        .await
        .expect("issue an expired token");
    assert_eq!(TokenStore::prune_expired().await.expect("prune"), 1);

    database
        .inner()
        .execute_unprepared("DROP TABLE auth_flow_tokens")
        .await
        .expect("drop auth_flow_tokens");
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_scaffold_token_table_issues_checks_and_consumes() {
    issue_check_consume_and_prune("sqlite::memory:", Shape::Current).await;
    issue_check_consume_and_prune("sqlite::memory:", Shape::Legacy).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_scaffold_token_table_issues_checks_and_consumes() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    issue_check_consume_and_prune(&url, Shape::Current).await;
    // MySQL never had a legacy table: the older builder's migration failed
    // there with error 1170, so only MariaDB carries one.
    if !is_mysql_proper(&connect(&url).await).await {
        issue_check_consume_and_prune(&url, Shape::Legacy).await;
    }
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_scaffold_token_table_issues_checks_and_consumes() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    issue_check_consume_and_prune(&url, Shape::Current).await;
    issue_check_consume_and_prune(&url, Shape::Legacy).await;
}
