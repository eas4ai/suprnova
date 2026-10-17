//! The graph engine's reads (REC-007) and its discover query (REC-006).

use super::support::*;
use suprnova::{
    DatabaseGraphStore, EventType, ExplanationEvidence, GraphStore, GraphSurface, Recommend,
    RecommendConfig, Recommendation, Viewer,
};

fn graph(change: impl FnOnce(&mut RecommendConfig)) -> RecommendConfig {
    let mut config = RecommendConfig {
        engine: "graph".into(),
        public_event_types: ["like".to_owned()].into(),
        ..Default::default()
    };
    change(&mut config);
    config
}

async fn with<F: std::future::Future<Output = ()>>(config: RecommendConfig, future: F) {
    let _queue = suprnova::Queue::fake();
    RecommendConfig::scope(config, future).await.unwrap();
}

/// Peers who each like every note in `notes`, so every pair becomes a shared edge.
async fn peers_like(peers: &[&str], notes: &[&Note]) {
    for peer in peers {
        for note in notes {
            record(peer, note, EventType::Like, &format!("n{}", note.id)).await;
        }
    }
}

async fn for_alice(viewer: Viewer) -> Vec<Recommendation> {
    Recommend::for_user("alice")
        .limit(10)
        .for_viewer(viewer)
        .get()
        .await
        .unwrap()
}

fn targets(rows: &[Recommendation]) -> Vec<String> {
    rows.iter().map(|row| row.target.key.clone()).collect()
}

#[tokio::test]
async fn rec_007_a_fact_outside_the_viewer_scope_never_influences_a_score() {
    setup(with(graph(|_| {}), async {
        let a = note("A", "a", true, "o", 1).await;
        let private = note("P", "p", false, "bob", 2).await;
        let beyond = note("C", "c", true, "o", 3).await;
        record("alice", &a, EventType::Like, "a").await;
        peers_like(&["p1", "p2", "p3"], &[&a, &private]).await;
        peers_like(&["q1", "q2", "q3"], &[&private, &beyond]).await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        // Alice cannot see the private note, so nothing reached through it counts.
        let rows = for_alice(viewer("alice")).await;
        let reached = targets(&rows);
        assert!(!reached.contains(&private.id.to_string()));
        assert!(rows.iter().all(
            |row| row.explanation.engine != "graph" || row.target.key != beyond.id.to_string()
        ));
        // Its owner sees the private note and what lies beyond it.
        let owner = for_alice(viewer("bob")).await;
        let reached = targets(&owner);
        assert!(reached.contains(&private.id.to_string()), "{reached:?}");
        assert!(reached.contains(&beyond.id.to_string()), "{reached:?}");
        assert!(owner.iter().all(|row| row.explanation.engine == "graph"));
    }))
    .await;
}

#[tokio::test]
async fn rec_007_more_hide_and_report_events_lower_an_otherwise_equal_target() {
    setup(with(graph(|_| {}), async {
        let a = note("A", "a", true, "o", 1).await;
        // Created first, so the key tie-break alone would put this one first.
        let reported = note("Reported", "r", true, "o", 5).await;
        let clean = note("Clean", "k", true, "o", 5).await;
        record("alice", &a, EventType::Like, "a").await;
        peers_like(&["p1", "p2", "p3"], &[&a, &reported, &clean]).await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let before = targets(&for_alice(viewer("alice")).await);
        assert_eq!(before.first(), Some(&reported.id.to_string()));
        record("h1", &reported, EventType::Hide, "r").await;
        record("h2", &reported, EventType::Report, "r").await;
        let after = targets(&for_alice(viewer("alice")).await);
        let at = |key: &str| after.iter().position(|k| k == key).unwrap();
        assert!(
            at(&clean.id.to_string()) < at(&reported.id.to_string()),
            "{after:?}"
        );
    }))
    .await;
}

#[tokio::test]
async fn rec_007_followed_actors_add_social_proximity_only_when_three_agree() {
    let config = graph(|config| config.actor_index = Some("recommendation_notes".into()));
    setup(with(config, async {
        // Notes stand in for profiles: a note's key names the user it is.
        let profiles = [
            note("U1", "u", true, "o", 1).await,
            note("U2", "u", true, "o", 1).await,
            note("U3", "u", true, "o", 1).await,
        ];
        let agreed = note("Agreed", "x", true, "o", 2).await;
        let pair = note("Pair", "y", true, "o", 2).await;
        for profile in &profiles {
            record(
                "alice",
                profile,
                EventType::Follow,
                &format!("f{}", profile.id),
            )
            .await;
            record(&profile.id.to_string(), &agreed, EventType::Like, "agreed").await;
        }
        for profile in &profiles[..2] {
            record(&profile.id.to_string(), &pair, EventType::Like, "pair").await;
        }
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        let social = rows
            .iter()
            .find(|row| row.target == key(&agreed))
            .expect("three agreeing followed actors put the target in the list");
        assert_eq!(social.explanation.engine, "graph");
        assert_eq!(social.explanation.peer_count, 3);
        assert!(
            rows.iter()
                .filter(|row| row.target == key(&pair))
                .all(|row| row.explanation.peer_count == 0)
        );
        // A public engagement edge is cited, never the followed actors themselves.
        let text = serde_json::to_string(&social.explanation).unwrap();
        assert!(
            !profiles
                .iter()
                .any(|p| text.contains(&format!("\"{}\"", p.id)))
        );
    }))
    .await;
}

#[tokio::test]
async fn rec_007_a_list_never_holds_more_than_the_cap_of_one_facet_value() {
    let config = graph(|config| {
        config.graph_surfaces.insert(
            "home".into(),
            GraphSurface {
                per_value: 2,
                ..Default::default()
            },
        );
    });
    setup(with(config, async {
        let a = note("A", "seed", true, "o", 1).await;
        let mut same = Vec::new();
        for i in 0..5 {
            same.push(note(&format!("Same {i}"), "same", true, "o", 2).await);
        }
        let other = note("Other", "other", true, "o", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        let mut liked: Vec<&Note> = vec![&a, &other];
        liked.extend(same.iter());
        peers_like(&["p1", "p2", "p3"], &liked).await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        let in_same = rows
            .iter()
            .filter(|row| same.iter().any(|note| row.target == key(note)))
            .count();
        assert_eq!(in_same, 2, "{:?}", targets(&rows));
        assert!(rows.iter().any(|row| row.target == key(&other)));
    }))
    .await;
}

#[tokio::test]
async fn rec_007_a_low_confidence_surface_answers_with_the_starter_rows() {
    setup(with(graph(|_| {}), async {
        let a = note("A", "a", true, "o", 1).await;
        note("B", "b", true, "o", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        // No peers share anything: the graph has no path from Alice's seed.
        Recommend::generate(suprnova::GenerateRun::default())
            .await
            .unwrap();
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        assert!(!rows.is_empty());
        for row in &rows {
            assert_eq!(row.explanation.engine, "starter");
            assert!(row.explanation.fallback_reason.is_some());
        }
    }))
    .await;
}

#[tokio::test]
async fn rec_007_every_graph_explanation_cites_permitted_stored_evidence() {
    setup(with(graph(|_| {}), async {
        let a = note("A", "a", true, "o", 1).await;
        let b = note("B", "b", true, "o", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        peers_like(&["p1", "p2", "p3"], &[&a, &b]).await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        let row = rows.iter().find(|row| row.target == key(&b)).unwrap();
        assert_eq!(row.explanation.engine, "graph");
        assert!(!row.explanation.evidence.is_empty());
        let store = DatabaseGraphStore;
        let generation = store.current_generation().await.unwrap().unwrap();
        for evidence in &row.explanation.evidence {
            match evidence {
                ExplanationEvidence::Event(event) => {
                    assert_eq!(
                        Recommend::verify_provenance(event).await.unwrap(),
                        suprnova::ProvenanceState::Intact
                    );
                    let actor = suprnova::DB::select(
                        "SELECT actor_id FROM recommendation_events WHERE id = ?",
                        vec![event.id.into()],
                    )
                    .await
                    .unwrap();
                    assert_eq!(actor[0].get_string("actor_id").unwrap(), "alice");
                }
                ExplanationEvidence::Edge { key: edge, kind } => {
                    let hops = store
                        .outgoing(
                            &generation,
                            &[suprnova::recommendations::graph::document_node_key(&key(
                                &a,
                            ))],
                            std::slice::from_ref(kind),
                            50,
                        )
                        .await
                        .unwrap();
                    assert!(hops.iter().any(|hop| &hop.key == edge));
                }
                ExplanationEvidence::Document(target) => {
                    assert_eq!(target, &key(&a));
                }
            }
        }
    }))
    .await;
}

#[tokio::test]
async fn rec_006_the_discover_query_needs_the_graph_engine_and_steers_by_the_viewer_events() {
    setup(async {
        let a = note("A", "a", true, "o", 1).await;
        let b = note("B", "b", true, "o", 2).await;
        peers_like(&["p1", "p2", "p3"], &[&a, &b]).await;
        let refused = Recommend::discover(key(&a))
            .for_viewer(viewer("alice"))
            .get()
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("RECOMMEND_ENGINE=graph"));
        with(graph(|_| {}), async {
            Recommend::rebuild_graph().await.unwrap().unwrap();
            let rows = Recommend::discover(key(&a))
                .for_viewer(viewer("alice"))
                .limit(5)
                .get()
                .await
                .unwrap();
            assert_eq!(targets(&rows), [b.id.to_string()]);
            let hidden = note("Hidden", "h", false, "bob", 3).await;
            let refused = Recommend::discover(key(&hidden))
                .for_viewer(viewer("alice"))
                .get()
                .await
                .unwrap_err();
            assert!(refused.to_string().contains("outside the viewer's scope"));
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn rec_007_a_list_never_holds_more_than_the_cap_reached_through_one_node() {
    let config = graph(|config| {
        config.graph_surfaces.insert(
            "home".into(),
            GraphSurface {
                per_node: 2,
                per_value: 10,
                ..Default::default()
            },
        );
    });
    setup(with(config, async {
        let a = note("A", "seed", true, "o", 1).await;
        let hub = note("Hub", "hub", true, "o", 2).await;
        let mut spokes = Vec::new();
        for i in 0..5 {
            spokes.push(note(&format!("Spoke {i}"), &format!("s{i}"), true, "o", 3).await);
        }
        record("alice", &a, EventType::Like, "a").await;
        peers_like(&["p1", "p2", "p3"], &[&a, &hub]).await;
        let mut around: Vec<&Note> = vec![&hub];
        around.extend(spokes.iter());
        peers_like(&["q1", "q2", "q3"], &around).await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        let through_hub = rows
            .iter()
            .filter(|row| spokes.iter().any(|spoke| row.target == key(spoke)))
            .count();
        assert_eq!(through_hub, 2, "{:?}", targets(&rows));
        assert!(rows.iter().any(|row| row.target == key(&hub)));
    }))
    .await;
}

#[tokio::test]
async fn rec_007_a_list_never_holds_more_than_the_cap_of_the_popular_band() {
    let config = graph(|config| {
        config.graph_surfaces.insert(
            "home".into(),
            GraphSurface {
                per_band: 1,
                per_value: 10,
                ..Default::default()
            },
        );
    });
    setup(with(config, async {
        let a = note("A", "seed", true, "o", 1).await;
        let mut popular = Vec::new();
        for i in 0..3 {
            popular.push(note(&format!("Popular {i}"), &format!("p{i}"), true, "o", 2).await);
        }
        let quiet = note("Quiet", "q", true, "o", 2).await;
        record("alice", &a, EventType::Like, "a").await;
        let mut liked: Vec<&Note> = vec![&a];
        liked.extend(popular.iter());
        peers_like(&["p1", "p2", "p3", "p4", "p5", "p6"], &liked).await;
        peers_like(&["q1", "q2", "q3"], &[&a, &quiet]).await;
        // Single likes of other notes spread popularity, so the top tenth is narrow.
        for i in 0..7 {
            let filler = note(&format!("Filler {i}"), &format!("f{i}"), true, "o", 1).await;
            record(&format!("f{i}"), &filler, EventType::Like, "filler").await;
        }
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let rows = for_alice(viewer("alice")).await;
        let banded = rows
            .iter()
            .filter(|row| popular.iter().any(|note| row.target == key(note)))
            .count();
        assert_eq!(banded, 1, "{:?}", targets(&rows));
        assert!(rows.iter().any(|row| row.target == key(&quiet)));
    }))
    .await;
}

/// REC-009: the same events and relations, built into the database store and into
/// Memgraph, give the same ranked lists, because the framework scores what either store
/// returns.
#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_both_stores_rank_the_same_fixture_alike_memgraph() {
    let url = std::env::var("MEMGRAPH_TEST_URL").expect("MEMGRAPH_TEST_URL is required");
    let graph = neo4rs::Graph::connect(
        neo4rs::ConfigBuilder::default()
            .uri(&url)
            .user("")
            .password("")
            .db("memgraph")
            .build()
            .unwrap(),
    )
    .await
    .unwrap();
    graph
        .run(neo4rs::query("MATCH (n) DETACH DELETE n"))
        .await
        .unwrap();
    setup(with(graph_config_for("database", None), async {
        let a = note("A", "a", true, "o", 1).await;
        let b = note("B", "b", true, "o", 2).await;
        let c = note("C", "c", true, "o", 3).await;
        let private = note("P", "p", false, "bob", 4).await;
        let d = note("D", "d", true, "o", 5).await;
        record("alice", &a, EventType::Like, "a").await;
        peers_like(&["p1", "p2", "p3"], &[&a, &b, &c]).await;
        peers_like(&["q1", "q2", "q3"], &[&a, &private]).await;
        peers_like(&["r1", "r2", "r3"], &[&private, &d]).await;
        record("h1", &c, EventType::Hide, "c").await;
        Recommend::rebuild_graph().await.unwrap().unwrap();
        let mut by_store = Vec::new();
        for viewer_id in ["alice", "bob"] {
            by_store.push(ranked(viewer_id).await);
        }
        // A comparison of empty or fallback lists would prove nothing.
        assert!(
            by_store
                .iter()
                .all(|rows| rows.iter().any(|row| row.2 == "graph"))
        );
        let memgraph = graph_config_for("memgraph", Some(url.clone()));
        RecommendConfig::scope(memgraph, async {
            Recommend::rebuild_graph().await.unwrap().unwrap();
            for (i, viewer_id) in ["alice", "bob"].into_iter().enumerate() {
                assert_eq!(ranked(viewer_id).await, by_store[i], "viewer {viewer_id}");
            }
        })
        .await
        .unwrap();
    }))
    .await;
}

#[cfg(feature = "graph-memgraph")]
fn graph_config_for(store: &str, url: Option<String>) -> RecommendConfig {
    graph(|config| {
        config.graph_store = store.into();
        config.graph_url = url;
    })
}

#[cfg(feature = "graph-memgraph")]
async fn ranked(viewer_id: &str) -> Vec<(String, i64, String)> {
    for_alice(viewer(viewer_id))
        .await
        .into_iter()
        .map(|row| (row.target.key, row.score, row.explanation.engine))
        .collect()
}
