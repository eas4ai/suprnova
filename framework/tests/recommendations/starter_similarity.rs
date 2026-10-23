use super::support::*;
use suprnova::{Actor, EventType, Recommend, RecommendConfig, RecommendationEvent, Viewer};

#[tokio::test]
async fn similarity_peer_selection_preserves_actor_ties_and_capped_histories() {
    setup(async {
        let seed = note("Seed", "seed", true, "author", 1).await;
        let selected = note("Selected", "user", true, "author", 2).await;
        let guest_only = note("Guest only", "guest", true, "author", 3).await;
        let stale = note("Stale seed", "stale", true, "author", 4).await;
        let extra = note("Extra", "extra", true, "author", 5).await;
        for (actor, targets) in [
            (Actor::User("Z".into()), vec![&seed, &selected]),
            (Actor::User("a".into()), vec![&seed, &guest_only]),
            (Actor::guest("session").unwrap(), vec![&seed, &guest_only]),
            (Actor::User("stale".into()), vec![&seed, &stale, &extra]),
        ] {
            for (position, target) in targets.into_iter().enumerate() {
                let mut event = RecommendationEvent::new(
                    EventType::Like,
                    key(target),
                    Some(actor.clone()),
                    "home",
                    format!("{position}"),
                )
                .await
                .unwrap();
                event.occurred_at =
                    chrono::DateTime::from_timestamp(1_000 + position as i64, 0).unwrap();
                Recommend::record(event).await.unwrap();
            }
        }
        RecommendConfig::scope(
            RecommendConfig {
                history: 2,
                peers: 1,
                min_peers: 1,
                ..Default::default()
            },
            async {
                let rows = Recommend::similar_to(key(&seed))
                    .for_viewer(Viewer::Guest)
                    .get()
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].target, key(&selected));
                assert_eq!(rows[0].score, 1400);
                assert_eq!(rows[0].explanation.peer_count, 1);
            },
        )
        .await
        .unwrap();
    })
    .await;
}
