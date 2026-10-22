use super::*;
use suprnova::{Aggregate, EventStore, Queue, console};

#[tokio::test]
#[serial_test::serial]
async fn es_005_commands_list_events_make_snapshots_and_select_replays() {
    let _queue = Queue::fake();
    let db = super::projectors::setup().await;
    db.execute_unprepared("CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, views BIGINT NOT NULL DEFAULT 0); CREATE TABLE es_signals (id INTEGER PRIMARY KEY, total BIGINT NOT NULL)").await.unwrap();
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(RenamedAddition { amount: 2 }).await.unwrap();
    root.record(RenamedAddition { amount: 5 }).await.unwrap();
    root.persist().await.unwrap();
    let listed = console::test(["event-sourcing:events", root.uuid()])
        .run()
        .await;
    listed.assert_successful();
    let events: Vec<StoredEvent> = listed
        .output()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].aggregate_version, 2);
    assert_eq!(events[0].event_type, "es.added");
    console::test(["event-sourcing:snapshot", root.uuid()])
        .run()
        .await
        .assert_successful();
    let snapshot = EventStore::snapshot(root.uuid()).await.unwrap().unwrap();
    assert_eq!(snapshot.aggregate_version, 2);
    assert_eq!(snapshot.state["total"], 7);
    console::test([
        "event-sourcing:replay",
        "--projector",
        "es.inline",
        "--projector",
        "es.queued",
        "--from",
        "1",
    ])
    .run()
    .await
    .assert_successful();
    console::test(["event-sourcing:replay"])
        .run()
        .await
        .assert_successful();
}

#[tokio::test]
#[serial_test::serial]
async fn es_005_commands_reject_missing_unknown_and_invalid_arguments() {
    let _db = database().await;
    for args in [
        vec!["event-sourcing:events"],
        vec!["event-sourcing:events", "not-a-uuid"],
        vec!["event-sourcing:snapshot"],
        vec!["event-sourcing:snapshot", "not-a-uuid"],
        vec!["event-sourcing:replay", "--projector"],
        vec!["event-sourcing:replay", "--projector", "unknown.projector"],
        vec!["event-sourcing:replay", "--from", "-1"],
        vec!["event-sourcing:replay", "--from", "0", "--from", "0"],
        vec!["event-sourcing:replay", "--from", "invalid"],
        vec!["event-sourcing:replay", "--unknown"],
    ] {
        console::test(args).run().await.assert_failed();
    }
    let missing = uuid();
    console::test(["event-sourcing:snapshot", missing.as_str()])
        .run()
        .await
        .assert_failed();
}

#[test]
fn es_005_chapter_is_linked_and_distinguishes_events_and_workflows() {
    let chapter = include_str!("../../../manual/event-sourcing.md");
    assert!(chapter.contains("### Why Suprnova diverges"));
    assert!(chapter.contains("## Next"));
    for manual in [
        include_str!("../../../manual/documentation.md"),
        include_str!("../../../manual/events.md"),
        include_str!("../../../manual/workflows.md"),
    ] {
        assert!(manual.contains("event-sourcing.md"));
    }
}

#[test]
fn es_005_manual_warns_about_destructive_event_history_rollback() {
    let chapter = include_str!("../../../manual/event-sourcing.md");
    let migration_section = chapter
        .split("## Record and rebuild an aggregate")
        .next()
        .unwrap();
    assert!(migration_section.contains("Rolling the framework migration back drops `stored_events`, `snapshots` and\n`projector_checkpoints`, so back history up before a rollback."));
}
