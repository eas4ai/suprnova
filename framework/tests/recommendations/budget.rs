//! A deterministic million-fact SQL fixture measures the complete locked batch.
use super::support::*;
use std::time::{Duration, Instant};
use suprnova::{DB, EventType, GenerateRun, GenerationOutcome, Recommend};

// Measured 169.97 seconds on an Intel Core i9-13900K, Linux x86_64, unoptimized
// test profile on the shared development host. The 300-second budget allows
// scheduling and build contention without hiding an order-of-magnitude regression.
const MILLION_EVENT_BUDGET: Duration = Duration::from_secs(300);

#[tokio::test]
#[ignore = "slow"]
async fn million_events_complete_within_the_committed_budget() {
    setup(async {
        for id in 1..=240 { note(&format!("Item {id}"), &format!("category-{}", id % 12), true, "author", id).await; }
        let seed = suprnova::RecommendationEvent::new(EventType::Like,
            suprnova::RecommendationTarget { index: "recommendation_notes".into(), key: "1".into() },
            Some(suprnova::Actor::User("fixture".into())), "home", "fixture").await.unwrap();
        let scope = serde_json::to_string(&seed.access_scope.unwrap()).unwrap();
        // 1000 actors, 1000 events each, 200 overlapping targets, 40 unseen fallback targets.
        // SQL builds the fixture in the test without a checked-in event dump or engine shortcut.
        DB::insert("WITH RECURSIVE n(x) AS (SELECT ? UNION ALL SELECT x + ? FROM n WHERE x < ?) INSERT INTO recommendation_events (actor_kind, actor_id, target_index, target_key, event_type, weight, surface, occurred_at, access_scope, source_version, dedupe_key, event_class, score) SELECT ?, CAST(x % ? AS TEXT), ?, CAST((x / ?) % ? + ? AS TEXT), ?, ?, ?, ? + x, ?, ?, CAST(x AS TEXT), ?, ? FROM n", vec![
            0.into(), 1.into(), 999_999.into(), "user".into(), 1000.into(), "recommendation_notes".into(), 1000.into(), 200.into(), 1.into(),
            "like".into(), 0.into(), "home".into(), (chrono::Utc::now().timestamp_micros() - 1_000_000).into(), scope.into(), seed.source_version.unwrap().into(), "positive".into(), 1400.into(),
        ]).await.unwrap();
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_events", []).await.unwrap(), 1_000_000);
        let start = Instant::now();
        let outcome = Recommend::generate(GenerateRun::default()).await.unwrap();
        let elapsed = start.elapsed();
        eprintln!("million-event recommendation batch: {elapsed:?}; budget {MILLION_EVENT_BUDGET:?}");
        assert!(elapsed <= MILLION_EVENT_BUDGET, "batch took {elapsed:?}, budget {MILLION_EVENT_BUDGET:?}");
        let GenerationOutcome::Completed(run) = outcome else { panic!("fixture lock contention") };
        assert_eq!(run.events_read, 500_000);
        assert_eq!(run.new_events, 1_000_000);
        assert_eq!(run.high_water_mark, 1_000_000);
        assert_eq!(run.users_covered, 1000);
        assert!(run.coverage > 0.99 && run.items_scored > 0);
        assert_eq!(Recommend::for_user("0").limit(10).get().await.unwrap().len(), 10);
    }).await;
}
