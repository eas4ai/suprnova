#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::{AuthMethod, MethodStore, SeaOrmStorage};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use storage_schema::{StorageSchema, database, methods, users};

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn remove_last_method_is_rejected_by_census() {
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
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    assert!(
        !store
            .remove_method_if_not_last("1", AuthMethod::Passkey("1".into()), 1)
            .await
            .unwrap()
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn removing_one_of_two_methods_advances_epoch() {
    let db = database().await;
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
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    assert!(
        store
            .remove_method_if_not_last("1", AuthMethod::Passkey("1".into()), 3)
            .await
            .unwrap()
    );
    assert_eq!(
        methods::Entity::find_by_id(2)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .user_id,
        1
    );
    assert_eq!(
        users::Entity::find_by_id(1)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .auth_epoch,
        1
    );
}
