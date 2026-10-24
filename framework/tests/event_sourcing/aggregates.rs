use super::*;
use suprnova::{Aggregate, DB, EventStore};

#[tokio::test]
#[serial_test::serial]
async fn es_001_recording_rebuilds_exact_state() {
    let _db = database().await;
    let mut tally = Aggregate::<Tally>::new(&uuid()).unwrap();
    tally.record(RenamedAddition { amount: 3 }).await.unwrap();
    tally.record(RenamedAddition { amount: 7 }).await.unwrap();
    assert_eq!(tally.state().total, 10);
    tally.persist().await.unwrap();
    let rebuilt = Aggregate::<Tally>::retrieve(tally.uuid()).await.unwrap();
    assert_eq!(tally.state(), rebuilt.state());
    assert_eq!(rebuilt.persisted_version(), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_snapshot_plus_tail_equals_full_rebuild() {
    let _db = database().await;
    let mut tally = Aggregate::<Tally>::new(&uuid()).unwrap();
    tally.record(RenamedAddition { amount: 4 }).await.unwrap();
    tally.persist().await.unwrap();
    tally.snapshot().await.unwrap();
    tally.snapshot().await.unwrap();
    tally.record(RenamedAddition { amount: 9 }).await.unwrap();
    assert!(tally.snapshot().await.is_err());
    tally.persist().await.unwrap();
    let snap = Aggregate::<Tally>::retrieve(tally.uuid()).await.unwrap();
    let full = Aggregate::<Tally>::retrieve_without_snapshot(tally.uuid())
        .await
        .unwrap();
    assert_eq!(snap.state(), full.state());
    assert_eq!(snap.persisted_version(), 2);
    assert_eq!(
        EventStore::snapshot(tally.uuid())
            .await
            .unwrap()
            .unwrap()
            .aggregate_version,
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn es_001_concurrent_persists_have_one_winner() {
    use suprnova::testing::TestContainer;
    TestContainer::scope(async {
        let _db = database().await;
        let id = uuid();
        let mut left = Aggregate::<Tally>::retrieve(&id).await.unwrap();
        let mut right = Aggregate::<Tally>::retrieve(&id).await.unwrap();
        left.record(RenamedAddition { amount: 1 }).await.unwrap();
        right.record(RenamedAddition { amount: 2 }).await.unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let other = barrier.clone();
        let left = TestContainer::spawn(async move {
            other.wait().await;
            left.persist().await
        });
        let right = TestContainer::spawn(async move {
            barrier.wait().await;
            right.persist().await
        });
        let (left, right) = tokio::join!(left, right);
        let (left, right) = (left.unwrap(), right.unwrap());
        assert_ne!(left.is_ok(), right.is_ok());
        let error = left.err().or(right.err()).unwrap().to_string();
        assert!(
            error.contains(&id)
                && error.contains("expected version 0")
                && error.contains("version 1"),
            "{error}"
        );
        assert_eq!(EventStore::events(&id).await.unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn es_002_stable_name_reads_renamed_type_and_upgrades_old_payload() {
    let _db = database().await;
    let id = uuid();
    DB::statement("INSERT INTO stored_events (aggregate_uuid, aggregate_version, event_version, event_type, payload, metadata, created_at) VALUES (?, 1, 1, 'es.added', ?, ?, '2026-10-22 00:00:00')",
        vec![id.clone().into(), r#"{"old_amount":8}"#.into(), r#"{"aggregate_type":"es.tally"}"#.into()]).await.unwrap();
    let tally = Aggregate::<Tally>::retrieve(&id).await.unwrap();
    assert_eq!(tally.state().total, 8);
    let events = EventStore::events(&id).await.unwrap();
    assert_eq!(events[0].event_type, "es.added");
    assert_eq!(events[0].event_version, 1);
    assert_eq!(
        events[0]
            .decode::<RenamedAddition>()
            .unwrap()
            .unwrap()
            .amount,
        8
    );
}

#[tokio::test]
#[serial_test::serial]
async fn es_002_unknown_type_fails_by_name() {
    let _db = database().await;
    let mut tally = Aggregate::<Tally>::new(&uuid()).unwrap();
    tally.record(RenamedAddition { amount: 1 }).await.unwrap();
    tally.persist().await.unwrap();
    DB::statement(
        "UPDATE stored_events SET event_type = 'unknown.fact'",
        vec![],
    )
    .await
    .unwrap();
    let error = Aggregate::<Tally>::retrieve(tally.uuid())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown.fact"), "{error}");
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_failed_batch_rolls_back_and_keeps_recorded_facts() {
    let db = database().await;
    let mut tally = Aggregate::<Tally>::new(&uuid()).unwrap();
    tally.record(RenamedAddition { amount: 1 }).await.unwrap();
    tally.record(RenamedAddition { amount: 2 }).await.unwrap();
    db.execute_unprepared("CREATE TRIGGER reject_second BEFORE INSERT ON stored_events WHEN NEW.aggregate_version = 2 BEGIN SELECT RAISE(ABORT, 'second fact refused'); END").await.unwrap();
    assert!(tally.persist().await.is_err());
    assert_eq!(tally.recorded().len(), 2);
    assert_eq!(tally.persisted_version(), 0);
    assert!(EventStore::events(tally.uuid()).await.unwrap().is_empty());
    db.execute_unprepared("DROP TRIGGER reject_second")
        .await
        .unwrap();
    tally.persist().await.unwrap();
    assert_eq!(
        Aggregate::<Tally>::retrieve(tally.uuid())
            .await
            .unwrap()
            .state()
            .total,
        3
    );
}

#[tokio::test]
#[serial_test::serial]
async fn es_002_future_payload_version_fails_by_name() {
    let _db = database().await;
    let mut tally = Aggregate::<Tally>::new(&uuid()).unwrap();
    tally.record(RenamedAddition { amount: 1 }).await.unwrap();
    tally.persist().await.unwrap();
    DB::statement("UPDATE stored_events SET event_version = 3", vec![])
        .await
        .unwrap();
    let error = EventStore::events(tally.uuid())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("es.added") && error.contains("version 3"),
        "{error}"
    );
}

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
struct EveryFactField {
    seen: Vec<serde_json::Value>,
}
impl AggregateRoot for EveryFactField {
    const NAME: &'static str = "es.every-fact-field";
    fn apply(&mut self, event: &Fact<'_>) -> Result<(), FrameworkError> {
        self.seen.push(
            serde_json::to_value(event)
                .map_err(|cause| FrameworkError::internal(cause.to_string()))?,
        );
        Ok(())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn es_001_fact_view_matches_recording_snapshot_and_full_rebuild() {
    let _db = database().await;
    let mut root = Aggregate::<EveryFactField>::new(&uuid()).unwrap();
    for amount in 1..=3 {
        root.record_with_metadata(
            RenamedAddition { amount },
            serde_json::json!({"source":amount}),
        )
        .await
        .unwrap();
    }
    let recorded = root.state().clone();
    root.persist().await.unwrap();
    let snapshot = root.snapshot().await.unwrap();
    let full = Aggregate::<EveryFactField>::retrieve_without_snapshot(root.uuid())
        .await
        .unwrap();
    let snap = Aggregate::<EveryFactField>::retrieve(root.uuid())
        .await
        .unwrap();
    assert_eq!(recorded, *full.state());
    assert_eq!(snap.state(), full.state());
    assert_eq!(snapshot.state, serde_json::to_value(full.state()).unwrap());
    for (index, fact) in recorded.seen.iter().enumerate() {
        let fields = fact.as_object().unwrap();
        assert_eq!(
            fields.len(),
            5,
            "Fact contains only stable application fields"
        );
        assert!(!fields.contains_key("id"));
        assert!(!fields.contains_key("created_at"));
        assert_eq!(fact["event_type"], "es.added");
        assert_eq!(fact["event_version"], 2);
        assert_eq!(fact["aggregate_version"], index as i64 + 1);
        assert_eq!(fact["payload"]["amount"], index as i64 + 1);
        assert_eq!(fact["metadata"]["source"], index as i64 + 1);
    }
}
