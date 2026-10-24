#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::{CredentialActor, MethodStore, SeaOrmStorage};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
use storage_schema::{StorageSchema, accounts, database, methods, users};

async fn credential_actor(database: &DatabaseConnection) -> CredentialActor {
    storage_schema::credential_actor(database, "1", 0, "method-store-session").await
}

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
    let actor = credential_actor(&db).await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    assert!(!store.remove_passkey_if_not_last(&actor, "1").await.unwrap());
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
    let actor = credential_actor(&db).await;
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    assert!(store.remove_passkey_if_not_last(&actor, "1").await.unwrap());
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

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn stale_actor_cannot_remove_linked_account() {
    let db = database().await;
    accounts::ActiveModel {
        id: Set(1),
        user_id: Set(1),
        provider: Set("example".to_owned()),
        provider_account_id: Set("subject-1".to_owned()),
    }
    .insert(&db)
    .await
    .unwrap();
    let actor = credential_actor(&db).await;
    users::ActiveModel {
        id: Set(1),
        auth_epoch: Set(1),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());

    let error = store
        .remove_linked_account_if_not_last(&actor, "1")
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        magnetar::Error::NotFound {
            resource,
            identifier
        } if resource == "credential actor" && identifier == "expired or revoked"
    ));
    assert!(
        accounts::Entity::find_by_id(1)
            .one(&db)
            .await
            .unwrap()
            .is_some()
    );
}
