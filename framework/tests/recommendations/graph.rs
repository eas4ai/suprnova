//! The graph store and its scoped traversal (REC-009), on the application's database.

use super::support::*;
use std::collections::BTreeSet;
use suprnova::{
    AttributeValue, EventProvenance, EventType, Model, RecommendationTarget, Viewer,
    recommendations::graph::{
        DatabaseGraphStore, GraphEdge, GraphNode, GraphProvenance, GraphScope, GraphStore,
        Revocation, Traversal, document_node_key, edge_key, traverse,
    },
};

fn target(key: &str) -> RecommendationTarget {
    RecommendationTarget {
        index: "recommendation_notes".into(),
        key: key.into(),
    }
}

fn scope(public: bool, owner: &str) -> GraphScope {
    GraphScope {
        index: Some("recommendation_notes".into()),
        values: [
            ("owner".to_owned(), AttributeValue::Text(owner.into())),
            ("public".to_owned(), AttributeValue::Bool(public)),
        ]
        .into(),
    }
}

fn event(id: i64) -> GraphProvenance {
    GraphProvenance::Event(EventProvenance {
        id,
        digest: format!("{id:064}"),
    })
}

fn node(key: &str, public: bool, owner: &str, version: i64, from: i64) -> GraphNode {
    let target = target(key);
    GraphNode {
        key: document_node_key(&target),
        kind: "recommendation_notes".into(),
        target: Some(target),
        scope: scope(public, owner),
        source_version: version,
        provenance: event(from),
    }
}

fn edge(
    kind: &str,
    from: &GraphNode,
    to: &GraphNode,
    confidence: i32,
    provenance: GraphProvenance,
) -> GraphEdge {
    // A relation edge carries the version it was read from; an event edge its target's.
    let source_version = match &provenance {
        GraphProvenance::Relation { source_version, .. } => *source_version,
        GraphProvenance::Event(_) => to.source_version,
    };
    GraphEdge {
        key: edge_key(kind, &from.key, &to.key),
        source: from.key.clone(),
        target: to.key.clone(),
        kind: kind.into(),
        confidence_bps: confidence,
        scope: to.scope.clone(),
        source_version,
        provenance,
        created_at: 1,
        updated_at: 1,
    }
}

fn relation(from: &GraphNode, version: i64) -> GraphProvenance {
    GraphProvenance::Relation {
        name: "series".into(),
        source: from.target.clone().unwrap(),
        source_version: version,
    }
}

fn walk(seed: &GraphNode, depth: usize) -> Traversal {
    Traversal {
        seeds: vec![seed.key.clone()],
        depth,
        per_node: 10,
        max_nodes: 100,
        kinds: vec![],
    }
}

fn reached_keys(reached: &[suprnova::recommendations::graph::Reached]) -> BTreeSet<String> {
    reached.iter().map(|item| item.node.key.clone()).collect()
}

/// Conformance: a store answers hops strongest first within its fan out.
pub async fn a_store_answers_hops_strongest_first_within_its_fan_out(store: &dyn GraphStore) {
    let generation = store.begin_generation().await.unwrap();
    let [a, b, c, d] = ["a", "b", "c", "d"].map(|key| node(key, true, "x", 1, 1));
    store
        .put_nodes(&generation, &[a.clone(), b.clone(), c.clone(), d.clone()])
        .await
        .unwrap();
    store
        .put_edges(
            &generation,
            &[
                edge("co_engaged", &a, &b, 9_000, event(1)),
                edge("co_engaged", &a, &c, 5_000, event(1)),
                edge("relation:series", &a, &d, 7_000, relation(&a, 1)),
            ],
        )
        .await
        .unwrap();
    let top = store
        .outgoing(&generation, std::slice::from_ref(&a.key), &[], 2)
        .await
        .unwrap();
    assert_eq!(
        top.iter()
            .map(|edge| edge.target.clone())
            .collect::<Vec<_>>(),
        [b.key.clone(), d.key.clone()]
    );
    let relations = store
        .outgoing(
            &generation,
            std::slice::from_ref(&a.key),
            &["relation:series".into()],
            10,
        )
        .await
        .unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].provenance, relation(&a, 1));
    let refused = store
        .put_edges(&generation, &[edge("co_engaged", &b, &c, 10_001, event(1))])
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("10000"));
}

#[tokio::test]
async fn rec_009_a_store_answers_hops_strongest_first_within_its_fan_out() {
    setup(a_store_answers_hops_strongest_first_within_its_fan_out(
        &DatabaseGraphStore,
    ))
    .await;
}

/// Conformance: a traversal never returns or walks through a fact outside the viewer scope.
pub async fn a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope(
    store: &dyn GraphStore,
) {
    let generation = store.begin_generation().await.unwrap();
    let a = node("a", true, "x", 1, 1);
    let private = node("p", false, "bob", 1, 1);
    let beyond = node("c", true, "x", 1, 1);
    let hidden_edge_target = node("r", true, "x", 1, 1);
    store
        .put_nodes(
            &generation,
            &[
                a.clone(),
                private.clone(),
                beyond.clone(),
                hidden_edge_target.clone(),
            ],
        )
        .await
        .unwrap();
    let mut hidden = edge("co_engaged", &a, &hidden_edge_target, 9_000, event(1));
    hidden.scope = scope(false, "bob");
    store
        .put_edges(
            &generation,
            &[
                edge("co_engaged", &a, &private, 9_000, event(1)),
                edge("co_engaged", &private, &beyond, 9_000, event(1)),
                hidden,
            ],
        )
        .await
        .unwrap();
    let guest = traverse(store, &generation, &walk(&a, 3), &Viewer::Guest)
        .await
        .unwrap();
    assert!(guest.is_empty(), "{:?}", reached_keys(&guest));
    let owner = traverse(store, &generation, &walk(&a, 3), &viewer("bob"))
        .await
        .unwrap();
    assert_eq!(
        reached_keys(&owner),
        [
            private.key.clone(),
            beyond.key.clone(),
            hidden_edge_target.key.clone()
        ]
        .into()
    );
    let private_seed = traverse(store, &generation, &walk(&private, 3), &Viewer::Guest)
        .await
        .unwrap();
    assert!(private_seed.is_empty());
}

#[tokio::test]
async fn rec_009_a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope() {
    setup(
        a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope(
            &DatabaseGraphStore,
        ),
    )
    .await;
}

/// Conformance: a traversal stops at its depth bound.
pub async fn a_traversal_stops_at_its_depth_bound(store: &dyn GraphStore) {
    let generation = store.begin_generation().await.unwrap();
    let chain = ["a", "b", "c", "d"].map(|key| node(key, true, "x", 1, 1));
    store.put_nodes(&generation, &chain).await.unwrap();
    let edges: Vec<GraphEdge> = chain
        .windows(2)
        .map(|pair| edge("co_engaged", &pair[0], &pair[1], 8_000, event(1)))
        .collect();
    store.put_edges(&generation, &edges).await.unwrap();
    let reached = traverse(store, &generation, &walk(&chain[0], 2), &Viewer::Guest)
        .await
        .unwrap();
    assert_eq!(
        reached_keys(&reached),
        [chain[1].key.clone(), chain[2].key.clone()].into()
    );
    assert!(reached.iter().all(|item| item.path.len() <= 2));
    let far = reached
        .iter()
        .find(|item| item.node.key == chain[2].key)
        .unwrap();
    assert!((far.weight - 0.64).abs() < 1e-9);
}

#[tokio::test]
async fn rec_009_a_traversal_stops_at_its_depth_bound() {
    setup(a_traversal_stops_at_its_depth_bound(&DatabaseGraphStore)).await;
}

/// Conformance: publishing switches readers in one step and keeps the previous generation.
pub async fn publishing_switches_readers_in_one_step_and_keeps_the_previous_generation(
    store: &dyn GraphStore,
) {
    let a = node("a", true, "x", 1, 1);
    let first = store.begin_generation().await.unwrap();
    store
        .put_nodes(&first, std::slice::from_ref(&a))
        .await
        .unwrap();
    assert_eq!(store.current_generation().await.unwrap(), None);
    store.publish(&first).await.unwrap();
    let second = store.begin_generation().await.unwrap();
    store
        .put_nodes(&second, std::slice::from_ref(&a))
        .await
        .unwrap();
    assert_eq!(
        store.current_generation().await.unwrap(),
        Some(first.clone())
    );
    store.publish(&second).await.unwrap();
    assert_eq!(
        store.current_generation().await.unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        store
            .nodes(&first, std::slice::from_ref(&a.key))
            .await
            .unwrap()
            .len(),
        1
    );
    let third = store.begin_generation().await.unwrap();
    store.publish(&third).await.unwrap();
    assert!(
        store
            .nodes(&first, std::slice::from_ref(&a.key))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .nodes(&second, std::slice::from_ref(&a.key))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(store.publish("no-such-generation").await.is_err());
}

#[tokio::test]
async fn rec_009_publishing_switches_readers_in_one_step_and_keeps_the_previous_generation() {
    setup(
        publishing_switches_readers_in_one_step_and_keeps_the_previous_generation(
            &DatabaseGraphStore,
        ),
    )
    .await;
}

/// Conformance: revocations remove the facts resting on a changed or removed source.
pub async fn revocations_remove_the_facts_resting_on_a_changed_or_removed_source(
    store: &dyn GraphStore,
) {
    let generation = store.begin_generation().await.unwrap();
    let a = node("a", true, "x", 3, 10);
    let b = node("b", true, "x", 1, 11);
    let c = node("c", true, "x", 1, 12);
    let d = node("d", true, "x", 1, 13);
    store
        .put_nodes(&generation, &[a.clone(), b.clone(), c.clone(), d.clone()])
        .await
        .unwrap();
    store
        .put_edges(
            &generation,
            &[
                edge("co_engaged", &a, &b, 6_000, event(20)),
                edge("relation:series", &a, &c, 6_000, relation(&a, 2)),
                edge("relation:series", &a, &d, 6_000, relation(&a, 3)),
                edge("co_engaged", &c, &d, 6_000, event(21)),
            ],
        )
        .await
        .unwrap();
    let generation_ref = &generation;
    let hops = |sources: Vec<String>| async move {
        store
            .outgoing(generation_ref, &sources, &[], 10)
            .await
            .unwrap()
    };
    // Retention removed event 20: the edge resting on it goes.
    store
        .revoke(&generation, &Revocation::Events(vec![20]))
        .await
        .unwrap();
    assert!(
        hops(vec![a.key.clone()])
            .await
            .iter()
            .all(|edge| edge.kind != "co_engaged")
    );
    // Document a reached version 3: what was read from version 2 goes, version 3 stays.
    store
        .revoke(
            &generation,
            &Revocation::DocumentBefore(a.target.clone().unwrap(), 3),
        )
        .await
        .unwrap();
    let from_a = hops(vec![a.key.clone()]).await;
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_a[0].target, d.key);
    // Document d left its index: its node and every edge touching it go.
    store
        .revoke(
            &generation,
            &Revocation::Document(d.target.clone().unwrap()),
        )
        .await
        .unwrap();
    assert!(
        store
            .nodes(&generation, std::slice::from_ref(&d.key))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(hops(vec![a.key.clone(), c.key.clone()]).await.is_empty());
    // Retention removed the event node b rests on: no edge is left pointing at it.
    store
        .revoke(&generation, &Revocation::Events(vec![11]))
        .await
        .unwrap();
    assert!(
        store
            .nodes(&generation, std::slice::from_ref(&b.key))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn rec_009_revocations_remove_the_facts_resting_on_a_changed_or_removed_source() {
    setup(revocations_remove_the_facts_resting_on_a_changed_or_removed_source(&DatabaseGraphStore))
        .await;
}

/// Conformance: the same facts give the same digest in any generation.
pub async fn the_same_facts_give_the_same_digest_in_any_generation(store: &dyn GraphStore) {
    let a = node("a", true, "x", 1, 1);
    let b = node("b", false, "bob", 1, 2);
    let link = edge("co_engaged", &a, &b, 4_000, event(3));
    let first = store.begin_generation().await.unwrap();
    store
        .put_nodes(&first, &[b.clone(), a.clone()])
        .await
        .unwrap();
    store
        .put_edges(&first, std::slice::from_ref(&link))
        .await
        .unwrap();
    let second = store.begin_generation().await.unwrap();
    store
        .put_nodes(&second, &[a.clone(), b.clone()])
        .await
        .unwrap();
    let mut later = link.clone();
    later.created_at = 99;
    later.updated_at = 99;
    store.put_edges(&second, &[later.clone()]).await.unwrap();
    assert_eq!(
        store.fact_digest(&first).await.unwrap(),
        store.fact_digest(&second).await.unwrap()
    );
    later.confidence_bps = 4_001;
    store.put_edges(&second, &[later]).await.unwrap();
    assert_ne!(
        store.fact_digest(&first).await.unwrap(),
        store.fact_digest(&second).await.unwrap()
    );
}

#[tokio::test]
async fn rec_009_the_same_facts_give_the_same_digest_in_any_generation() {
    setup(the_same_facts_give_the_same_digest_in_any_generation(
        &DatabaseGraphStore,
    ))
    .await;
}

#[tokio::test]
async fn rec_009_an_unset_store_is_the_application_database() {
    setup(async {
        assert_eq!(suprnova::RecommendConfig::default().graph_store, "database");
        let store = suprnova::recommendations::graph::store().await.unwrap();
        assert_eq!(store.name(), "database");
        let generation = store.begin_generation().await.unwrap();
        store.publish(&generation).await.unwrap();
        assert_eq!(store.current_generation().await.unwrap(), Some(generation));
    })
    .await;
}

#[test]
fn rec_009_graph_settings_are_validated_and_name_their_variable() {
    let config = |change: fn(&mut suprnova::RecommendConfig)| {
        let mut config = suprnova::RecommendConfig::default();
        change(&mut config);
        config.validate().unwrap_err().to_string()
    };
    assert!(config(|c| c.graph_store = "neo4j".into()).contains("RECOMMEND_GRAPH_STORE"));
    assert!(config(|c| c.engine = "magic".into()).contains("RECOMMEND_ENGINE"));
    assert!(config(|c| c.graph_depth = 5).contains("RECOMMEND_GRAPH_DEPTH"));
    assert!(
        config(|c| c.graph_url = Some("bolt://user:secret@host:7687".into()))
            .contains("without credentials")
    );
    assert!(config(|c| c.social_min_actors = 0).contains("RECOMMEND_SOCIAL_MIN_ACTORS"));
    assert!(
        config(|c| {
            c.graph_surfaces.insert(
                "home".into(),
                suprnova::GraphSurface {
                    threshold: 10_001,
                    ..Default::default()
                },
            );
        })
        .contains("home.threshold")
    );
}

// Derivation (REC-005) and rebuilds (REC-009) through the facade, on the database store.

fn graph_config() -> suprnova::RecommendConfig {
    suprnova::RecommendConfig {
        engine: "graph".into(),
        public_event_types: ["like".to_owned()].into(),
        ..Default::default()
    }
}

/// Run `future` with the graph engine configured and queued jobs recorded.
async fn in_graph<F: std::future::Future<Output = ()>>(future: F) {
    let _queue = suprnova::Queue::fake();
    suprnova::RecommendConfig::scope(graph_config(), future)
        .await
        .unwrap();
}

/// Run the graph jobs the fake queue recorded, as a worker would. A job run twice finds
/// its work done, so earlier jobs may run again.
async fn drain_graph_jobs() {
    for job in suprnova::queue::testing::pending_jobs() {
        if job.name.starts_with("Suprnova::RecommendationGraph") {
            suprnova::queue::worker::dispatch_by_name(&job.name, job.payload.clone())
                .await
                .unwrap();
        }
    }
}

async fn current() -> (DatabaseGraphStore, String) {
    let store = DatabaseGraphStore;
    let generation = store.current_generation().await.unwrap().unwrap();
    (store, generation)
}

async fn edges_from(note: &Note, kind: &str) -> Vec<GraphEdge> {
    let (store, generation) = current().await;
    store
        .outgoing(
            &generation,
            &[document_node_key(&key(note))],
            &[kind.to_owned()],
            50,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn rec_005_shared_engagement_becomes_an_edge_only_where_enough_actors_agree() {
    setup(in_graph(async {
        let a = note("A", "x", true, "o", 1).await;
        let b = note("B", "x", true, "o", 2).await;
        let c = note("C", "x", true, "o", 3).await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Purchase, "a").await;
            record(peer, &b, EventType::Purchase, "b").await;
        }
        record("erin", &a, EventType::Purchase, "a").await;
        record("erin", &c, EventType::Purchase, "c").await;
        let built = suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        assert!(built.nodes > 0 && built.edges > 0);
        let shared = edges_from(&a, "co_engaged").await;
        assert_eq!(shared.len(), 1, "{shared:?}");
        assert_eq!(shared[0].target, document_node_key(&key(&b)));
        assert!(shared[0].confidence_bps > 0);
        // Purchases are not public: no actor node or per-actor edge exists.
        let (store, generation) = current().await;
        assert!(
            store
                .nodes(
                    &generation,
                    &[suprnova::recommendations::graph::actor_node_key("bob")]
                )
                .await
                .unwrap()
                .is_empty()
        );
        // Every fact carries scope, version and provenance that traces to its source.
        let GraphProvenance::Event(evidence) = &shared[0].provenance else {
            panic!("shared engagement rests on an event")
        };
        assert_eq!(
            suprnova::Recommend::verify_provenance(evidence)
                .await
                .unwrap(),
            suprnova::ProvenanceState::Intact
        );
        assert_eq!(
            shared[0].scope.index.as_deref(),
            Some("recommendation_notes")
        );
        for node in store
            .nodes(
                &generation,
                &[document_node_key(&key(&a)), document_node_key(&key(&b))],
            )
            .await
            .unwrap()
        {
            assert!(node.scope.index.is_some());
            let GraphProvenance::Event(evidence) = &node.provenance else {
                panic!("a stream-named document rests on an event")
            };
            assert_eq!(
                suprnova::Recommend::verify_provenance(evidence)
                    .await
                    .unwrap(),
                suprnova::ProvenanceState::Intact
            );
        }
    }))
    .await;
}

#[tokio::test]
async fn rec_005_only_public_event_types_become_per_actor_facts() {
    setup(in_graph(async {
        let a = note("A", "x", true, "o", 1).await;
        let b = note("B", "x", true, "o", 2).await;
        record("bob", &a, EventType::Like, "a").await;
        record("bob", &b, EventType::Purchase, "b").await;
        suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        let (store, generation) = current().await;
        let actor = suprnova::recommendations::graph::actor_node_key("bob");
        let engaged = store
            .outgoing(
                &generation,
                std::slice::from_ref(&actor),
                &["engaged".into()],
                50,
            )
            .await
            .unwrap();
        assert_eq!(
            engaged
                .iter()
                .map(|edge| edge.target.clone())
                .collect::<Vec<_>>(),
            [document_node_key(&key(&a))]
        );
    }))
    .await;
}

/// A note whose category `next:<id>` names the note it continues into.
fn declare_next() {
    suprnova::Recommend::graph_relation(
        suprnova::recommendations::graph::GraphRelation::new(
            "next",
            "recommendation_notes",
            8_000,
            |document| {
                Ok(match document.attributes.get("category") {
                    Some(AttributeValue::Text(category)) => category
                        .strip_prefix("next:")
                        .map(|key| vec![target(key)])
                        .unwrap_or_default(),
                    _ => Vec::new(),
                })
            },
        )
        .unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn rec_005_a_newer_revision_replaces_the_relation_facts_read_from_an_older_one() {
    setup(in_graph(async {
        declare_next();
        let b = note("B", "plain", true, "o", 2).await;
        let c = note("C", "plain", true, "o", 3).await;
        let a = note("A", &format!("next:{}", b.id), true, "o", 1).await;
        record("bob", &a, EventType::Like, "a").await;
        suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        let first = edges_from(&a, "relation:next").await;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].target, document_node_key(&key(&b)));
        let GraphProvenance::Relation {
            source,
            source_version,
            ..
        } = &first[0].provenance
        else {
            panic!("a relation edge rests on a relation")
        };
        assert_eq!(source, &key(&a));
        let a = a
            .update(suprnova::attrs! { category: format!("next:{}", c.id) })
            .await
            .unwrap();
        drain_graph_jobs().await;
        let replaced = edges_from(&a, "relation:next").await;
        assert_eq!(replaced.len(), 1, "{replaced:?}");
        assert_eq!(replaced[0].target, document_node_key(&key(&c)));
        let GraphProvenance::Relation {
            source_version: newer,
            ..
        } = &replaced[0].provenance
        else {
            panic!("a relation edge rests on a relation")
        };
        assert!(newer > source_version);
        let (store, generation) = current().await;
        let walked = traverse(
            &store,
            &generation,
            &Traversal {
                seeds: vec![document_node_key(&key(&a))],
                depth: 2,
                per_node: 50,
                max_nodes: 100,
                kinds: vec![],
            },
            &Viewer::Guest,
        )
        .await
        .unwrap();
        assert!(reached_keys(&walked).contains(&document_node_key(&key(&c))));
        assert!(!reached_keys(&walked).contains(&document_node_key(&key(&b))));
    }))
    .await;
}

#[tokio::test]
async fn rec_005_a_document_leaving_its_index_loses_its_facts() {
    setup(in_graph(async {
        let a = note("A", "x", true, "o", 1).await;
        let b = note("B", "x", true, "o", 2).await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &b, EventType::Like, "b").await;
        }
        suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        assert_eq!(edges_from(&a, "co_engaged").await.len(), 1);
        suprnova::unsearchable::<Note>(&[b.id.to_string()])
            .await
            .unwrap();
        drain_graph_jobs().await;
        let (store, generation) = current().await;
        assert!(
            store
                .nodes(&generation, &[document_node_key(&key(&b))])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(edges_from(&a, "co_engaged").await.is_empty());
        let engaged = store
            .outgoing(
                &generation,
                &[suprnova::recommendations::graph::actor_node_key("bob")],
                &["engaged".into()],
                50,
            )
            .await
            .unwrap();
        assert!(
            engaged
                .iter()
                .all(|edge| edge.target != document_node_key(&key(&b)))
        );
    }))
    .await;
}

#[tokio::test]
async fn rec_005_retention_revokes_the_facts_resting_on_pruned_events() {
    setup(in_graph(async {
        let a = note("A", "x", true, "o", 1).await;
        let b = note("B", "x", true, "o", 2).await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &b, EventType::Like, "b").await;
        }
        suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        assert_eq!(edges_from(&a, "co_engaged").await.len(), 1);
        let old = (suprnova::clock::now() - chrono::Duration::days(30)).timestamp_micros();
        suprnova::DB::update(
            "UPDATE recommendation_events SET occurred_at = ?",
            vec![old.into()],
        )
        .await
        .unwrap();
        assert_eq!(suprnova::Recommend::prune(7).await.unwrap(), 6);
        assert!(edges_from(&a, "co_engaged").await.is_empty());
        let (store, generation) = current().await;
        assert!(
            store
                .outgoing(
                    &generation,
                    &[suprnova::recommendations::graph::actor_node_key("bob")],
                    &[],
                    50
                )
                .await
                .unwrap()
                .is_empty()
        );
    }))
    .await;
}

#[tokio::test]
async fn rec_009_a_rebuild_of_an_emptied_store_reproduces_the_graph() {
    setup(in_graph(async {
        declare_next();
        let b = note("B", "plain", true, "o", 2).await;
        let a = note("A", &format!("next:{}", b.id), true, "o", 1).await;
        let c = note("C", "plain", false, "carol", 3).await;
        for peer in ["bob", "carol", "dave"] {
            record(peer, &a, EventType::Like, "a").await;
            record(peer, &c, EventType::Like, "c").await;
        }
        suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        let (store, before) = current().await;
        let digest = store.fact_digest(&before).await.unwrap();
        // The store loses everything it held, as a restarted in-memory server would.
        for table in [
            "recommendation_graph_edges",
            "recommendation_graph_nodes",
            "recommendation_graph_generations",
        ] {
            suprnova::DB::update(&format!("DELETE FROM {table}"), vec![])
                .await
                .unwrap();
        }
        let rebuilt = suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        assert_ne!(rebuilt.generation, before);
        assert_eq!(
            store.fact_digest(&rebuilt.generation).await.unwrap(),
            digest
        );
    }))
    .await;
}

#[tokio::test]
async fn rec_009_a_failed_rebuild_leaves_readers_on_the_previous_generation() {
    setup(in_graph(async {
        let a = note("A", "x", true, "o", 1).await;
        record("bob", &a, EventType::Like, "a").await;
        let first = suprnova::Recommend::rebuild_graph().await.unwrap().unwrap();
        suprnova::Recommend::graph_relation(
            suprnova::recommendations::graph::GraphRelation::new(
                "broken",
                "recommendation_notes",
                5_000,
                |_| Err(suprnova::FrameworkError::internal("relation read failed")),
            )
            .unwrap(),
        )
        .unwrap();
        let failed = suprnova::Recommend::rebuild_graph().await.unwrap_err();
        assert!(failed.to_string().contains("relation read failed"));
        let (_, generation) = current().await;
        assert_eq!(generation, first.generation);
        let building = suprnova::DB::scalar::<i64>(
            "SELECT COUNT(*) FROM recommendation_graph_generations WHERE state = 'building'",
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(building, 0);
    }))
    .await;
}

#[cfg(feature = "graph-memgraph")]
async fn memgraph_store() -> suprnova::MemgraphGraphStore {
    let url = std::env::var("MEMGRAPH_TEST_URL").expect("MEMGRAPH_TEST_URL is required");
    let config = neo4rs::ConfigBuilder::default()
        .uri(&url)
        .user("")
        .password("")
        .db("memgraph")
        .build()
        .unwrap();
    let graph = neo4rs::Graph::connect(config).await.unwrap();
    graph
        .run(neo4rs::query("MATCH (n) DETACH DELETE n"))
        .await
        .unwrap();
    suprnova::MemgraphGraphStore::connect(&url, "", "")
        .await
        .unwrap()
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_a_store_answers_hops_strongest_first_within_its_fan_out_memgraph() {
    setup(async {
        let store = memgraph_store().await;
        a_store_answers_hops_strongest_first_within_its_fan_out(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope_memgraph()
 {
    setup(async {
        let store = memgraph_store().await;
        a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_a_traversal_stops_at_its_depth_bound_memgraph() {
    setup(async {
        let store = memgraph_store().await;
        a_traversal_stops_at_its_depth_bound(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_publishing_switches_readers_in_one_step_and_keeps_the_previous_generation_memgraph()
 {
    setup(async {
        let store = memgraph_store().await;
        publishing_switches_readers_in_one_step_and_keeps_the_previous_generation(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_revocations_remove_the_facts_resting_on_a_changed_or_removed_source_memgraph() {
    setup(async {
        let store = memgraph_store().await;
        revocations_remove_the_facts_resting_on_a_changed_or_removed_source(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_the_same_facts_give_the_same_digest_in_any_generation_memgraph() {
    setup(async {
        let store = memgraph_store().await;
        the_same_facts_give_the_same_digest_in_any_generation(&store).await;
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_the_two_drivers_answer_alike_memgraph() {
    setup(async {
        let memgraph = memgraph_store().await;
        let database = DatabaseGraphStore;
        let nodes: Vec<_> = (0..12)
            .map(|i| node(&format!("fixture-{i}"), i % 3 != 1, "bob", 1, i + 1))
            .collect();
        let mut edges = Vec::new();
        for i in 1..12 {
            let from = if i < 5 { 0 } else { 1 + i % 4 };
            let kind = ["co_engaged", "relation:series", "engaged"][i % 3];
            edges.push(edge(
                kind,
                &nodes[from],
                &nodes[i],
                10_000 - i as i32 * 431,
                event(i as i64 + 20),
            ));
        }
        let mut generations = Vec::new();
        for store in [&database as &dyn GraphStore, &memgraph] {
            let generation = store.begin_generation().await.unwrap();
            store.put_nodes(&generation, &nodes).await.unwrap();
            store.put_edges(&generation, &edges).await.unwrap();
            generations.push(generation);
        }
        assert_eq!(
            database.fact_digest(&generations[0]).await.unwrap(),
            memgraph.fact_digest(&generations[1]).await.unwrap()
        );
        for viewer in [Viewer::Guest, viewer("bob")] {
            for depth in 1..=3 {
                let request = walk(&nodes[0], depth);
                let expected = traverse(&database, &generations[0], &request, &viewer)
                    .await
                    .unwrap();
                let actual = traverse(&memgraph, &generations[1], &request, &viewer)
                    .await
                    .unwrap();
                assert_eq!(expected.len(), actual.len());
                for (expected, actual) in expected.iter().zip(&actual) {
                    assert_eq!(expected.node.key, actual.node.key);
                    assert_eq!(expected.path, actual.path);
                    assert_eq!(expected.weight, actual.weight);
                }
            }
        }
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_updates_paging_and_revocation_counts_match_memgraph() {
    setup(async {
        let memgraph = memgraph_store().await;
        let stores: [&dyn GraphStore; 2] = [&DatabaseGraphStore, &memgraph];
        let mut results = Vec::new();
        for store in stores {
            let generation = store.begin_generation().await.unwrap();
            let mut a = node("quote-'-$value", true, "bob", 2, 10);
            let b = node("b", true, "bob", 1, 11);
            let c = node("c", false, "bob", 1, 12);
            store
                .put_nodes(&generation, &[a.clone(), b.clone(), c.clone()])
                .await
                .unwrap();
            a.source_version = 3;
            store.put_nodes(&generation, &[a.clone()]).await.unwrap();
            let mut link = edge("co_engaged", &a, &b, 7_000, event(20));
            store
                .put_edges(
                    &generation,
                    &[link.clone(), edge("engaged", &b, &c, 6_000, event(21))],
                )
                .await
                .unwrap();
            link.created_at = 99;
            link.updated_at = 100;
            link.confidence_bps = 8_000;
            store.put_edges(&generation, &[link]).await.unwrap();
            let links = store
                .outgoing(&generation, &[a.key.clone()], &[], 0)
                .await
                .unwrap();
            assert_eq!(links[0].created_at, 1);
            assert_eq!(links[0].updated_at, 100);
            let mut page = store.document_nodes(&generation, None, 1).await.unwrap();
            let last = page[0].key.clone();
            page.extend(
                store
                    .document_nodes(&generation, Some(&last), 10)
                    .await
                    .unwrap(),
            );
            assert_eq!(page.len(), 3);
            let updated = store.nodes(&generation, &[a.key.clone()]).await.unwrap();
            assert_eq!(updated, vec![a.clone()]);
            let removed = store
                .revoke(&generation, &Revocation::Events(vec![11]))
                .await
                .unwrap();
            assert_eq!(removed, 3);
            let before = store
                .revoke(
                    &generation,
                    &Revocation::DocumentBefore(a.target.clone().unwrap(), 4),
                )
                .await
                .unwrap();
            assert_eq!(before, 1);
            store.publish(&generation).await.unwrap();
            store.publish(&generation).await.unwrap();
            let next = store.begin_generation().await.unwrap();
            store.publish(&next).await.unwrap();
            assert!(store.publish(&generation).await.is_err());
            store.discard(&generation).await.unwrap();
            assert!(
                store
                    .nodes(&generation, std::slice::from_ref(&c.key))
                    .await
                    .unwrap()
                    .is_empty()
            );
            results.push((page, links, removed, before));
        }
        assert_eq!(results[0], results[1]);
    })
    .await;
}

#[cfg(feature = "graph-memgraph")]
#[tokio::test]
#[ignore = "requires Memgraph at MEMGRAPH_TEST_URL"]
async fn rec_009_actor_nodes_and_edges_before_nodes_match_memgraph() {
    setup(async {
        let memgraph = memgraph_store().await;
        let mut results = Vec::new();
        for store in [&DatabaseGraphStore as &dyn GraphStore, &memgraph] {
            let generation = store.begin_generation().await.unwrap();
            let mut actor = node("actor", true, "bob", 1, 1);
            actor.key = suprnova::recommendations::graph::actor_node_key("bob");
            actor.kind = "actor".into();
            actor.target = None;
            actor.scope = GraphScope::default();
            let document = node("document", true, "bob", 1, 2);
            let mut link = edge("engaged", &actor, &document, 4_000, event(3));
            store.put_edges(&generation, &[link.clone()]).await.unwrap();
            assert!(
                store
                    .nodes(&generation, &[actor.key.clone(), document.key.clone()])
                    .await
                    .unwrap()
                    .is_empty()
            );
            let orphan_digest = store.fact_digest(&generation).await.unwrap();
            store
                .put_nodes(&generation, &[actor.clone(), document.clone()])
                .await
                .unwrap();
            let actors = store
                .nodes(&generation, &[actor.key.clone()])
                .await
                .unwrap();
            assert_eq!(actors, vec![actor.clone()]);
            assert_eq!(
                store.document_nodes(&generation, None, 10).await.unwrap(),
                vec![document.clone()]
            );
            // Upsert by key also permits changing endpoints, as the SQL driver does.
            link.source = document.key.clone();
            link.target = actor.key.clone();
            store.put_edges(&generation, &[link.clone()]).await.unwrap();
            assert!(
                store
                    .outgoing(&generation, &[actor.key.clone()], &[], 10)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                store
                    .outgoing(&generation, std::slice::from_ref(&document.key), &[], 10)
                    .await
                    .unwrap(),
                vec![link]
            );
            let digest = store.fact_digest(&generation).await.unwrap();
            let removed = store
                .revoke(&generation, &Revocation::Events(vec![1]))
                .await
                .unwrap();
            assert_eq!(removed, 2);
            results.push((orphan_digest, digest, removed));
        }
        assert_eq!(results[0], results[1]);
    })
    .await;
}
