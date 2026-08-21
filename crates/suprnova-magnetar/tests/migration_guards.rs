#![cfg(all(feature = "migration", feature = "seaorm-sqlite"))]

use std::collections::BTreeMap;

use magnetar::Error;
use magnetar::migration::fingerprint::TableFingerprint;
use magnetar::migration::schema_guards::{create_index_if_missing, has_column, has_index};
use magnetar::migration::{BackendStrategy, MigrationBackend};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

#[tokio::test]
async fn schema_guards_replay_without_backend_if_not_exists_shortcuts() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    database
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE app_external_identities (id INTEGER PRIMARY KEY, provider TEXT NOT NULL, external_user_id TEXT NOT NULL, app_user_id INTEGER NOT NULL)",
        ))
        .await
        .unwrap();

    assert!(
        has_column(&database, None, "app_external_identities", "provider")
            .await
            .unwrap()
    );
    assert!(
        !has_column(&database, None, "app_external_identities", "missing")
            .await
            .unwrap()
    );
    assert!(
        !has_index(
            &database,
            None,
            "app_external_identities",
            "magnetar_external_identity_lookup"
        )
        .await
        .unwrap()
    );

    let first = create_index_if_missing(
        &database,
        None,
        "app_external_identities",
        "magnetar_external_identity_lookup",
        &["provider", "external_user_id"],
    )
    .await
    .unwrap();
    let second = create_index_if_missing(
        &database,
        None,
        "app_external_identities",
        "magnetar_external_identity_lookup",
        &["provider", "external_user_id"],
    )
    .await
    .unwrap();
    assert_eq!(first.statements, 1);
    assert_eq!(second.statements, 0);
    assert!(
        has_index(
            &database,
            None,
            "app_external_identities",
            "magnetar_external_identity_lookup"
        )
        .await
        .unwrap()
    );
}

#[test]
fn backend_strategy_declares_mysql_shadow_copy_and_fingerprint_posture() {
    assert_eq!(
        BackendStrategy::for_backend(DbBackend::MySql),
        BackendStrategy::MySqlShadowSwap {
            copy: "shadow-copy",
            verification: "table-fingerprint",
            cutover: "rename-journal",
            recovery: "reverse-rename-restore",
        }
    );
    assert_eq!(
        BackendStrategy::for_backend(DbBackend::Postgres),
        BackendStrategy::Transactional {
            backend: MigrationBackend::Postgres,
        }
    );
}

#[test]
fn fingerprints_detect_row_count_and_field_drift() {
    let baseline = TableFingerprint::from_rows(
        &["id", "email"],
        vec![row(&[("id", "1"), ("email", "one@example.test")])],
    )
    .unwrap();
    let additional_row = TableFingerprint::from_rows(
        &["id", "email"],
        vec![
            row(&[("id", "1"), ("email", "one@example.test")]),
            row(&[("id", "2"), ("email", "two@example.test")]),
        ],
    )
    .unwrap();
    let changed_field = TableFingerprint::from_rows(
        &["id", "email"],
        vec![row(&[("id", "1"), ("email", "changed@example.test")])],
    )
    .unwrap();

    assert_ne!(baseline, additional_row);
    assert_ne!(baseline, changed_field);
    let field_error =
        TableFingerprint::from_rows(&["id", "email"], vec![row(&[("id", "1")])]).unwrap_err();
    assert!(matches!(field_error, Error::InvalidInput { .. }));
}

fn row(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}
