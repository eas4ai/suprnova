use suprnova::{EventClass, EventType, RecommendConfig};

#[test]
fn every_event_has_a_declared_class_and_score() {
    let config = RecommendConfig::default();
    for (kind, base) in [
        (EventType::View, 1000),
        (EventType::Consume, 1000),
        (EventType::Complete, 1200),
        (EventType::Comment, 1200),
        (EventType::Like, 1400),
        (EventType::Share, 1500),
        (EventType::Favorite, 1600),
        (EventType::Purchase, 2000),
        (EventType::Follow, 800),
        (EventType::Click, 300),
        (EventType::Custom("playlist_add".into()), 500),
    ] {
        let scoring = kind.scoring(100, &config).unwrap();
        assert_eq!(scoring.class, EventClass::Positive);
        assert_eq!(scoring.score, base + 5);
    }
    assert_eq!(
        EventType::Impression.scoring(100, &config).unwrap().class,
        EventClass::Seen
    );
    for kind in [EventType::Skip, EventType::Hide, EventType::Report] {
        assert_eq!(
            kind.scoring(0, &config).unwrap().class,
            EventClass::Negative
        );
    }
}

use super::support::*;
use suprnova::{Actor, DB, ProvenanceState, Recommend};

#[tokio::test]
async fn duplicate_is_immutable_and_provenance_detects_changes_and_retention() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let event = event("alice", &a, EventType::Like, "same").await;
        let expected_digest = suprnova::event_digest(
            &event,
            event
                .event_type
                .scoring(event.weight, &RecommendConfig::default())
                .unwrap(),
        )
        .unwrap();
        let first = Recommend::record(event.clone()).await.unwrap();
        assert_eq!(first.digest, expected_digest);
        let mut changed = event;
        changed.weight = 200;
        changed.event_type = EventType::Hide;
        let second = Recommend::record(changed).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_events", [])
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            Recommend::verify_provenance(&first).await.unwrap(),
            ProvenanceState::Intact
        );
        DB::update(
            "UPDATE recommendation_events SET weight = ? WHERE id = ?",
            vec![300.into(), first.id.into()],
        )
        .await
        .unwrap();
        assert_eq!(
            Recommend::verify_provenance(&first).await.unwrap(),
            ProvenanceState::Altered
        );
        DB::delete(
            "DELETE FROM recommendation_events WHERE id = ?",
            vec![first.id.into()],
        )
        .await
        .unwrap();
        assert_eq!(
            Recommend::verify_provenance(&first).await.unwrap(),
            ProvenanceState::Removed
        );
    })
    .await;
}

#[tokio::test]
async fn incomplete_facts_are_refused_and_actor_dedupe_is_separate() {
    setup(async {
        let a = note("A", "rust", true, "alice", 1).await;
        let valid = event("alice", &a, EventType::View, "key").await;
        for field in ["target", "scope", "version"] {
            let mut broken = valid.clone();
            match field {
                "target" => broken.target = None,
                "scope" => broken.access_scope = None,
                _ => broken.source_version = None,
            }
            assert!(Recommend::record(broken).await.is_err(), "{field}");
        }
        Recommend::record(valid).await.unwrap();
        record("bob", &a, EventType::View, "key").await;
        assert_eq!(
            DB::scalar::<i64>("SELECT COUNT(*) FROM recommendation_events", [])
                .await
                .unwrap(),
            2
        );
        let guest = Actor::guest("secret session").unwrap();
        assert!(
            matches!(guest, Actor::Guest(hash) if hash.len() == 64 && !hash.contains("secret"))
        );
    })
    .await;
}

#[test]
fn custom_scoring_overrides_are_typed_and_defaults_are_bounded() {
    let mut config = RecommendConfig::default();
    assert_eq!(
        (
            config.history,
            config.peers,
            config.popularity_window,
            config.lock_ttl_secs
        ),
        (500, 200, 30, 3600)
    );
    config.event_types.insert(
        "dislike".into(),
        suprnova::EventScoring {
            class: EventClass::Negative,
            score: -700,
        },
    );
    assert_eq!(
        EventType::Custom("dislike".into())
            .scoring(40, &config)
            .unwrap()
            .score,
        -698
    );
    assert!(EventType::Custom("".into()).scoring(0, &config).is_err());
}

#[test]
fn lock_ttl_cannot_overflow_the_cache_clock() {
    let config = RecommendConfig {
        lock_ttl_secs: i64::MAX as u64,
        ..Default::default()
    };
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("RECOMMEND_LOCK_TTL_SECS")
    );
}

#[tokio::test]
async fn retention_and_popularity_windows_cannot_overflow_the_calendar() {
    let config = RecommendConfig {
        popularity_window: u32::MAX,
        ..Default::default()
    };
    assert!(config.validate().is_err());
    assert!(Recommend::prune(u32::MAX).await.is_err());
}

#[tokio::test]
async fn access_scope_records_each_targets_own_scope_values() {
    use super::support::{event, note, setup};
    use suprnova::Recommend;
    setup(async {
        let open = note("Open", "fruit", true, "", 1).await;
        let closed = note("Closed", "fruit", false, "alice", 2).await;
        let open_event = event("reader", &open, EventType::View, "open").await;
        let closed_event = event("reader", &closed, EventType::View, "closed").await;
        assert_ne!(open_event.access_scope, closed_event.access_scope);
        let values = &closed_event.access_scope.as_ref().unwrap()["values"];
        assert!(values.get("public").is_some() && values.get("owner").is_some(), "{values}");
        let mut forged = event("forger", &closed, EventType::View, "forged").await;
        forged.access_scope = open_event.access_scope.clone();
        assert!(Recommend::record(forged).await.is_err());
        Recommend::record(closed_event).await.unwrap();
    })
    .await;
}
