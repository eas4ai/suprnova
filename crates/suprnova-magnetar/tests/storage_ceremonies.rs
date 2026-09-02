#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use chrono::{Duration as ChronoDuration, Utc};
use magnetar::storage::{CeremonyStore, NewCeremony, SeaOrmStorage};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use storage_schema::{StorageSchema, database};

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn peek_does_not_delete_and_consume_respects_kind_and_row_id() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let expires = Utc::now() + ChronoDuration::minutes(5);
    let first = store
        .create(NewCeremony {
            selector: "reuse".into(),
            kind: "login".into(),
            state: "pending".into(),
            payload: b"one".to_vec(),
            expires_at: expires,
        })
        .await
        .unwrap();
    assert_eq!(
        store.peek("reuse", "login").await.unwrap().unwrap().id,
        first.id
    );
    assert!(store.consume("reuse", "wrong").await.unwrap().is_none());
    assert_eq!(
        store
            .consume("reuse", "login")
            .await
            .unwrap()
            .unwrap()
            .payload,
        b"one"
    );
    let second = store
        .create(NewCeremony {
            selector: "reuse".into(),
            kind: "login".into(),
            state: "pending".into(),
            payload: b"two".to_vec(),
            expires_at: expires,
        })
        .await
        .unwrap();
    let wrong_kind = store
        .create(NewCeremony {
            selector: "reuse".into(),
            kind: "reset".into(),
            state: "pending".into(),
            payload: b"wrong-kind".to_vec(),
            expires_at: expires,
        })
        .await
        .unwrap();
    assert_eq!(
        store.peek("reuse", "login").await.unwrap().unwrap().id,
        second.id
    );
    assert_eq!(
        store.peek("reuse", "reset").await.unwrap().unwrap().id,
        wrong_kind.id
    );
    let binary = store
        .create(NewCeremony {
            selector: "binary".into(),
            kind: "login".into(),
            state: "pending".into(),
            payload: vec![0xff, 0x00, 0x80],
            expires_at: expires,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .peek("binary", "login")
            .await
            .unwrap()
            .unwrap()
            .payload,
        binary.payload
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn conditional_transition_has_one_winner() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    store
        .create(NewCeremony {
            selector: "device".into(),
            kind: "device-authorization".into(),
            state: "pending".into(),
            payload: vec![],
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .unwrap();
    store
        .create(NewCeremony {
            selector: "device".into(),
            kind: "device-authorization".into(),
            state: "pending".into(),
            payload: b"second".to_vec(),
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .unwrap();
    assert!(
        store
            .transition("device", "device-authorization", "pending", "approved")
            .await
            .unwrap()
    );
    use sea_orm::EntityTrait;
    let rows = storage_schema::ceremonies::Entity::find()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.iter().filter(|row| row.state == "approved").count(), 1);
    assert_eq!(rows.iter().filter(|row| row.state == "pending").count(), 1);
    assert!(
        store
            .transition("device", "device-authorization", "pending", "denied")
            .await
            .unwrap()
    );
    assert!(
        !store
            .transition("device", "device-authorization", "pending", "denied")
            .await
            .unwrap()
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn device_peek_approve_deny_is_single_winner() {
    use magnetar::storage::DeviceStore;
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    store
        .create(NewCeremony {
            selector: "device-test".into(),
            kind: "device-authorization".into(),
            state: "pending".into(),
            payload: b"device".to_vec(),
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .unwrap();
    assert!(store.peek_device("device-test").await.unwrap().is_some());
    assert!(store.approve_device("device-test").await.unwrap());
    assert!(!store.deny_device("device-test").await.unwrap());
    assert_eq!(
        store
            .peek_device("device-test")
            .await
            .unwrap()
            .unwrap()
            .state,
        "approved"
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn racing_consumers_have_one_ceremony_winner() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    store
        .create(NewCeremony {
            selector: "race".into(),
            kind: "login".into(),
            state: "pending".into(),
            payload: vec![],
            expires_at: Utc::now() + ChronoDuration::minutes(5),
        })
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        store.consume("race", "login"),
        store.consume("race", "login")
    );
    assert_eq!(
        usize::from(matches!(left, Ok(Some(_)))) + usize::from(matches!(right, Ok(Some(_)))),
        1
    );
}

#[tokio::test]
async fn delete_failure_rolls_back_transition_and_preserves_grant() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    store
        .create(NewCeremony {
            selector: "delete-failure-device".into(),
            kind: "device-authorization".into(),
            state: "approved:delete-failure-grant".into(),
            payload: b"device".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    store
        .create(NewCeremony {
            selector: "delete-failure-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant".to_vec(),
            expires_at,
        })
        .await
        .unwrap();

    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TRIGGER fail_device_grant_delete
         BEFORE DELETE ON storage_ceremonies
         WHEN OLD.kind = 'device-authorization-grant'
          AND OLD.selector = 'delete-failure-grant'
         BEGIN
             SELECT RAISE(ABORT, 'forced device grant delete failure');
         END"
        .to_owned(),
    ))
    .await
    .expect("install scoped grant delete failure trigger");

    let error = store
        .transition_and_consume(
            "delete-failure-device",
            "device-authorization",
            "approved:delete-failure-grant",
            "issued",
            "delete-failure-grant",
            "device-authorization-grant",
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        magnetar::Error::Internal { message }
            if message.contains("forced device grant delete failure")
    ));
    assert_eq!(
        store
            .peek("delete-failure-device", "device-authorization")
            .await
            .unwrap()
            .unwrap()
            .state,
        "approved:delete-failure-grant"
    );
    assert!(
        store
            .peek("delete-failure-grant", "device-authorization-grant")
            .await
            .unwrap()
            .is_some()
    );

    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER fail_device_grant_delete".to_owned(),
    ))
    .await
    .expect("remove grant delete failure trigger before retry");

    let grant = store
        .transition_and_consume(
            "delete-failure-device",
            "device-authorization",
            "approved:delete-failure-grant",
            "issued",
            "delete-failure-grant",
            "device-authorization-grant",
        )
        .await
        .unwrap()
        .expect("retry returns the preserved grant");
    assert_eq!(grant.payload, b"grant");
    assert_eq!(
        store
            .peek("delete-failure-device", "device-authorization")
            .await
            .unwrap()
            .unwrap()
            .state,
        "issued"
    );
    assert!(
        store
            .transition_and_consume(
                "delete-failure-device",
                "device-authorization",
                "issued",
                "issued",
                "delete-failure-grant",
                "device-authorization-grant",
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn exact_transition_and_consume_rejects_replaced_consume_record() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    store
        .create(NewCeremony {
            selector: "replacement-device".into(),
            kind: "device-authorization".into(),
            state: "approved:replacement-grant".into(),
            payload: b"device".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    store
        .create(NewCeremony {
            selector: "replacement-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant-a".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    let grant_a = store
        .peek("replacement-grant", "device-authorization-grant")
        .await
        .unwrap()
        .expect("grant A is available for preflight");
    assert_eq!(
        store
            .consume("replacement-grant", "device-authorization-grant")
            .await
            .unwrap()
            .expect("grant A can be consumed before replacement")
            .id,
        grant_a.id
    );
    let grant_b = store
        .create(NewCeremony {
            selector: "replacement-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant-b".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    assert_ne!(grant_b.id, grant_a.id);

    assert!(
        store
            .transition_and_consume_exact(
                "replacement-device",
                "device-authorization",
                "approved:replacement-grant",
                "issued",
                "replacement-grant",
                "device-authorization-grant",
                &grant_a.id,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .peek("replacement-device", "device-authorization")
            .await
            .unwrap()
            .expect("stale grant ID must not advance the device ceremony")
            .state,
        "approved:replacement-grant"
    );
    let preserved = store
        .peek("replacement-grant", "device-authorization-grant")
        .await
        .unwrap()
        .expect("stale grant ID must not consume replacement grant B");
    assert_eq!(preserved.id, grant_b.id);
    assert_eq!(preserved.payload, b"grant-b");
}

#[tokio::test]
async fn exact_transition_and_consume_selects_expected_duplicate_record() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    store
        .create(NewCeremony {
            selector: "duplicate-device".into(),
            kind: "device-authorization".into(),
            state: "approved:duplicate-grant".into(),
            payload: b"device".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    let grant_a = store
        .create(NewCeremony {
            selector: "duplicate-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant-a".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    let grant_b = store
        .create(NewCeremony {
            selector: "duplicate-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant-b".to_vec(),
            expires_at,
        })
        .await
        .unwrap();

    let consumed = store
        .transition_and_consume_exact(
            "duplicate-device",
            "device-authorization",
            "approved:duplicate-grant",
            "issued",
            "duplicate-grant",
            "device-authorization-grant",
            &grant_b.id,
        )
        .await
        .unwrap()
        .expect("the exact operation must select grant B by ID");
    assert_eq!(consumed.id, grant_b.id);
    assert_eq!(consumed.payload, b"grant-b");
    assert_eq!(
        store
            .peek("duplicate-device", "device-authorization")
            .await
            .unwrap()
            .expect("the matching transition must win")
            .state,
        "issued"
    );
    let preserved = store
        .peek("duplicate-grant", "device-authorization-grant")
        .await
        .unwrap()
        .expect("the unselected duplicate grant A must remain live");
    assert_eq!(preserved.id, grant_a.id);
    assert_eq!(preserved.payload, b"grant-a");
}

#[tokio::test]
async fn transition_and_consume_lost_comparison_is_non_destructive_and_single_winner() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    store
        .create(NewCeremony {
            selector: "race-device".into(),
            kind: "device-authorization".into(),
            state: "approved:race-grant".into(),
            payload: b"device".to_vec(),
            expires_at,
        })
        .await
        .unwrap();
    store
        .create(NewCeremony {
            selector: "race-grant".into(),
            kind: "device-authorization-grant".into(),
            state: "available".into(),
            payload: b"grant".to_vec(),
            expires_at,
        })
        .await
        .unwrap();

    assert!(
        store
            .transition_and_consume(
                "race-device",
                "device-authorization",
                "approved:stale-grant",
                "issued",
                "race-grant",
                "device-authorization-grant",
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .peek("race-device", "device-authorization")
            .await
            .unwrap()
            .unwrap()
            .state,
        "approved:race-grant"
    );
    assert!(
        store
            .peek("race-grant", "device-authorization-grant")
            .await
            .unwrap()
            .is_some()
    );

    let (left, right) = tokio::join!(
        store.transition_and_consume(
            "race-device",
            "device-authorization",
            "approved:race-grant",
            "issued",
            "race-grant",
            "device-authorization-grant",
        ),
        store.transition_and_consume(
            "race-device",
            "device-authorization",
            "approved:race-grant",
            "issued",
            "race-grant",
            "device-authorization-grant",
        )
    );
    let outcomes = [left.unwrap(), right.unwrap()];
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_some()).count(),
        1
    );
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_none()).count(),
        1
    );
    assert_eq!(
        store
            .peek("race-device", "device-authorization")
            .await
            .unwrap()
            .unwrap()
            .state,
        "issued"
    );
    assert!(
        store
            .peek("race-grant", "device-authorization-grant")
            .await
            .unwrap()
            .is_none()
    );
}
