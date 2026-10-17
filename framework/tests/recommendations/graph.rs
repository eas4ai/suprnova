//! The graph store and its scoped traversal (REC-009), on the application's database.

use super::support::*;
use std::collections::BTreeSet;
use suprnova::{
    AttributeValue, EventProvenance, RecommendationTarget, Viewer,
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
