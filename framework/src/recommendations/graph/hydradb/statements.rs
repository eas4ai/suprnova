//! HydraDB's native subset has numeric identities, scalar projections and auto indexes.

/// Establish the head label without resetting a published generation on reconnect.
pub const HEAD: &str = "UNWIND $facts AS fact MERGE (n {id: fact.vertex}) SET n:SuprnovaGraphHead";
/// Read the single publication pointer rather than changing every generation's state.
pub const READ_HEAD: &str =
    "MATCH (n:SuprnovaGraphHead) RETURN n.generation AS generation, n.previous AS previous";
/// Switch both pointers in one durable statement so readers never see a partial build.
pub const PUBLISH: &str = "MATCH (n:SuprnovaGraphHead), (g:SuprnovaGraphGeneration {generation: $generation}) SET n.generation = $generation, n.previous = $previous, g.state = $state";
/// Register a build through HydraDB's supported vertex upsert form.
pub const BEGIN: &str = "UNWIND $facts AS fact MERGE (n {id: fact.vertex}) SET n:SuprnovaGraphGeneration, n.generation = fact.generation, n.state = fact.state";
/// Determine whether a build exists before accepting publication.
pub const GENERATION: &str = "MATCH (n:SuprnovaGraphGeneration {generation: $generation}) RETURN n.generation AS generation, n.state AS state";
/// Delete a generation's vertices, which also deletes its incident relationships.
pub const DISCARD_NODES: &str =
    "MATCH (n:SuprnovaGraphNode {generation: $generation}) DETACH DELETE n";
/// Remove the build record only after its facts have gone.
pub const DISCARD_GENERATION: &str =
    "MATCH (n:SuprnovaGraphGeneration {generation: $generation}) DETACH DELETE n";
/// Save complete nodes as scalars and retain the original typed fact in JSON.
pub const PUT_NODES: &str = "UNWIND $facts AS fact MERGE (n {id: fact.vertex}) SET n:SuprnovaGraphNode, n.generation = fact.generation, n.key = fact.key, n.materialized = fact.materialized, n.document = fact.document, n.json = fact.json, n.source_version = fact.source_version, n.provenance_source = fact.provenance_source, n.provenance_version = fact.provenance_version, n.event = fact.event";
/// Edges may precede nodes; placeholders never overwrite materialized node facts.
pub const PLACEHOLDERS: &str = "UNWIND $facts AS fact MERGE (n {id: fact.vertex}) SET n:SuprnovaGraphNode, n.generation = fact.generation, n.key = fact.key";
/// Upsert a relationship by numeric identity without using map replacement or FOREACH.
pub const PUT_EDGES: &str = "UNWIND $facts AS fact MATCH (s:SuprnovaGraphNode {id: fact.source_vertex}), (t:SuprnovaGraphNode {id: fact.target_vertex}) MERGE (s)-[e:SUPRNOVA_GRAPH_EDGE {id: fact.vertex}]->(t) SET e.generation = fact.generation, e.key = fact.key, e.kind = fact.kind, e.confidence = fact.confidence, e.json = fact.json, e.source = fact.source, e.target = fact.target, e.provenance_source = fact.provenance_source, e.provenance_version = fact.provenance_version, e.event = fact.event";
/// Fetch named vertices in one request; absent placeholder JSON decodes as null.
pub const NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation})";
/// Page documents by key; boolean flags replace unsupported IS NOT NULL predicates.
pub const DOCUMENTS: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.document = true AND n.key > $after RETURN n.json AS json, n.key AS key ORDER BY key LIMIT $limit";
/// Page typed node facts without loading placeholders.
pub const ALL_NODES: &str = "MATCH (n:SuprnovaGraphNode {generation: $generation}) WHERE n.materialized = true AND n.key > $after RETURN n.json AS json, n.key AS key ORDER BY key LIMIT $limit";
/// Page relationship facts for digests and provenance revocations.
pub const ALL_EDGES: &str = "MATCH (s)-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->(t) WHERE e.key > $after RETURN e.json AS json, e.key AS key ORDER BY key LIMIT $limit";
/// Remove revoked vertices and their incident edges in a bounded batch.
pub const DELETE_NODES: &str = "UNWIND $facts AS fact MATCH (n {id: fact.vertex}) DETACH DELETE n";
/// Inspect the complete stored key before writing a numeric identity.
pub const IDENTITY: &str = "MATCH (n:SuprnovaGraphNode)";
/// Bound each source independently; kind predicates are added with bound parameters.
pub const OUTGOING: &str = "MATCH (s:SuprnovaGraphNode {id: $source, key: $source_key, generation: $generation})-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->(t)";

/// HydraDB maintains label, vertex-property and relationship-property indexes on writes.
/// Its public query API rejects CREATE INDEX, so no DDL is sent or silently ignored.
pub const INDEX_DDL: &[&str] = &[];

/// Delete selected edges in one durable batch before detaching their revoked vertices.
pub const DELETE_EDGES: &str =
    "UNWIND $facts AS fact MATCH ()-[e:SUPRNOVA_GRAPH_EDGE {id: fact.vertex}]->() DELETE e";

/// Read a batch of edge facts before preserving their original creation times.
pub const EDGES_BY_KEY: &str = "MATCH (s)-[e:SUPRNOVA_GRAPH_EDGE {generation: $generation}]->(t)";
