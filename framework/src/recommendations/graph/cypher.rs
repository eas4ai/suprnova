//! Cypher shared by the transactional Bolt stores, kept explicit for dialect checks.

/// Discard nodes without interpolating application values.
pub const DISCARD_NODES: &str =
    "MATCH (n:SuprnovaGraphNode {generation: $generation}) DETACH DELETE n";

/// Discard generation without interpolating application values.
pub const DISCARD_GENERATION: &str =
    "MATCH (g:SuprnovaGraphGeneration {id: $generation}) WHERE g.state <> $current DELETE g";

/// Current generation without interpolating application values.
pub const CURRENT_GENERATION: &str =
    "MATCH (g:SuprnovaGraphGeneration {state: $state}) RETURN g.id AS id";

/// Begin generation without interpolating application values.
pub const BEGIN_GENERATION: &str =
    "CREATE (:SuprnovaGraphGeneration {id: $id, state: $state, created_at: $created_at})";

/// Generation state without interpolating application values.
pub const GENERATION_STATE: &str =
    "MATCH (g:SuprnovaGraphGeneration {id: $generation}) RETURN g.state AS state";

/// Publish without interpolating application values.
pub const PUBLISH: &str = "MATCH (g:SuprnovaGraphGeneration) WHERE g.state = $current OR g.id = $generation SET g.state = CASE WHEN g.id = $generation THEN $current ELSE $retired END";

/// Retired generations without interpolating application values.
pub const RETIRED_GENERATIONS: &str = "MATCH (g:SuprnovaGraphGeneration {state: $state}) RETURN g.id AS id ORDER BY g.created_at DESC, g.id DESC";

/// Put nodes without interpolating application values.
pub const PUT_NODES: &str = "UNWIND $facts AS fact MERGE (n:SuprnovaGraphNode {generation: $generation, key: fact.key}) SET n = fact, n.generation = $generation";

/// Put edges without interpolating application values.
pub const PUT_EDGES: &str = "UNWIND $facts AS fact OPTIONAL MATCH (previous_source:SuprnovaGraphNode {generation: $generation})-[old:SUPRNOVA_GRAPH_EDGE {generation: $generation, key: fact.key}]->() WITH fact, collect(old) AS previous, coalesce(min(old.created_at), fact.created_at) AS created FOREACH (r IN previous | DELETE r) MERGE (s:SuprnovaGraphNode {generation: $generation, key: fact.source}) MERGE (t:SuprnovaGraphNode {generation: $generation, key: fact.target}) CREATE (s)-[e:SUPRNOVA_GRAPH_EDGE]->(t) SET e = fact, e.generation = $generation, e.created_at = created";

/// Revoke nodes without interpolating application values.
pub const REVOKE_NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.scope_json IS NOT NULL AND (($mode = $document_mode AND (n.key = $key OR n.provenance_source = $source)) OR ($mode = $before_mode AND ((n.key = $key AND n.source_version < $version) OR (n.provenance_source = $source AND n.provenance_version < $version))) OR ($mode = $relations_before_mode AND n.key <> $key AND n.provenance_source = $source AND n.provenance_version < $version) OR ($mode = $events_mode AND n.provenance_event IN $events)) RETURN n.key AS key";

/// Revoke edges without interpolating application values.
pub const REVOKE_EDGES: &str = "MATCH (s:SuprnovaGraphNode)-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->(t:SuprnovaGraphNode) WHERE s.key IN $removed OR t.key IN $removed OR s.scope_json IS NULL OR t.scope_json IS NULL OR ($mode = $document_mode AND (e.source = $key OR e.target = $key OR e.provenance_source = $source)) OR (($mode = $before_mode OR $mode = $relations_before_mode) AND e.provenance_source = $source AND e.provenance_version < $version) OR ($mode = $events_mode AND e.provenance_event IN $events) DELETE e RETURN count(e) AS removed";

/// Delete nodes without interpolating application values.
pub const DELETE_NODES: &str =
    "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.key IN $keys DETACH DELETE n";

/// Nodes without interpolating application values.
pub const NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.key IN $keys AND n.scope_json IS NOT NULL RETURN n AS fact ORDER BY n.key";

/// Outgoing without interpolating application values.
pub const OUTGOING: &str = "MATCH (s:SuprnovaGraphNode {generation: $generation})-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->() WHERE s.key IN $sources AND (size($kinds) = 0 OR e.kind IN $kinds) WITH s, e ORDER BY e.confidence_bps DESC, e.key WITH s, collect(e)[..$limit] AS edges UNWIND edges AS e RETURN e AS fact ORDER BY s.key, e.confidence_bps DESC, e.key";

/// Document nodes without interpolating application values.
pub const DOCUMENT_NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.target_index IS NOT NULL AND n.key > $after RETURN n AS fact ORDER BY n.key LIMIT $limit";

/// Digest nodes without interpolating application values.
pub const DIGEST_NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.scope_json IS NOT NULL RETURN n AS fact ORDER BY n.key";

/// Digest edges without interpolating application values.
pub const DIGEST_EDGES: &str = "MATCH ()-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->() RETURN e AS fact ORDER BY e.key";

/// Lookup indexes for Memgraph's label-property and edge-property syntax.
#[cfg(any(test, feature = "graph-memgraph"))]
pub const MEMGRAPH_INDEXES: &[&str] = &[
    "CREATE INDEX ON :SuprnovaGraphNode(generation)",
    "CREATE INDEX ON :SuprnovaGraphNode(key)",
    "CREATE INDEX ON :SuprnovaGraphGeneration(id)",
    "CREATE INDEX ON :SuprnovaGraphGeneration(state)",
    "CREATE EDGE INDEX ON :SUPRNOVA_GRAPH_EDGE(key)",
    "CREATE EDGE INDEX ON :SUPRNOVA_GRAPH_EDGE(generation)",
];

/// Idempotent Neo4j 5 range indexes, including relationship-property lookups.
#[cfg(any(test, feature = "graph-neo4j"))]
pub const NEO4J_INDEXES: &[&str] = &[
    "CREATE INDEX suprnova_graph_node_generation IF NOT EXISTS FOR (n:SuprnovaGraphNode) ON (n.generation)",
    "CREATE INDEX suprnova_graph_node_key IF NOT EXISTS FOR (n:SuprnovaGraphNode) ON (n.key)",
    "CREATE INDEX suprnova_graph_generation_id IF NOT EXISTS FOR (g:SuprnovaGraphGeneration) ON (g.id)",
    "CREATE INDEX suprnova_graph_generation_state IF NOT EXISTS FOR (g:SuprnovaGraphGeneration) ON (g.state)",
    "CREATE INDEX suprnova_graph_edge_key IF NOT EXISTS FOR ()-[r:SUPRNOVA_GRAPH_EDGE]-() ON (r.key)",
    "CREATE INDEX suprnova_graph_edge_generation IF NOT EXISTS FOR ()-[r:SUPRNOVA_GRAPH_EDGE]-() ON (r.generation)",
];
