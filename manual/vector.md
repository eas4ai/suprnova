# Vector

Suprnova ships a Laravel-shape `Vector` facade backed by one of four
drivers — in-process Memory, Qdrant, Pinecone, or MariaDB native
`VECTOR(N)` — picked explicitly at boot via `Vector::register`. The
facade is a thin layer over a `VectorDriver` trait, so custom backends
plug in the same way the built-ins do.

## Quickstart

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// Bootstrap (typically once at app start)
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// Use it
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## The contract

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` carries an arbitrary `String` id, an `embedding: Vec<f32>`, and freeform `metadata: serde_json::Value` (must be a JSON object or `null`). `VectorMatch` returns the original id, the backend's similarity score, and the same metadata shape.

The trait is intentionally small. When you need filter expressions on search, sparse vectors, scroll/list, snapshots, or quantization knobs, drop down to the driver's underlying SDK via its public `client()` trapdoor.

### Why Suprnova diverges

Laravel ships vectors only through Postgres `pgvector`. That's the
PHP-shape answer: pick one storage backend, hide it behind a single
driver, and call it done. Suprnova treats the choice as a configuration
concern. The same trait covers an in-process `HashMap` for tests,
a dedicated vector DB (Qdrant, Pinecone) when the embedding count
justifies the operational cost, and a relational backend (MariaDB
11.7+) when you'd rather keep vectors next to the rows that produced
them. Weaviate, Milvus, LanceDB, pgvector, and LibSQL queue up behind
real consumer demand — none are blocked by the trait shape.

When the rest of your app fits on one engine, MariaDB 11.7+ keeps
vectors alongside relational tables, JSON documents, and
system-versioned temporal data — fewer moving parts than running
Postgres + Redis + Qdrant separately. See [Deployment](deployment.md)
for the recommendation in context.

## Drivers

### Memory — `MemoryVectorDriver`

In-process driver backed by `HashMap`. Cosine similarity, dimension-mismatch points are silently skipped on query (so mixed-dim test data doesn't blow up), zero-vector queries error clearly.

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

Use in tests and dev. Each `MemoryVectorDriver::new()` instance is hermetic — no shared state between two new()s.

### Qdrant — `QdrantVectorDriver`

Talks to Qdrant over gRPC (default port 6334) via the official `qdrant-client` SDK.

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // default
    .with_auto_create(true);                // default

Vector::register("docs", Arc::new(driver));
```

For Qdrant Cloud:

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**ID mapping.** Qdrant requires point IDs to be either `u64` or a valid UUID. The framework bridges arbitrary strings with three rules:

1. If the string parses as `u64`, use the `Num(u64)` variant.
2. If the string is a valid UUID, use the `Uuid(String)` variant verbatim.
3. Otherwise, derive a deterministic v5 UUID from a stable namespace.

The caller's original string is stashed in the point's payload under the reserved key `__suprnova_id` (exported as `SUPRNOVA_ID_PAYLOAD_KEY`) and stripped from `VectorMatch.metadata` on retrieval. Power users who query Qdrant directly via `driver.client()` can filter on `__suprnova_id` to bridge framework writes with direct calls.

**Auto-create.** On first `upsert` for an unseen collection, the driver creates it with the dimension inferred from the first item and the configured distance metric (Cosine by default). Race-safe — concurrent upserters on the same fresh collection won't fail; whichever creates first wins, the other proceeds. Disable via `.with_auto_create(false)` to require explicit creation.

**Cache invalidation.** If a collection is dropped externally (or Qdrant restarts before persistence flushed), the driver detects the "not found" error on upsert, drops the cache entry, re-runs `ensure_collection`, and retries once.

**Trapdoor.** `driver.client()` returns the underlying `qdrant_client::Qdrant` — use it for filter expressions on search, scroll, snapshots, or other APIs not surfaced via the trait. `QdrantVectorDriver::resolve_point_id`, `build_point`, and `decode_match` let you mix direct and trait-routed calls without losing id translation.

**Local setup.** Run Qdrant via Docker:

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

Integration tests run via:

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone — `PineconeVectorDriver`

> **Feature-gated — off by default.** Enable with `cargo build --features vector-pinecone` (or add `features = ["vector-pinecone"]` under the `suprnova` dep in your `Cargo.toml`). The feature costs no extra dependencies — it gates compilation of the driver, nothing more — so it is off simply because most apps don't use Pinecone and shouldn't pay to compile it.

Talks to Pinecone over its REST API, using the HTTP client the framework already carries.

> **Why not the official SDK?** The driver used to wrap `pinecone-sdk`, which speaks gRPC. That crate's newest release (0.1.2, published 2024-09-06) pins `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`, and `rustls-webpki 0.102` carries four RustSec advisories that are all fixed upstream in `>= 0.103.13`. One abandoned crate held the whole tree back, with no version of "wait for upstream" that was going to end. Pinecone exposes every operation this driver needs over HTTPS, so the REST route removed four advisories and two dependencies at once.

```rust
use suprnova::PineconeVectorDriver;

// API key directly
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// Or via env: PINECONE_API_KEY, plus optional PINECONE_CONTROLLER_HOST
// and PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// Bind to a non-default namespace
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

The store name passed via `Vector::store(name)` maps to a Pinecone index name. The driver resolves that index's host lazily on first use via the control plane's `GET /indexes/{name}`, then caches it. Skip the round trip by pinning the host you already know:

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

A host learned from the control plane is always contacted over `https`, whatever the response says. A host pinned through `with_index_host` keeps the scheme you gave it, so a local emulator on `http://` works.

**API version.** Pinecone versions its REST API by date and wants that version pinned in a header. The driver pins `2025-04` — the version its request and response shapes were written and tested against — and exposes `with_api_version` (or `PINECONE_API_VERSION`) for moving deliberately. It does not float: the namespace-key convention in `describe_index_stats` is one of the things that has changed between versions, and `count()` reads that map.

**No auto-create.** Pinecone index creation requires picking cloud (AWS/GCP/Azure), region, vector dimension, distance metric, and deletion-protection — too many trade-offs to default well. Create indexes via the Pinecone console, the Pinecone CLI, or a `control_plane_post` call before registering, then point the framework at the existing name.

This is the principal asymmetry with the Qdrant driver, which auto-creates collections on first upsert.

**IDs and metadata.** Pinecone accepts arbitrary `String` ids natively, so `VectorItem::id` passes straight through. Metadata is carried as JSON end to end — `PineconeVectorDriver::metadata_from_json` / `metadata_to_json` only enforce the framework's own rule that metadata is an object or null. Pinecone itself restricts metadata *values* to strings, numbers, booleans and lists of strings, and rejects nested objects server-side; the driver doesn't re-implement that check, because Pinecone's rules are versioned and a local copy would drift.

**Batch limits.** Pinecone documents a maximum of 1000 vectors per upsert and 1000 ids per delete. The driver sends what you give it in one request rather than chunking silently — a partial-success write is harder to reason about than a rejected one. Batch on your side if you exceed those limits.

**Namespaces.** One driver instance binds to one namespace. To use multiple namespaces of the same index, register one driver per namespace under different store names:

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**Throughput.** Nothing serializes. The driver caches a host string per index, not a connection handle, and requests share `reqwest`'s connection pool — so concurrent calls to the same index proceed concurrently. (The gRPC driver this replaces held one `Index` per name behind a `tokio::Mutex`, because `pinecone-sdk` exposed `Index` only behind `&mut self`.)

**Trapdoor.** `control_plane_get`, `control_plane_post` and `data_plane_post` reach any endpoint Pinecone ships, with your own request and response types, over the driver's authenticated and host-resolved transport — filter expressions, sparse vectors, fetch-by-id, `/vectors/list`, index management:

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**Tests.** Wire-contract tests run by default under the feature: they drive the driver against a local fake and assert the exact method, path, headers and JSON body it puts on the wire. Those pin the driver to Pinecone's *documented* contract. Confirming the documentation matches the live service needs the `#[ignore]`d integration tests, which require both env vars:

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB — `MariaDbVectorDriver`

Talks to MariaDB 11.7+ via direct `sqlx::MySqlPool`, using MariaDB's native `VECTOR(N)` column type and HNSW indexing. The first time you call a driver method, it runs `SELECT VERSION()` and rejects anything below 11.7 — older servers don't have the vector functions.

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // default

Vector::register("documents", Arc::new(driver));
```

`from_url` is lazy — it validates the URL syntax but does NOT open a connection until first use, so calling it at app bootstrap is safe even before the database is reachable. Wrap an existing pool with `MariaDbVectorDriver::from_pool(pool)` when you need custom pool options.

**Schema is yours.** The driver does not auto-create tables — schema is a migration concern. The recommended path is `driver.ensure_table_sql_for(name, dim)`, which inherits the driver's configured distance so the migration's `DISTANCE=` clause and the query function `similar` uses are guaranteed to match:

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// Result:
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

For migration generators that don't have a driver in scope (CLI tools, build scripts), use the static `MariaDbVectorDriver::ensure_table_sql(name, dim, distance)` and pass the same `MariaDbDistance` you'll later configure on the driver.

**Distance must match on both ends.** MariaDB silently falls back to a full table scan when the function used at query time doesn't match the index's `DISTANCE=` clause. The driver guards against this in two layers:

1. **`ensure_table_sql_for(name, dim)`** reads `self.distance` for both the emitted migration SQL and the runtime function in `similar` — they cannot drift apart by construction.
2. **A runtime check on first `similar` call** runs one `SHOW CREATE TABLE` per store, parses the actual `DISTANCE=` clause from the live schema, and errors clearly if it disagrees with `with_distance(...)`. Result is cached, so subsequent calls are zero-cost. This catches hand-written migrations or `from_pool` setups that bypass `ensure_table_sql_for`.

**Store-name safety.** Store names interpolate into emitted SQL (MySQL doesn't parameterize identifiers). Names are validated as `[A-Za-z_][A-Za-z0-9_]*` of length ≤ 64; the validated name is then backtick-quoted in every statement. Invalid names error with `FrameworkError::param` at the `register`/`upsert`/`similar`/`delete`/`count` boundary.

**IDs and metadata.** `VARCHAR(255)` accepts arbitrary `String` ids — no UUID derivation, no reserved payload keys. Metadata round-trips through MariaDB's `JSON` column type; `null` metadata stores as SQL `NULL`. Non-object metadata (arrays, primitives) is rejected with `FrameworkError::param` for parity with Qdrant and Pinecone.

**Score normalization.** MariaDB returns raw *distance* (lower = closer). The trait contract is *score* (higher = more similar) — the driver converts per metric:

| Metric    | MariaDB returns       | Exposed `score`              |
| --------- | --------------------- | ---------------------------- |
| Cosine    | `[0, 2]` (`1 - cos`)  | `1.0 - d / 2.0` → `[0, 1]`   |
| Euclidean | `[0, ∞)` L2 norm      | `1.0 / (1.0 + d)` → `(0, 1]` |

In both cases, ranking is preserved (best result first), but the absolute score values are NOT comparable across drivers — only the ordering is. Each backend lands on a `higher = better` convention, but the ranges differ: Memory's cosine returns `[-1, 1]`, MariaDB's normalized cosine returns `[0, 1]`, Qdrant emits its native cosine similarity in `[-1, 1]`, and Pinecone returns the raw similarity for whichever metric the index was created with. Use `score` to sort within a single driver's result set; don't compare numeric scores across drivers without re-normalizing yourself.

**Trapdoor.** `driver.pool()` returns the underlying `sqlx::MySqlPool` for raw queries the trait doesn't cover. `MariaDbVectorDriver::embedding_to_vec_text`, `score_from_distance`, and `ensure_table_sql` are pure functions you can call independently when mixing direct SQL with trait-routed calls.

**Bulk upsert behavior.** `upsert` emits one multi-row `INSERT ... VALUES (...), (...), ...` statement per 500-row chunk, all wrapped in a single transaction. Network round-trips drop ~500x vs per-row inserts when loading a fresh corpus; the call stays atomic across the whole batch. The batch size is internal — call `upsert` once with all your items and the driver handles chunking.

**HNSW indexes rebuild at commit time.** MariaDB updates the HNSW graph as rows go in, but the index work concentrates at commit. A 1M-row `upsert` will hold the transaction open for the full duration of the index build, which can be minutes. For very large initial loads, break the corpus into 10k–100k-row batches and call `upsert` repeatedly so each batch commits and frees the lock between rounds. (Smaller `upsert` calls are not slower per row — they just spread the index work into more commit points.)

**Dimension is pinned at table creation.** `VECTOR(N)` fixes the dimension; switching embedding models from a 768-dim model to a 1536-dim model means a full table migration (new table, re-embed, swap). Plan model upgrades the same way you'd plan a schema migration — there is no "ALTER COLUMN VECTOR(768) → VECTOR(1536)" path.

**Pool sizing.** `from_url` uses sqlx's default `MySqlPoolOptions` — `max_connections = 10` at the time of writing. For high-QPS workloads (hundreds of `similar` calls per second), build the pool yourself with `MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` and pass to `from_pool`. The driver doesn't impose its own connection cap.

**Local setup.** Run MariaDB 11.7+ via Docker:

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

Integration tests run via:

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## Driver comparison

| Aspect | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| Backing store | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| Persistence | None | Yes | Yes | Yes |
| Auto-create | n/a | Yes (configurable) | No (user creates index) | No (migration is yours) |
| String IDs | Native | Hashed to UUID-5 | Native | Native |
| Metadata key reserved | None | `__suprnova_id` | None | None |
| Throughput | Per-process | Concurrent | Concurrent (pool-bounded) | Concurrent (pool-bounded) |
| Distance metric | Cosine | Configurable | Set at index creation | Cosine / Euclidean |
| Version requirement | — | Any | Any | **11.7+** |

## Operational notes

**Store name conventions.** The store name passed to `Vector::register` and `Vector::store` is a label — it can be any string. For Qdrant the framework uses it as the collection name; for Pinecone as the index name. Match the label to the backend's existing naming scheme.

**Re-registering** a name with a new driver instance is a last-write-wins operation by design — useful for swapping drivers in test harnesses without restarting the process.

**Test isolation.** Both Memory and registry-backed driver tests use timestamp-tagged unique store names to avoid collisions under parallel test runs.

**Error semantics.** `Vector::store(name)` returns `FrameworkError::not_found` for unregistered names. Driver-level failures (network, auth, dimension mismatch) come back as `FrameworkError::internal` or `FrameworkError::param` with the cause string in the display message.

## Extending

To add a fifth backend (Weaviate, Milvus, LanceDB, pgvector, LibSQL, ...):

1. Add a new `framework/src/vector/<backend>.rs` implementing `VectorDriver`.
2. Re-export the driver type from `framework/src/vector/mod.rs` and the crate root.
3. Mirror the Pinecone test split: pure-function tests and wire-contract tests (against a local `wiremock` fake) always run; integration tests are `#[ignore]`-gated behind env vars for credentials. The middle layer is the one that earns its keep — a backend nobody can reach from CI still has a wire format that a typo can break.

The trait is intentionally small so the bar to ship a new driver stays low. If a backend needs surface that doesn't fit (filter expressions, sparse vectors, hybrid search), expose it through a trapdoor on the driver — don't bloat the trait.

## Next

- [Deployment](deployment.md) — the MariaDB-as-default-production
  recommendation in context
- [Database](database.md) — multi-driver SeaORM setup, including
  MariaDB as a relational backend alongside vectors
- [Environment Variables](env-vars.md) — `QDRANT_URL`,
  `PINECONE_API_KEY`, `MARIADB_URL` and other driver env contracts
- [Cache](cache.md) — sibling facade with the same driver-trait shape
- [Laravel Parity Map](parity.md) — where vector search sits relative
  to Scout
