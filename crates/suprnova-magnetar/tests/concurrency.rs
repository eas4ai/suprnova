#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::{MethodStore, SeaOrmStorage};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use storage_schema::{StorageSchema, database, methods, users};

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn concurrent_remove_last_method_has_single_winner() {
    let db = database().await;
    users::ActiveModel {
        id: Set(1),
        password_hash: Set(None),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    methods::ActiveModel {
        id: Set(1),
        user_id: Set(1),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    methods::ActiveModel {
        id: Set(2),
        user_id: Set(1),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
    let actor = storage_schema::credential_actor(&db, "1", 0, "concurrent-removal-session").await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let (left, right) = tokio::join!(
        store.remove_passkey_if_not_last(&actor, "1"),
        store.remove_passkey_if_not_last(&actor, "2"),
    );
    let outcomes = [left, right];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(true)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Err(magnetar::Error::NotFound { resource, identifier })
                    if resource == "credential actor" && identifier == "expired or revoked"
            ))
            .count(),
        1
    );
}
