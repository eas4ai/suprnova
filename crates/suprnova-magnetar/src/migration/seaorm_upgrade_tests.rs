use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;

use crate::migration::fingerprint::source_database_fingerprints;
use crate::{Error, Result, default_schema};
use futures_util::future::FutureExt;
use sea_orm::DatabaseConnection;

#[cfg(test)]
#[path = "../../tests/fixtures/seaorm_upgrade.rs"]
mod seaorm_upgrade_fixture;

#[cfg(test)]
use seaorm_upgrade_fixture::{SeaOrm11Fixture, import_fixture};

const EXPECTED_SOURCE_TABLES: [&str; 14] = [
    "app_users",
    "auth_ceremonies",
    "auth_lifecycle_deliveries",
    "auth_linked_accounts",
    "auth_lockouts",
    "auth_methods",
    "auth_migration_identities",
    "auth_migration_runs",
    "auth_provider_tokens",
    "auth_remember_tokens",
    "auth_sessions",
    "auth_tokens",
    "auth_two_factor",
    "magnetar_migration_state",
];

async fn schema_digests(database: &DatabaseConnection) -> Result<BTreeMap<String, String>> {
    Ok(
        source_database_fingerprints(database, &[], &BTreeMap::new())
            .await?
            .into_iter()
            .map(|fingerprint| (fingerprint.table, fingerprint.schema_digest))
            .collect(),
    )
}

#[cfg(test)]
fn assert_expected_source_tables(before: &BTreeMap<String, String>) {
    let expected: BTreeSet<&str> = EXPECTED_SOURCE_TABLES.iter().copied().collect();
    let actual: BTreeSet<&str> = before.keys().map(String::as_str).collect();
    assert_eq!(
        expected, actual,
        "seaorm 1.1 source catalogs must contain exactly the documented tables for compatibility"
    );
}

#[cfg(test)]
async fn verify_parity(fixture: SeaOrm11Fixture) {
    let imported = import_fixture(fixture)
        .await
        .expect("SeaORM 1.1 fixture import should succeed");

    let run = AssertUnwindSafe(async {
        let before = schema_digests(&imported.connection).await?;
        assert_expected_source_tables(&before);

        default_schema::migrate(&imported.connection)
            .await
            .map_err(|error| Error::Internal {
                message: format!("first migration replay pass failed: {error}"),
            })?;

        let after = schema_digests(&imported.connection).await?;
        assert_eq!(before, after, "first migration replay should be no-op");

        default_schema::migrate(&imported.connection)
            .await
            .map_err(|error| Error::Internal {
                message: format!("second migration replay pass failed: {error}"),
            })?;

        let replay = schema_digests(&imported.connection).await?;
        assert_eq!(after, replay, "second migration replay should be no-op");
        Ok::<_, Error>(())
    })
    .catch_unwind()
    .await;

    imported
        .cleanup()
        .await
        .expect("fixture cleanup must run after source catalog replay assertions");

    match run {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!(
            "SeaORM 1.1 migration catalog should remain stable across replay passes: {error}"
        ),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(all(test, feature = "seaorm-sqlite"))]
#[tokio::test]
async fn sqlite_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::Sqlite).await;
}

#[cfg(all(test, feature = "seaorm-postgres"))]
#[ignore = "requires T2 live Postgres/MySQL database"]
#[tokio::test]
async fn postgres_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::Postgres).await;
}

#[cfg(all(test, feature = "seaorm-mysql"))]
#[ignore = "requires T2 live Postgres/MySQL database"]
#[tokio::test]
async fn mysql_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::MySql).await;
}
