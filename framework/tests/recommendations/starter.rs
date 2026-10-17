use super::support::*;
use std::time::Duration;
use suprnova::{Cache, DB, EventType, GenerateRun, GenerationOutcome, Recommend, Viewer};

#[tokio::test]
async fn shared_targets_recommend_unseen_c_and_project_explanations() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let b = note("B", "rust", true, "alice", 2).await;
        let c = note("C", "other", true, "alice", 3).await;
        record("alice", &a, EventType::Like, "a").await;
        record("alice", &b, EventType::Like, "b").await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &b, EventType::Like, "b").await;
            record(peer, &c, EventType::Like, "c").await;
        }
        let GenerationOutcome::Completed(run) =
            Recommend::generate(GenerateRun::default()).await.unwrap()
        else {
            panic!("unexpected contention")
        };
        assert_eq!(run.state, "complete");
        assert_eq!(run.users_covered, 4);
        assert_eq!(run.events_read, 11);
        assert_eq!(run.high_water_mark, 11);
        assert!(run.items_scored > 0 && run.coverage > 0.0 && run.duration_ms >= 0);
        let rows = Recommend::for_user("alice")
            .surface("home")
            .limit(1)
            .for_viewer(Viewer::Guest)
            .get()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target, key(&c));
        assert_eq!(rows[0].explanation.peer_count, 3);
        assert_eq!(rows[0].explanation.shared_target_count, 2);
        let text = serde_json::to_string(&rows[0].explanation).unwrap();
        for name in ["alice", "bob", "carol", "dave"] {
            assert!(!text.contains(name));
        }
    })
    .await;
}

#[tokio::test]
async fn lock_contention_returns_immediately_and_releases_after_runs() {
    setup(async {
        let lock = Cache::lock("recommendations:generate", Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let start = std::time::Instant::now();
        assert!(matches!(
            Recommend::generate(GenerateRun::default()).await.unwrap(),
            GenerationOutcome::Locked
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(
            DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_runs", [])
                .await
                .unwrap(),
            0
        );
        assert!(lock.release().await.unwrap());
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert!(
            Cache::lock("recommendations:generate", Duration::from_secs(60))
                .await
                .unwrap()
                .unwrap()
                .release()
                .await
                .unwrap()
        );
    })
    .await;
}

#[tokio::test]
async fn negative_and_seen_targets_are_excluded_before_fallbacks() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let hidden = note("Hidden", "rust", true, "alice", 2).await;
        let seen = note("Seen", "rust", true, "alice", 3).await;
        let affinity = note("Affinity", "rust", true, "alice", 4).await;
        let popular = note("Popular", "other", true, "alice", 5).await;
        let recent = note("Recent", "new", true, "alice", 100).await;
        record("alice", &a, EventType::Like, "a").await;
        record("alice", &hidden, EventType::Hide, "hide").await;
        record("alice", &seen, EventType::Impression, "seen").await;
        record("bob", &popular, EventType::Like, "popular").await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let results = Recommend::for_user("alice")
            .surface("home")
            .limit(3)
            .get()
            .await
            .unwrap();
        assert_eq!(
            results.iter().map(|r| r.target.clone()).collect::<Vec<_>>(),
            vec![key(&affinity), key(&popular), key(&recent)]
        );
        assert_eq!(
            results
                .iter()
                .map(|r| r.explanation.fallback_tier.as_str())
                .collect::<Vec<_>>(),
            vec!["affinity", "popularity", "recent"]
        );
    })
    .await;
}

#[tokio::test]
async fn reads_check_current_viewer_scope_and_live_rows_and_seed_scope() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let c = note("Secret", "rust", true, "alice", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        record("bob", &a, EventType::Like, "a").await;
        record("bob", &c, EventType::Like, "c").await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        // Bypass the indexing hook to exercise stale index authorization.
        DB::update(
            "UPDATE recommendation_notes SET public = ? WHERE id = ?",
            vec![false.into(), c.id.into()],
        )
        .await
        .unwrap();
        assert!(
            Recommend::for_user("alice")
                .surface("home")
                .for_viewer(Viewer::Guest)
                .get()
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            Recommend::similar_to(key(&c))
                .for_viewer(Viewer::Guest)
                .get()
                .await
                .is_err()
        );
        assert_eq!(
            Recommend::for_user("alice")
                .surface("home")
                .for_viewer(viewer("alice"))
                .get()
                .await
                .unwrap()
                .len(),
            1
        );
        DB::update(
            "UPDATE recommendation_notes SET public = ? WHERE id = ?",
            vec![false.into(), a.id.into()],
        )
        .await
        .unwrap();
        let results = Recommend::for_user("alice")
            .surface("home")
            .for_viewer(viewer("alice"))
            .get()
            .await
            .unwrap();
        assert!(!results.is_empty());
        // The public explanation is counts and categories, never stored event ids.
        let explanation = serde_json::to_value(&results[0].explanation).unwrap();
        assert!(explanation.get("event_ids").is_none());
    })
    .await;
}

#[tokio::test]
async fn history_reads_only_newest_five_hundred_and_incremental_run_keeps_other_actors() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let c = note("C", "rust", true, "alice", 2).await;
        for i in 0..600 {
            record("alice", &a, EventType::Like, &i.to_string()).await;
        }
        let GenerationOutcome::Completed(first) =
            Recommend::generate(GenerateRun::default()).await.unwrap()
        else {
            panic!("lock")
        };
        assert_eq!(first.events_read, 500);
        assert_eq!(first.high_water_mark, 600);
        record("bob", &a, EventType::Like, "a").await;
        record("bob", &c, EventType::Like, "c").await;
        let GenerationOutcome::Completed(second) =
            Recommend::generate(GenerateRun::default()).await.unwrap()
        else {
            panic!("lock")
        };
        assert_eq!(second.high_water_mark, 602);
        assert_eq!(second.new_events, 2);
        assert_eq!(
            Recommend::for_user("alice")
                .surface("home")
                .get()
                .await
                .unwrap()[0]
                .target,
            key(&c)
        );
        assert_eq!(
            DB::scalar::<i64>(
                "SELECT COUNT(*) FROM starter_recommendations WHERE run_id = ?",
                vec![first.id.into()]
            )
            .await
            .unwrap(),
            0
        );
    })
    .await;
}

#[tokio::test]
async fn scoped_read_fills_after_private_collaborative_candidate_and_includes_owner_fallback() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let private = note("Private", "rust", false, "alice", 2).await;
        let public = note("Public", "rust", true, "alice", 3).await;
        record("alice", &a, EventType::Like, "a").await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &private, EventType::Like, "private").await;
        }
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let guest = Recommend::for_user("alice")
            .limit(1)
            .for_viewer(Viewer::Guest)
            .get()
            .await
            .unwrap();
        assert_eq!(guest.len(), 1);
        assert_eq!(guest[0].target, key(&public));
        let owner = Recommend::for_user("alice")
            .limit(1)
            .for_viewer(viewer("alice"))
            .get()
            .await
            .unwrap();
        assert_eq!(owner[0].target, key(&private));
        assert!(owner[0].explanation.peer_count > 0);
        let fallback = Recommend::for_user("bob")
            .for_viewer(viewer("alice"))
            .get()
            .await
            .unwrap();
        assert!(fallback.iter().any(|row| row.target == key(&public)));
    })
    .await;
}

#[tokio::test]
async fn empty_results_recover_on_catalog_growth_without_new_events() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        record("alice", &a, EventType::Like, "a").await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert!(Recommend::for_user("alice").get().await.unwrap().is_empty());
        let b = note("B", "rust", true, "alice", 2).await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert_eq!(
            Recommend::for_user("alice").get().await.unwrap()[0].target,
            key(&b)
        );
        let private = note("Owner only", "rust", false, "alice", 3).await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        assert!(
            Recommend::for_user("alice")
                .for_viewer(viewer("alice"))
                .get()
                .await
                .unwrap()
                .iter()
                .any(|row| row.target == key(&private))
        );
    })
    .await;
}

#[tokio::test]
async fn similarity_folds_semantic_neighbors_and_rechecks_scope() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let b = note("B", "other", true, "alice", 2).await;
        let secret = note("Secret", "other", false, "bob", 3).await;
        let fake = std::sync::Arc::new(suprnova::FakeEmbedder::new(8).unwrap());
        suprnova::SearchRegistry::register_embedder("fake", fake).unwrap();
        let descriptor = <Note as suprnova::Searchable>::descriptor();
        let engine = suprnova::SearchRegistry::engine_for(descriptor.name).unwrap();
        let old = suprnova::SearchRegistry::generation_for(descriptor.name)
            .await
            .unwrap();
        let mut settings = engine.read_settings(descriptor, &old).await.unwrap();
        settings.index_time.embedder = Some("fake".into());
        settings.index_time.dimensions = Some(8);
        let new = suprnova::GenerationId::new("with_vectors").unwrap();
        engine
            .create_generation(descriptor, &settings, &new)
            .await
            .unwrap();
        engine
            .switch_generation(descriptor, Some(old), new)
            .await
            .unwrap();
        suprnova::searchable::<Note>(&[a.id.to_string(), b.id.to_string(), secret.id.to_string()])
            .await
            .unwrap();
        let results = Recommend::similar_to(key(&a))
            .for_viewer(Viewer::Guest)
            .limit(2)
            .get()
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target, key(&b));
        assert_eq!(results[0].explanation.fallback_tier, "semantic");
        assert!(
            Recommend::similar_to(key(&secret))
                .for_viewer(Viewer::Guest)
                .get()
                .await
                .is_err()
        );
        record("alice", &a, EventType::Like, "a").await;
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let personal = Recommend::for_user("alice")
            .for_viewer(Viewer::Guest)
            .get()
            .await
            .unwrap();
        assert_eq!(personal[0].target, key(&b));
        assert_eq!(personal[0].explanation.fallback_tier, "semantic");
    })
    .await;
}

#[tokio::test]
async fn most_recent_shared_peers_are_sampled_to_the_configured_cap() {
    setup(async {
        let a = note("A", "seed", true, "alice", 1).await;
        let older = note("Older peer", "old", true, "old", 2).await;
        let newest = note("Newest peer", "new", true, "new", 3).await;
        record("alice", &a, EventType::Like, "a").await;
        record("old", &a, EventType::Like, "a").await;
        record("old", &older, EventType::Like, "older").await;
        record("new", &a, EventType::Like, "a").await;
        record("new", &newest, EventType::Like, "newest").await;
        suprnova::RecommendConfig::scope(
            // The cap is the subject here, so one sampled peer may contribute alone.
            suprnova::RecommendConfig {
                peers: 1,
                min_peers: 1,
                ..Default::default()
            },
            async {
                Recommend::generate(GenerateRun::default()).await.unwrap();
            },
        )
        .await
        .unwrap();
        let rows = Recommend::for_user("alice").limit(1).get().await.unwrap();
        assert_eq!(rows[0].target, key(&newest));
        assert_eq!(rows[0].explanation.peer_count, 1);
    })
    .await;
}

#[tokio::test]
async fn private_shared_titles_are_projected_out_and_evidence_retains_provenance() {
    setup(async {
        let shared = note("Shared secret title", "rust", true, "alice", 1).await;
        let candidate = note("Public candidate", "other", true, "author", 2).await;
        record("alice", &shared, EventType::Like, "shared").await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &shared, EventType::Like, "shared").await;
            record(peer, &candidate, EventType::Like, "candidate").await;
        }
        Recommend::generate(GenerateRun::default()).await.unwrap();
        DB::update(
            "UPDATE recommendation_notes SET public = ? WHERE id = ?",
            vec![false.into(), shared.id.into()],
        )
        .await
        .unwrap();
        let results = Recommend::for_user("alice")
            .for_viewer(Viewer::Guest)
            .get()
            .await
            .unwrap();
        assert_eq!(results[0].target, key(&candidate));
        assert_eq!(results[0].explanation.shared_target_count, 1);
        assert!(results[0].explanation.shared_target_titles.is_empty());
        let rows = DB::select("SELECT event_ids FROM recommendation_explanations", [])
            .await
            .unwrap();
        assert!(!rows.is_empty());
        for row in rows {
            let refs: Vec<suprnova::EventProvenance> =
                serde_json::from_str(&row.get_string("event_ids").unwrap()).unwrap();
            for reference in refs {
                assert_eq!(
                    Recommend::verify_provenance(&reference).await.unwrap(),
                    suprnova::ProvenanceState::Intact
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn filtered_generation_keeps_other_actors_and_does_not_skip_the_global_delta() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let c = note("C", "other", true, "author", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        record("bob", &a, EventType::Like, "a").await;
        let GenerationOutcome::Completed(first) =
            Recommend::generate(GenerateRun::default()).await.unwrap()
        else {
            panic!("lock")
        };
        let before = Recommend::for_user("bob").get().await.unwrap();
        record("alice", &c, EventType::Like, "c").await;
        let GenerationOutcome::Completed(filtered) = Recommend::generate(GenerateRun {
            user: Some("alice".into()),
            surface: Some("home".into()),
        })
        .await
        .unwrap() else {
            panic!("lock")
        };
        assert_eq!(filtered.high_water_mark, first.high_water_mark);
        let after = Recommend::for_user("bob").get().await.unwrap();
        assert_eq!(
            after.iter().map(|r| &r.target).collect::<Vec<_>>(),
            before.iter().map(|r| &r.target).collect::<Vec<_>>()
        );
        assert!(
            after
                .iter()
                .all(|r| r.run_id.as_ref() == Some(&filtered.id))
        );
        let GenerationOutcome::Completed(full) =
            Recommend::generate(GenerateRun::default()).await.unwrap()
        else {
            panic!("lock")
        };
        assert_eq!(full.new_events, 1);
        assert_eq!(full.high_water_mark, first.high_water_mark + 1);
    })
    .await;
}

#[tokio::test]
async fn affinity_does_not_reload_seeds_absent_from_the_search_engine() {
    setup(async {
        let a = note("Removed seed", "rust", true, "alice", 1).await;
        let matching = note("Matching category", "rust", true, "author", 2).await;
        let recent = note("Newest document", "other", true, "author", 100).await;
        record("alice", &a, EventType::Like, "a").await;
        let descriptor = <Note as suprnova::Searchable>::descriptor();
        let engine = suprnova::SearchRegistry::engine_for(descriptor.name).unwrap();
        let generation = suprnova::SearchRegistry::generation_for(descriptor.name)
            .await
            .unwrap();
        engine
            .remove(descriptor, &generation, &[(a.id.to_string(), i64::MAX)])
            .await
            .unwrap();
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let results = Recommend::for_user("alice").get().await.unwrap();
        assert_eq!(
            results
                .iter()
                .map(|row| row.target.clone())
                .collect::<Vec<_>>(),
            vec![key(&recent), key(&matching)]
        );
        assert!(
            results
                .iter()
                .all(|row| row.explanation.fallback_tier == "recent")
        );
    })
    .await;
}

/// No suggestion may name fewer than three peers: one peer's explanation would point at them.
fn assert_no_lone_peers(rows: &[suprnova::Recommendation]) {
    for row in rows {
        let explanation = &row.explanation;
        assert!(
            explanation.fallback_tier != "collaborative" || explanation.peer_count >= 3,
            "{explanation:?}"
        );
    }
}

#[tokio::test]
async fn rec_002_a_candidate_one_peer_holds_falls_to_a_later_tier() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let b = note("B", "rust", true, "alice", 2).await;
        let c = note("C", "other", true, "alice", 3).await;
        for (actor, item, dedupe) in [
            ("alice", &a, "a"),
            ("alice", &b, "b"),
            ("bob", &a, "a"),
            ("bob", &b, "b"),
            ("bob", &c, "c"),
        ] {
            record(actor, item, EventType::Like, dedupe).await;
        }
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let rows = Recommend::for_user("alice").limit(5).get().await.unwrap();
        assert_no_lone_peers(&rows);
        let suggested = rows.iter().find(|row| row.target == key(&c)).unwrap();
        assert_ne!(suggested.explanation.fallback_tier, "collaborative");
        assert_eq!(suggested.explanation.peer_count, 0);
    })
    .await;
}

#[tokio::test]
async fn rec_002_three_agreeing_peers_make_a_collaborative_candidate() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let b = note("B", "rust", true, "alice", 2).await;
        let c = note("C", "other", true, "alice", 3).await;
        record("alice", &a, EventType::Like, "a").await;
        record("alice", &b, EventType::Like, "b").await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &b, EventType::Like, "b").await;
            record(peer, &c, EventType::Like, "c").await;
        }
        Recommend::generate(GenerateRun::default()).await.unwrap();
        let rows = Recommend::for_user("alice").limit(1).get().await.unwrap();
        assert_eq!(rows[0].target, key(&c));
        assert_eq!(rows[0].explanation.fallback_tier, "collaborative");
        assert_eq!(rows[0].explanation.peer_count, 3);
    })
    .await;
}

#[tokio::test]
async fn rec_002_similar_to_counts_co_occurrence_only_from_three_peers() {
    setup(async {
        let seed = note("Seed", "rust", true, "alice", 1).await;
        let lone = note("Lone", "other", true, "alice", 2).await;
        let agreed = note("Agreed", "other", true, "alice", 3).await;
        record("bob", &seed, EventType::Like, "seed").await;
        record("bob", &lone, EventType::Like, "lone").await;
        for peer in ["carol", "dave", "erin"] {
            record(peer, &seed, EventType::Like, "seed").await;
            record(peer, &agreed, EventType::Like, "agreed").await;
        }
        let rows = Recommend::similar_to(key(&seed))
            .limit(5)
            .for_viewer(Viewer::Guest)
            .get()
            .await
            .unwrap();
        assert_no_lone_peers(&rows);
        let first = &rows[0];
        assert_eq!(first.target, key(&agreed));
        assert_eq!(first.explanation.peer_count, 3);
        assert!(
            rows.iter()
                .filter(|row| row.target == key(&lone))
                .all(|row| row.explanation.fallback_tier != "collaborative")
        );
    })
    .await;
}

#[test]
fn rec_002_the_peer_threshold_is_positive() {
    let refused = suprnova::RecommendConfig {
        min_peers: 0,
        ..Default::default()
    }
    .validate()
    .unwrap_err();
    assert!(refused.to_string().contains("RECOMMEND_MIN_PEERS"));
    assert_eq!(suprnova::RecommendConfig::default().min_peers, 3);
}
