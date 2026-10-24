//! Driver selection and dialect contracts for REC-010.
#[allow(dead_code)]
#[path = "../../src/recommendations/graph/cypher.rs"]
mod cypher;
#[allow(dead_code)]
#[path = "../../src/recommendations/graph/hydradb/statements.rs"]
mod hydra;

#[cfg(any(feature = "graph-neo4j", feature = "graph-hydradb"))]
use suprnova::GraphStore;
use suprnova::RecommendConfig;

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_http_requires_https_in_production() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_http_requires_https_in_production_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_http_requires_https_in_production_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = crate::env_snapshot::EnvSnapshot::capture(&["APP_ENV"]);
    let mut config = RecommendConfig {
        graph_store: "hydradb".into(),
        ..Default::default()
    };
    for environment in [None, Some("production"), Some("PROD"), Some("testing")] {
        crate::env_snapshot::set_env("APP_ENV", environment);
        if environment == Some("testing") {
            suprnova::Config::register(
                suprnova::AppConfig::builder()
                    .environment(suprnova::Environment::Production)
                    .build(),
            );
        }
        assert!(suprnova::Config::is_production());
        for host in ["localhost", "127.0.0.1", "[::1]", "graph.example.com"] {
            config.graph_url = Some(format!("http://{host}:8443/v1/graphs/default/query"));
            let error = config.validate().expect_err("HTTP must fail in production");
            let message = error.to_string();
            assert!(message.contains("RECOMMEND_GRAPH_URL"), "{message}");
            assert!(message.contains("HTTPS"), "{message}");
            config.graph_url = Some(format!("https://{host}:8443/v1/graphs/default/query"));
            config
                .validate()
                .expect("HTTPS remains valid in production");
        }
    }
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_http_loopback_is_limited_to_development_and_testing() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_http_loopback_is_limited_to_development_and_testing_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_http_loopback_is_limited_to_development_and_testing_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let _env = crate::env_lock::lock_env();
    let _restore = crate::env_snapshot::EnvSnapshot::capture(&["APP_ENV"]);
    crate::env_snapshot::set_env("APP_ENV", Some("production"));
    let mut config = RecommendConfig {
        graph_store: "hydradb".into(),
        ..Default::default()
    };
    for environment in [
        suprnova::Environment::Local,
        suprnova::Environment::Development,
        suprnova::Environment::Testing,
        suprnova::Environment::Staging,
        suprnova::Environment::Custom("preview".into()),
    ] {
        let allow_http = matches!(
            environment,
            suprnova::Environment::Local
                | suprnova::Environment::Development
                | suprnova::Environment::Testing
        );
        suprnova::Config::register(
            suprnova::AppConfig::builder()
                .environment(environment.clone())
                .build(),
        );
        for host in ["localhost", "127.0.0.1", "[::1]", "graph.example.com"] {
            config.graph_url = Some(format!("http://{host}:8443/v1/graphs/default/query"));
            let result = config.validate();
            assert_eq!(
                result.is_ok(),
                allow_http && host != "graph.example.com",
                "{environment:?} with {host}: {result:?}"
            );
            config.graph_url = Some(format!("https://{host}:8443/v1/graphs/default/query"));
            config.validate().expect("HTTPS remains valid");
        }
    }
}

#[test]
fn rec_010_boot_rejects_unknown_and_unavailable_graph_drivers() {
    for (name, unavailable) in [
        ("unknown", true),
        ("neo4j", !cfg!(feature = "graph-neo4j")),
        ("hydradb", !cfg!(feature = "graph-hydradb")),
    ] {
        if !unavailable {
            continue;
        }
        suprnova::Config::register(RecommendConfig {
            graph_store: name.into(),
            ..Default::default()
        });
        let error = suprnova::Server::try_from_config_with_routes(|| Ok(suprnova::Router::new()))
            .err()
            .expect("boot must reject this driver")
            .to_string();
        assert!(error.contains("RECOMMEND_GRAPH_STORE"), "{error}");
        if name != "unknown" {
            assert!(error.contains(&format!("graph-{name}")), "{error}");
        }
    }
}

#[tokio::test]
async fn rec_010_store_names_resolve_and_features_gate_the_drivers() {
    let manifest = include_str!("../../Cargo.toml");
    for (name, enabled) in [
        ("neo4j", cfg!(feature = "graph-neo4j")),
        ("hydradb", cfg!(feature = "graph-hydradb")),
    ] {
        assert!(manifest.contains(&format!("graph-{name} = [\"dep:neo4rs\"]")));
        let config = RecommendConfig {
            graph_store: name.into(),
            ..Default::default()
        };
        if !enabled {
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains(&format!("graph-{name}")), "{error}");
            continue;
        }
        config.validate().unwrap();
        let error = RecommendConfig::scope(config, suprnova::recommendations::graph::store())
            .await
            .unwrap()
            .err()
            .expect("no endpoint was supplied")
            .to_string();
        assert!(!error.contains("unknown store"), "{error}");
        assert!(
            error.contains(if enabled {
                "RECOMMEND_GRAPH_URL"
            } else {
                if name == "neo4j" {
                    "graph-neo4j"
                } else {
                    "graph-hydradb"
                }
            }),
            "{error}"
        );
    }
    let config = RecommendConfig {
        graph_store: "not-a-store".into(),
        ..Default::default()
    };
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("RECOMMEND_GRAPH_STORE")
    );
}

#[test]
fn rec_010_neo4j_uses_idempotent_node_and_relationship_range_indexes() {
    let indexes = cypher::NEO4J_INDEXES;
    assert_eq!(indexes.len(), 6);
    for (pattern, properties) in [
        ("(n:SuprnovaGraphNode)", &["n.generation", "n.key"][..]),
        ("(g:SuprnovaGraphGeneration)", &["g.id", "g.state"][..]),
        (
            "()-[r:SUPRNOVA_GRAPH_EDGE]-()",
            &["r.key", "r.generation"][..],
        ),
    ] {
        for property in properties {
            assert_eq!(
                indexes
                    .iter()
                    .filter(
                        |ddl| ddl.contains(pattern) && ddl.ends_with(&format!("ON ({property})"))
                    )
                    .count(),
                1
            );
        }
    }
    assert!(indexes.iter().all(|ddl| ddl.starts_with("CREATE INDEX ")
        && ddl.contains(" IF NOT EXISTS FOR ")
        && !ddl.contains("EDGE INDEX")));
}

#[test]
fn rec_010_transactional_statements_use_the_common_cypher_dialect() {
    let queries = [
        cypher::DISCARD_NODES,
        cypher::DISCARD_GENERATION,
        cypher::CURRENT_GENERATION,
        cypher::BEGIN_GENERATION,
        cypher::GENERATION_STATE,
        cypher::PUBLISH,
        cypher::RETIRED_GENERATIONS,
        cypher::PUT_NODES,
        cypher::PUT_EDGES,
        cypher::REVOKE_NODES,
        cypher::REVOKE_EDGES,
        cypher::DELETE_NODES,
        cypher::NODES,
        cypher::OUTGOING,
        cypher::DOCUMENT_NODES,
        cypher::DIGEST_NODES,
        cypher::DIGEST_EDGES,
    ];
    assert_eq!(queries.len(), 17);
    for statement in queries {
        assert!(statement.contains('$'), "values must be bound: {statement}");
        for engine_extension in [
            "CREATE EDGE INDEX",
            "CREATE INDEX ON",
            "CALL mg.",
            "CALL apoc.",
            "USING INDEX",
            "timestamp()",
        ] {
            assert!(!statement.contains(engine_extension), "{statement}");
        }
    }
    assert!(cypher::PUT_EDGES.contains("coalesce(min(old.created_at), fact.created_at)"));
    assert!(cypher::OUTGOING.contains("collect(e)[..$limit]"));
}

#[test]
fn rec_010_hydradb_native_statements_avoid_unsupported_cypher() {
    assert!(
        hydra::INDEX_DDL.is_empty(),
        "HydraDB indexes properties automatically"
    );
    for statement in [
        hydra::HEAD,
        hydra::READ_HEAD,
        hydra::PUBLISH,
        hydra::BEGIN,
        hydra::GENERATION,
        hydra::DISCARD_NODES,
        hydra::DISCARD_GENERATION,
        hydra::PUT_NODES,
        hydra::PLACEHOLDERS,
        hydra::PUT_EDGES,
        hydra::NODES,
        hydra::DOCUMENTS,
        hydra::ALL_NODES,
        hydra::ALL_EDGES,
        hydra::DELETE_EDGES,
        hydra::DELETE_NODES,
        hydra::IDENTITY,
        hydra::OUTGOING,
        hydra::EDGES_BY_KEY,
    ] {
        for unsupported in [
            "CREATE INDEX",
            "EDGE INDEX",
            "FOREACH",
            "OPTIONAL MATCH",
            "collect(",
            " IS NULL",
            " IS NOT NULL",
            " IN $",
            "SET n =",
            "SET e =",
            "CASE WHEN",
            "RETURN n AS",
            "RETURN e AS",
        ] {
            assert!(!statement.contains(unsupported), "{statement}");
        }
    }
    assert!(hydra::PUT_NODES.contains("id: fact.vertex"));
    assert!(hydra::PUT_EDGES.contains("id: fact.vertex"));
    assert!(hydra::PUBLISH.contains("g.state = $state"));
    assert!(!hydra::PLACEHOLDERS.contains("n.json ="));
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_refuses_node_and_edge_numeric_identity_collisions() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_refuses_node_and_edge_numeric_identity_collisions_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
async fn rec_010_hydradb_refuses_node_and_edge_numeric_identity_collisions_child() {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method},
    };
    if !crate::own_process::is_child() {
        return;
    }
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Testing)
            .build(),
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"query":hydra::HEAD})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"columns":[],"rows":[],"next_cursor":null})),
        )
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"columns":["key","generation"],"rows":[[{"type":"string","value":"another-key"},{"type":"string","value":"generation"}]],"next_cursor":null})))
        .with_priority(2).expect(2).mount(&server).await;
    let store = suprnova::HydraDbGraphStore::connect(
        &format!("{}/v1/graphs/default/query", server.uri()),
        "default",
        "test-token",
    )
    .await
    .unwrap();
    let scope = suprnova::GraphScope {
        index: None,
        values: Default::default(),
    };
    let provenance = suprnova::GraphProvenance::Event(suprnova::EventProvenance {
        id: 1,
        digest: "1".repeat(64),
    });
    let node = suprnova::GraphNode {
        key: "node".into(),
        kind: "article".into(),
        target: None,
        scope: scope.clone(),
        source_version: 1,
        provenance: provenance.clone(),
    };
    let edge = suprnova::GraphEdge {
        key: "edge".into(),
        source: "node".into(),
        target: "other".into(),
        kind: "related".into(),
        confidence_bps: 5000,
        scope,
        source_version: 1,
        provenance,
        created_at: 1,
        updated_at: 1,
    };
    assert!(
        store
            .put_nodes("generation", &[node])
            .await
            .unwrap_err()
            .to_string()
            .contains("identity collision")
    );
    assert!(
        store
            .put_edges("generation", &[edge])
            .await
            .unwrap_err()
            .to_string()
            .contains("identity collision")
    );
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_http_pages_reuse_query_identity_without_replaying_read_epoch() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_http_pages_reuse_query_identity_without_replaying_read_epoch_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
async fn rec_010_hydradb_http_pages_reuse_query_identity_without_replaying_read_epoch_child() {
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use wiremock::{
        Mock, MockServer, Request, Respond, ResponseTemplate,
        matchers::{header, method, path},
    };

    if !crate::own_process::is_child() {
        return;
    }
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Testing)
            .build(),
    );

    let nodes: Vec<_> = ["one", "two"]
        .map(|key| suprnova::GraphNode {
            key: key.into(),
            kind: "article".into(),
            target: None,
            scope: suprnova::GraphScope {
                index: None,
                values: Default::default(),
            },
            source_version: 1,
            provenance: suprnova::GraphProvenance::Event(suprnova::EventProvenance {
                id: 1,
                digest: "1".repeat(64),
            }),
        })
        .into();
    struct Pages {
        query_id: Arc<Mutex<Option<String>>>,
        nodes: Vec<suprnova::GraphNode>,
    }
    impl Respond for Pages {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if body["query"] == hydra::HEAD {
                return ResponseTemplate::new(200)
                    .set_body_json(json!({"columns":[],"rows":[],"next_cursor":null}));
            }
            assert_eq!(body["query"], hydra::DOCUMENTS);
            assert!(
                body.get("read_epoch").is_none(),
                "read_epoch is output-only in HydraDB's public HTTP API"
            );
            let query_id = body["query_id"].as_str().unwrap();
            let mut first = self.query_id.lock().unwrap();
            let (index, next) = if let Some(first) = first.as_deref() {
                assert_eq!(query_id, first);
                assert_eq!(body["cursor"], 7);
                (1, None)
            } else {
                assert!(!query_id.is_empty());
                *first = Some(query_id.into());
                (0, Some(7))
            };
            ResponseTemplate::new(200).set_body_json(json!({"columns":["json","key"],"rows":[[{"type":"string","value":serde_json::to_string(&self.nodes[index]).unwrap()},{"type":"string","value":self.nodes[index].key}]],"read_epoch":123,"next_cursor":next}))
        }
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/graphs/default/query"))
        .and(header("Authorization", "Bearer test-token"))
        .and(header("X-Graph-Namespace", "default"))
        .respond_with(Pages {
            query_id: Arc::new(Mutex::new(None)),
            nodes: nodes.clone(),
        })
        .expect(3)
        .mount(&server)
        .await;
    let store = suprnova::HydraDbGraphStore::connect(
        &format!("{}/v1/graphs/default/query", server.uri()),
        "default",
        "test-token",
    )
    .await
    .unwrap();
    assert_eq!(
        store.document_nodes("generation", None, 2).await.unwrap(),
        nodes
    );
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
#[ignore = "requires a disposable HydraDB at HYDRADB_TEST_URL"]
async fn rec_010_full_store_conformance_hydradb() {
    let url = std::env::var("HYDRADB_TEST_URL").expect("HYDRADB_TEST_URL is required");
    let username = std::env::var("HYDRADB_TEST_USERNAME").unwrap_or_else(|_| "default".into());
    let password =
        std::env::var("HYDRADB_TEST_PASSWORD").expect("HYDRADB_TEST_PASSWORD is required");
    let client = reqwest::Client::new();
    for query in [
        "MATCH (n:SuprnovaGraphHead) DETACH DELETE n",
        "MATCH (n:SuprnovaGraphNode) DETACH DELETE n",
        "MATCH (n:SuprnovaGraphGeneration) DETACH DELETE n",
    ] {
        client
            .post(&url)
            .bearer_auth(&password)
            .header("X-Graph-Namespace", &username)
            .json(&serde_json::json!({"cell_id":"cell-0","query":query}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let store = suprnova::HydraDbGraphStore::connect(&url, &username, &password)
        .await
        .unwrap();
    assert_eq!(suprnova::GraphStore::name(&store), "hydradb");
    super::support::setup(conforms(&store)).await;
    let refused = suprnova::HydraDbGraphStore::connect(&url, &username, "invalid-test-token")
        .await
        .err()
        .expect("a cached pool must not bypass authentication")
        .to_string();
    assert!(!refused.contains("invalid-test-token"));
}

#[cfg(feature = "graph-neo4j")]
#[tokio::test]
#[ignore = "requires a disposable Neo4j at NEO4J_TEST_URL"]
async fn rec_010_full_store_conformance_neo4j() {
    let url = std::env::var("NEO4J_TEST_URL").expect("NEO4J_TEST_URL is required");
    let username = std::env::var("NEO4J_TEST_USERNAME").unwrap_or_else(|_| "neo4j".into());
    let password = std::env::var("NEO4J_TEST_PASSWORD").expect("NEO4J_TEST_PASSWORD is required");
    let graph = neo4rs::Graph::new(&url, &username, &password)
        .await
        .unwrap();
    graph
        .run(neo4rs::query("MATCH (n) DETACH DELETE n"))
        .await
        .unwrap();
    let store = suprnova::Neo4jGraphStore::connect(&url, &username, &password)
        .await
        .unwrap();
    assert_eq!(suprnova::GraphStore::name(&store), "neo4j");
    super::support::setup(conforms(&store)).await;
    // Reapplying every index must be safe even when a pool already exists.
    for statement in cypher::NEO4J_INDEXES {
        graph.run(neo4rs::query(statement)).await.unwrap();
    }
    suprnova::Neo4jGraphStore::connect(&url, &username, &password)
        .await
        .unwrap();
    let mut rows = graph.execute(neo4rs::query("SHOW INDEXES YIELD name, state WHERE name STARTS WITH 'suprnova_graph_' RETURN name, state")).await.unwrap();
    let mut count = 0;
    while rows.next().await.unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 6);
    let refused = suprnova::Neo4jGraphStore::connect(&url, &username, "invalid-test-password")
        .await
        .err()
        .expect("a cached pool must not bypass authentication")
        .to_string();
    assert!(!refused.contains("invalid-test-password"));
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
#[ignore = "requires a disposable Neo4j at NEO4J_TEST_URL"]
async fn rec_010_hydradb_bolt_transport_neo4j() {
    let url = std::env::var("NEO4J_TEST_URL").expect("NEO4J_TEST_URL is required");
    let username = std::env::var("NEO4J_TEST_USERNAME").unwrap_or_else(|_| "neo4j".into());
    let password = std::env::var("NEO4J_TEST_PASSWORD").expect("NEO4J_TEST_PASSWORD is required");
    let graph = neo4rs::Graph::new(&url, &username, &password)
        .await
        .unwrap();
    graph
        .run(neo4rs::query("MATCH (n) DETACH DELETE n"))
        .await
        .unwrap();
    let address = format!("{}/neo4j", url.trim_end_matches('/'));
    let store = suprnova::HydraDbGraphStore::connect(&address, &username, &password)
        .await
        .unwrap();
    super::support::setup(conforms(&store)).await;
}

#[cfg(any(feature = "graph-neo4j", feature = "graph-hydradb"))]
async fn conforms(store: &dyn suprnova::GraphStore) {
    use super::graph::*;
    publishing_switches_readers_in_one_step_and_keeps_the_previous_generation(store).await;
    a_store_answers_hops_strongest_first_within_its_fan_out(store).await;
    a_traversal_never_returns_or_walks_through_a_fact_outside_the_viewer_scope(store).await;
    a_traversal_stops_at_its_depth_bound(store).await;
    revocations_remove_the_facts_resting_on_a_changed_or_removed_source(store).await;
    the_same_facts_give_the_same_digest_in_any_generation(store).await;
    extra_operations(store).await;
    paginated_reads(store).await;
}

#[cfg(any(feature = "graph-neo4j", feature = "graph-hydradb"))]
async fn paginated_reads(store: &dyn suprnova::GraphStore) {
    let generation = store.begin_generation().await.unwrap();
    let nodes: Vec<_> = (0..503)
        .map(|id| suprnova::GraphNode {
            key: format!("page-{id:04}"),
            kind: "article".into(),
            target: Some(suprnova::RecommendationTarget {
                index: "articles".into(),
                key: id.to_string(),
            }),
            scope: suprnova::GraphScope {
                index: None,
                values: Default::default(),
            },
            source_version: 1,
            provenance: suprnova::GraphProvenance::Event(suprnova::EventProvenance {
                id,
                digest: format!("{id:064}"),
            }),
        })
        .collect();
    store.put_nodes(&generation, &nodes).await.unwrap();
    let keys: Vec<_> = nodes.iter().map(|node| node.key.clone()).collect();
    assert_eq!(store.nodes(&generation, &keys).await.unwrap(), nodes);
    assert_eq!(
        store.document_nodes(&generation, None, 503).await.unwrap(),
        nodes
    );
    let mut actor = nodes[0].clone();
    actor.target = None;
    store
        .put_nodes(&generation, &[actor.clone()])
        .await
        .unwrap();
    assert_eq!(
        store
            .nodes(&generation, &[actor.key.clone()])
            .await
            .unwrap(),
        [actor]
    );
    assert_eq!(
        store
            .document_nodes(&generation, None, 503)
            .await
            .unwrap()
            .len(),
        502
    );
    assert!(!store.fact_digest(&generation).await.unwrap().is_empty());
    store.discard(&generation).await.unwrap();
}

#[cfg(any(feature = "graph-neo4j", feature = "graph-hydradb"))]
async fn extra_operations(store: &dyn suprnova::GraphStore) {
    use suprnova::{
        EventProvenance, GraphEdge, GraphNode, GraphProvenance, GraphScope, RecommendationTarget,
        Revocation,
    };
    let generation = store.begin_generation().await.unwrap();
    let nodes: Vec<_> = (0..3)
        .map(|id| GraphNode {
            key: format!("node-{id}"),
            kind: "article".into(),
            target: Some(RecommendationTarget {
                index: "articles".into(),
                key: id.to_string(),
            }),
            scope: GraphScope {
                index: None,
                values: Default::default(),
            },
            source_version: 1,
            provenance: GraphProvenance::Event(EventProvenance {
                id,
                digest: format!("{id:064}"),
            }),
        })
        .collect();
    let mut edge = GraphEdge {
        key: "link".into(),
        source: nodes[0].key.clone(),
        target: nodes[1].key.clone(),
        kind: "related".into(),
        confidence_bps: 9000,
        scope: nodes[0].scope.clone(),
        source_version: 1,
        provenance: nodes[0].provenance.clone(),
        created_at: 1,
        updated_at: 2,
    };
    // Edges may precede materialized endpoints. Replacement preserves creation time.
    store.put_edges(&generation, &[edge.clone()]).await.unwrap();
    assert!(
        store
            .nodes(&generation, &[nodes[0].key.clone()])
            .await
            .unwrap()
            .is_empty()
    );
    store.put_nodes(&generation, &nodes).await.unwrap();
    edge.target = nodes[2].key.clone();
    edge.created_at = 99;
    edge.updated_at = 100;
    store.put_edges(&generation, &[edge]).await.unwrap();
    let outgoing = store
        .outgoing(&generation, &[nodes[0].key.clone()], &[], 10)
        .await
        .unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].created_at, 1);
    assert_eq!(outgoing[0].target, nodes[2].key);
    let database = suprnova::DatabaseGraphStore;
    let reference = database.begin_generation().await.unwrap();
    database.put_nodes(&reference, &nodes).await.unwrap();
    database.put_edges(&reference, &outgoing).await.unwrap();
    assert_eq!(
        store.fact_digest(&generation).await.unwrap(),
        database.fact_digest(&reference).await.unwrap()
    );
    let first = store.document_nodes(&generation, None, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let rest = store
        .document_nodes(&generation, Some(&first[1].key), 2)
        .await
        .unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(
        store
            .revoke(&generation, &Revocation::Events(vec![0]))
            .await
            .unwrap(),
        2
    );
    store.discard(&generation).await.unwrap();
    assert!(
        store
            .nodes(&generation, &[nodes[1].key.clone()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.publish(&generation).await.is_err());

    let generation = store.begin_generation().await.unwrap();
    let mut auxiliary = nodes[0].clone();
    auxiliary.key = "auxiliary".into();
    auxiliary.target = None;
    auxiliary.provenance = GraphProvenance::Relation {
        name: "series".into(),
        source: nodes[1].target.clone().unwrap(),
        source_version: 1,
    };
    store
        .put_nodes(
            &generation,
            &[nodes[1].clone(), nodes[2].clone(), auxiliary.clone()],
        )
        .await
        .unwrap();
    let edge = |key: &str, target: &GraphNode, provenance: GraphProvenance| GraphEdge {
        key: key.into(),
        source: nodes[1].key.clone(),
        target: target.key.clone(),
        kind: "series".into(),
        confidence_bps: 7000,
        scope: target.scope.clone(),
        source_version: 1,
        provenance,
        created_at: 1,
        updated_at: 1,
    };
    store
        .put_edges(
            &generation,
            &[
                edge("relation", &auxiliary, auxiliary.provenance.clone()),
                edge("event", &nodes[2], nodes[2].provenance.clone()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .revoke(
                &generation,
                &Revocation::DocumentRelationsBefore(nodes[1].target.clone().unwrap(), 2)
            )
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .nodes(&generation, &[nodes[1].key.clone()])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .nodes(&generation, &[auxiliary.key])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .outgoing(&generation, &[nodes[1].key.clone()], &[], 10)
            .await
            .unwrap()
            .len(),
        1
    );
    store.discard(&generation).await.unwrap();

    let generation = store.begin_generation().await.unwrap();
    store.put_nodes(&generation, &nodes).await.unwrap();
    store.publish(&generation).await.unwrap();
    store.publish(&generation).await.unwrap();
    assert!(store.discard(&generation).await.is_err());
    assert_eq!(
        store
            .nodes(&generation, &[nodes[0].key.clone()])
            .await
            .unwrap()
            .len(),
        1
    );
    let next = store.begin_generation().await.unwrap();
    store.publish(&next).await.unwrap();
    assert!(store.publish(&generation).await.is_err());
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_loopback_bypasses_environment_proxy() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_loopback_bypasses_environment_proxy_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
async fn rec_010_hydradb_loopback_bypasses_environment_proxy_child() {
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    if !crate::own_process::is_child() {
        return;
    }
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Testing)
            .build(),
    );
    let proxy = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"columns":[],"rows":[],"next_cursor":null})),
        )
        .mount(&proxy)
        .await;
    for (index, exclusion) in [None, Some("example.com"), Some("127.0.0.1")]
        .into_iter()
        .enumerate()
    {
        for name in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
            unsafe {
                std::env::set_var(name, proxy.uri());
            }
        }
        for name in ["NO_PROXY", "no_proxy"] {
            unsafe {
                match exclusion {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"columns":[],"rows":[],"next_cursor":null})),
            )
            .mount(&server)
            .await;
        suprnova::HydraDbGraphStore::connect(
            &format!("{}/v1/graphs/proxy-{index}/query", server.uri()),
            "default",
            "test-token",
        )
        .await
        .unwrap();
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "loopback request must reach its origin with NO_PROXY={exclusion:?}"
        );
        assert!(
            proxy.received_requests().await.unwrap().is_empty(),
            "proxy received the bearer credential"
        );
    }
}

#[cfg(feature = "graph-hydradb")]
#[test]
fn rec_010_hydradb_reads_complete_pages_and_queries_without_driver_ceilings() {
    crate::own_process::run_alone(
        "graph_drivers::rec_010_hydradb_reads_complete_pages_and_queries_without_driver_ceilings_child",
    );
}

#[cfg(feature = "graph-hydradb")]
#[tokio::test]
async fn rec_010_hydradb_reads_complete_pages_and_queries_without_driver_ceilings_child() {
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    if !crate::own_process::is_child() {
        return;
    }
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Testing)
            .build(),
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let request: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(request["page_size"], 500);
            let cursor = request["cursor"].as_u64().unwrap_or(0);
            // Each page respects the requested row count. The full query exceeds
            // every former byte, page and row ceiling.
            let padding = if cursor == 0 { 2 * 1024 * 1024 } else { 256 * 1024 };
            ResponseTemplate::new(200).set_body_string(format!(
                "{}{}",
                json!({"columns":[],"rows":vec![Vec::<u8>::new();500],"next_cursor":(cursor < 65).then_some(cursor+1)}),
                " ".repeat(padding),
            ))
        }).mount(&server).await;
    suprnova::HydraDbGraphStore::connect(
        &format!("{}/v1/graphs/default/query", server.uri()),
        "default",
        "test-token",
    )
    .await
    .expect("complete pages and advancing cursors have no driver ceiling");
    assert!(server.received_requests().await.unwrap().len() > 64);
}
