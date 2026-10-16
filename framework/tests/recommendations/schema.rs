use super::support::*;
use suprnova::{DB, EventType, GenerateRun, Recommend, RecommendationTarget};

#[tokio::test]
async fn versioned_schema_has_every_graph_input_column_and_batch_index() {
    setup(async {
        for (table, columns) in [
            ("recommendation_events", vec!["id", "actor_kind", "actor_id", "target_index", "target_key", "event_type", "weight", "surface", "occurred_at", "access_scope", "source_version", "dedupe_key", "event_class", "score"]),
            ("recommendation_explanations", vec!["id", "run_id", "event_ids", "attribute_values", "peer_count", "shared_target_count", "shared_targets", "fallback_tier"]),
            ("starter_recommendations", vec!["id", "run_id", "actor_kind", "actor_id", "surface", "target_index", "target_key", "score", "rank", "explanation_id"]),
            ("recommendation_runs", vec!["id", "started_at", "duration_ms", "events_read", "new_events", "users_covered", "items_scored", "coverage", "fallback_share", "high_water_mark", "catalog_digest", "actor_history_snapshots", "state", "error"]),
            ("recommendation_schema", vec!["id", "recommendation_schema_version"]),
        ] {
            let rows = DB::select(&format!("PRAGMA table_info({table})"), []).await.unwrap();
            let names: Vec<_> = rows.iter().map(|row| row.get_string("name").unwrap()).collect();
            assert_eq!(names.len(), columns.len(), "{table}");
            for column in columns { assert!(names.iter().any(|name| name == column), "{table}.{column}"); }
        }
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_schema WHERE id = ? AND recommendation_schema_version = ?", vec![1.into(), 1.into()]).await.unwrap(), 1);
        let indexes = DB::select("PRAGMA index_list(recommendation_events)", []).await.unwrap();
        assert!(indexes.iter().any(|index| index.get_string("name").unwrap() == "recommendation_events_dedupe"));
        assert!(indexes.len() >= 4);
        let names: Vec<_> = suprnova::inventory::iter::<suprnova::console::CommandEntry>.into_iter().map(|entry| entry.name).collect();
        for name in ["recommendations:generate", "recommendations:status", "recommendations:prune"] { assert!(names.contains(&name), "{name}"); }
    }).await;
}

#[tokio::test]
async fn search_bridge_obeys_configuration_and_retention_is_explicit() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let actor = suprnova::Actor::guest("trusted server session").unwrap();
        assert!(
            Recommend::record_search_event(
                EventType::Click,
                key(&a),
                actor.clone(),
                "search",
                "click"
            )
            .await
            .unwrap()
            .is_none()
        );
        suprnova::RecommendConfig::scope(
            suprnova::RecommendConfig {
                enabled: true,
                ..Default::default()
            },
            async {
                let reference = Recommend::record_search_event(
                    EventType::Click,
                    key(&a),
                    actor.clone(),
                    "search",
                    "click",
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(
                    Recommend::record_search_event(
                        EventType::Click,
                        key(&a),
                        actor.clone(),
                        "search",
                        "click"
                    )
                    .await
                    .unwrap(),
                    Some(reference)
                );
                assert!(
                    Recommend::record_search_event(
                        EventType::Like,
                        key(&a),
                        actor,
                        "search",
                        "bad"
                    )
                    .await
                    .is_err()
                );
            },
        )
        .await
        .unwrap();
        let mut old = event("alice", &a, EventType::View, "old").await;
        old.occurred_at -= chrono::Duration::days(31);
        let reference = Recommend::record(old).await.unwrap();
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert_eq!(
            DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_events", [])
                .await
                .unwrap(),
            2
        );
        assert!(Recommend::prune(0).await.is_err());
        assert_eq!(Recommend::prune(30).await.unwrap(), 1);
        assert_eq!(
            Recommend::verify_provenance(&reference).await.unwrap(),
            suprnova::ProvenanceState::Removed
        );
        assert_eq!(
            DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_events", [])
                .await
                .unwrap(),
            1
        );
        assert!(
            Recommend::similar_to(RecommendationTarget {
                index: "recommendation_notes".into(),
                key: "missing".into()
            })
            .get()
            .await
            .is_err()
        );
    })
    .await;
}
