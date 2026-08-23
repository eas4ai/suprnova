use std::collections::BTreeMap;

use crate::migration::fingerprint::source_database_fingerprints;
use crate::{default_schema, Error, Result};
use sea_orm::DatabaseConnection;

#[cfg(test)]
#[path = "../../tests/fixtures/seaorm_upgrade.rs"]
mod seaorm_upgrade_fixture;

#[cfg(test)]
use seaorm_upgrade_fixture::{import_fixture, SeaOrm11Fixture};

async fn schema_digests(
    database: &DatabaseConnection,
) -> Result<BTreeMap<String, String>> {
    Ok(source_database_fingerprints(database, &[], &BTreeMap::new())
        .await?
        .into_iter()
        .map(|fingerprint| (fingerprint.table, fingerprint.schema_digest))
        .collect())
}

#[cfg(test)]
async fn verify_parity(fixture: SeaOrm11Fixture) {
    let imported = import_fixture(fixture).await;
    let result = async {
        let before = schema_digests(&imported.connection).await?;

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
    }
    .await;
    imported.cleanup().await;
    result.expect("SeaORM 1.1 migration catalog should remain stable across replay passes");
}

#[cfg(all(test, feature = "seaorm-sqlite"))]
#[tokio::test]
async fn sqlite_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::Sqlite).await;
}

#[cfg(all(test, feature = "seaorm-postgres"))]
#[tokio::test]
async fn postgres_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::Postgres).await;
}

#[cfg(all(test, feature = "seaorm-mysql"))]
#[tokio::test]
async fn mysql_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1() {
    verify_parity(SeaOrm11Fixture::MySql).await;
}


