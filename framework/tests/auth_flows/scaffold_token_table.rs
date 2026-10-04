//! Email-verification and password-reset tokens work on the
//! `auth_flow_tokens` table a scaffolded app creates, on every engine.
//!
//! The table's `expires_at`, `used_at` and `created_at` are `.timestamp()`:
//! `TIMESTAMP` on MySQL and MariaDB. `TokenStore` once read whole rows into
//! an entity with `NaiveDateTime` fields, which the MySQL driver decodes
//! only from `DATETIME`, so `check`, `owner` and `consume` failed there and
//! no verification or reset link could be used.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth_flows -- --ignored scaffold_token_table::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test auth_flows -- --ignored scaffold_token_table::postgres_
//! ```

// `include!` rather than `#[path]`: rustfmt follows `#[path]` modules and
// would reformat the template, which is scaffold output, not workspace
// source.
mod scaffold_auth_flow_tokens {
    include!(
        "../../../suprnova-cli/src/templates/files/backend/migrations/create_auth_flow_tokens_table.rs.tpl"
    );
}

use sea_orm::sea_query::{ColumnDef, Table};
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

/// Create `auth_flow_tokens` with the scaffold's migration.
///
/// MySQL 8.4 refuses that migration's UNIQUE key on a `TEXT` column (error
/// 1170, a separate defect), so there the same table is created with a
/// `VARCHAR(64)` hash. Its time columns are the migration's `.timestamp()`.
async fn create_table(database: &DbConnection) {
    let manager = SchemaManager::new(database.inner());
    let created: Result<(), DbErr> = scaffold_auth_flow_tokens::Migration.up(&manager).await;
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
            let table = Table::create()
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
                .to_owned();
            manager
                .create_table(table)
                .await
                .expect("create auth_flow_tokens with a VARCHAR hash");
        }
        Err(error) => panic!("scaffold auth_flow_tokens migration: {error}"),
    }
}

async fn issue_check_consume_and_prune(url: &str) {
    let database = connect(url).await;
    database
        .inner()
        .execute_unprepared("DROP TABLE IF EXISTS auth_flow_tokens")
        .await
        .expect("drop auth_flow_tokens");
    create_table(&database).await;
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
        "check, owner, consume, check after consume, consume a reset token"
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
    issue_check_consume_and_prune("sqlite::memory:").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_scaffold_token_table_issues_checks_and_consumes() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    issue_check_consume_and_prune(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_scaffold_token_table_issues_checks_and_consumes() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    issue_check_consume_and_prune(&url).await;
}
