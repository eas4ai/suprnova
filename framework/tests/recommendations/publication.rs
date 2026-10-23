use super::support::*;
use suprnova::{DB, EventType, GenerateRun, GenerationOutcome, Recommend};

#[tokio::test]
async fn partial_snapshot_stays_invisible_and_failed_batch_keeps_previous_current() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let c = note("C", "rust", true, "alice", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        let GenerationOutcome::Completed(first) = Recommend::generate(GenerateRun::default()).await.unwrap() else { panic!("lock") };
        let expected = Recommend::for_user("alice").get().await.unwrap();
        assert_eq!(expected[0].target, key(&c));
        DB::insert("INSERT INTO recommendation_runs (id, started_at, duration_ms, events_read, new_events, users_covered, items_scored, coverage, fallback_share, high_water_mark, state) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", vec!["partial".into(), (first.started_at + 1).into(), 0.into(), 0.into(), 0.into(), 0.into(), 0.into(), 0.0.into(), 0.0.into(), 0.into(), "running".into()]).await.unwrap();
        DB::insert("INSERT INTO starter_recommendations (run_id, actor_kind, actor_id, surface, target_index, target_key, score, rank, explanation_id) SELECT ?, actor_kind, actor_id, surface, target_index, target_key, score, rank, explanation_id FROM starter_recommendations WHERE run_id = ?", vec!["partial".into(), first.id.clone().into()]).await.unwrap();
        let during = Recommend::for_user("alice").get().await.unwrap();
        assert_eq!(during, expected);
        // Fail after staging starts, exercising cleanup against real SQL writes.
        DB::unprepared("CREATE TRIGGER fail_recommendations BEFORE INSERT ON recommendation_explanations BEGIN SELECT RAISE(FAIL, 'fixture stage failure'); END").await.unwrap();
        assert!(Recommend::generate(GenerateRun::default()).await.is_err());
        assert_eq!(Recommend::for_user("alice").get().await.unwrap(), expected);
        let runs = Recommend::status(10).await.unwrap();
        assert_eq!(runs[0].state, "failed");
        assert!(runs[0].error.as_ref().unwrap().contains("fixture stage failure"));
        for run in runs.iter().filter(|run| run.state == "failed") {
            assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM starter_recommendations WHERE run_id = ?", vec![run.id.clone().into()]).await.unwrap(), 0);
            assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_explanations WHERE run_id = ?", vec![run.id.clone().into()]).await.unwrap(), 0);
        }
        DB::unprepared("DROP TRIGGER fail_recommendations").await.unwrap();
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let final_rows = Recommend::for_user("alice").get().await.unwrap();
        assert_eq!(final_rows.len(), 1);
        assert!(final_rows.iter().all(|row| row.run_id != Some(first.id.clone())));
    }).await;
}

#[tokio::test]
async fn late_commit_below_watermark_invalidates_actor_history() {
    setup(async {
        let a = note("A", "seed", true, "alice", 1).await;
        let c = note("C", "other", true, "author", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        record("bob", &a, EventType::Like, "a").await;
        let late = event("alice", &c, EventType::Hide, "late").await;
        let gap = Recommend::record(late.clone()).await.unwrap();
        // Simulate an allocated but uncommitted sequence id, then a later committed id.
        DB::delete("DELETE FROM recommendation_events WHERE id = ?", vec![gap.id.into()]).await.unwrap();
        record("bob", &c, EventType::Like, "c").await;
        let GenerationOutcome::Completed(first) = Recommend::generate(GenerateRun::default()).await.unwrap() else { panic!("lock") };
        assert_eq!(Recommend::for_user("alice").get().await.unwrap()[0].target, key(&c));
        DB::insert("INSERT INTO recommendation_events (id, actor_kind, actor_id, target_index, target_key, event_type, weight, surface, occurred_at, access_scope, source_version, dedupe_key, event_class, score) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", vec![
            gap.id.into(), "user".into(), "alice".into(), "recommendation_notes".into(), c.id.to_string().into(), "hide".into(), 0.into(), "home".into(),
            late.occurred_at.timestamp_micros().into(), serde_json::to_string(&late.access_scope.unwrap()).unwrap().into(), late.source_version.unwrap().into(), "late".into(), "negative".into(), (-1000).into(),
        ]).await.unwrap();
        let GenerationOutcome::Completed(second) = Recommend::generate(GenerateRun::default()).await.unwrap() else { panic!("lock") };
        assert_eq!(second.high_water_mark, first.high_water_mark);
        assert_eq!(second.new_events, 0);
        assert!(Recommend::for_user("alice").get().await.unwrap().is_empty());
    }).await;
}

#[tokio::test]
async fn filtered_run_preserves_pending_catalog_changes_for_other_actors() {
    setup(async {
        let a = note("A", "seed", true, "alice", 1).await;
        let c = note("C", "other", true, "author", 2).await;
        for actor in ["alice", "bob", "charlie"] {
            record(actor, &a, EventType::Like, "a").await;
        }
        record("charlie", &c, EventType::Like, "c").await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert_eq!(Recommend::for_user("bob").get().await.unwrap().len(), 1);
        let d = note("D", "new", true, "author", 3).await;
        Recommend::generate(GenerateRun {
            user: Some("alice".into()),
            surface: None,
        })
        .await
        .unwrap();
        assert_eq!(Recommend::for_user("bob").get().await.unwrap().len(), 1);
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let rows = Recommend::for_user("bob").get().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target, key(&c));
        assert_eq!(rows[1].target, key(&d));
        assert!(
            Recommend::status(10)
                .await
                .unwrap()
                .iter()
                .any(|run| run.state == "complete")
        );
    })
    .await;
}
