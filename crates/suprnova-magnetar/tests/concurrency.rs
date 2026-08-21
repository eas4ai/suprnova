#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::{AuthMethod, MethodStore, SeaOrmStorage};
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
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let (left, right) = tokio::join!(
        store.remove_method_if_not_last("1", AuthMethod::Passkey("1".into()), 2),
        store.remove_method_if_not_last("1", AuthMethod::Passkey("2".into()), 2),
    );
    assert_eq!(usize::from(left.unwrap()) + usize::from(right.unwrap()), 1);
}
