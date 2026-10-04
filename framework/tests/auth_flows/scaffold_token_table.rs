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
use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, DeriveIden};
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
    /// The table older builders created: every time column `.timestamp()`.
    Legacy,
}

/// `auth_flow_tokens` with a `VARCHAR(64)` hash and the given time columns.
///
/// MySQL 8.4 refuses the builder's UNIQUE key on a `TEXT` hash (error
/// 1170, a separate defect), so the legacy shape, and the current one on
/// MySQL 8.4, use this copy. Only the hash column differs.
fn varchar_hash_table(time: fn(&mut ColumnDef) -> &mut ColumnDef) -> TableCreateStatement {
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
                .string_len(64)
                .not_null()
                .unique_key(),
        )
        .col(ColumnDef::new(AuthFlowTokens::Purpose).text().not_null())
        .col(time(&mut ColumnDef::new(AuthFlowTokens::ExpiresAt)).not_null())
        .col(time(&mut ColumnDef::new(AuthFlowTokens::UsedAt)).null())
        .col(time(&mut ColumnDef::new(AuthFlowTokens::CreatedAt)).not_null())
        .to_owned()
}

async fn create_table(database: &DbConnection, shape: Shape) {
    let manager = SchemaManager::new(database.inner());
    let created: Result<(), DbErr> = match shape {
        Shape::Current => scaffold_auth_flow_tokens::Migration.up(&manager).await,
        Shape::Legacy => {
            manager
                .create_table(varchar_hash_table(ColumnDef::timestamp))
                .await
        }
    };
    match created {
        Ok(()) => {}
        Err(error)
            if database.inner().get_database_backend() == DatabaseBackend::MySql
                && error.to_string().contains("1170") =>
        {
            database
                .inner()
                .execute_unprepared("DROP TABLE IF EXISTS auth_flow_tokens")
                .await
                .expect("drop partial table");
            manager
                .create_table(varchar_hash_table(ColumnDef::date_time))
                .await
                .expect("create auth_flow_tokens with a VARCHAR hash");
        }
        Err(error) => panic!("{shape:?} auth_flow_tokens: {error}"),
    }
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
    issue_check_consume_and_prune(&url, Shape::Legacy).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_scaffold_token_table_issues_checks_and_consumes() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    issue_check_consume_and_prune(&url, Shape::Current).await;
    issue_check_consume_and_prune(&url, Shape::Legacy).await;
}
