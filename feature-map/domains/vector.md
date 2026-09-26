# Feature map: `manual/vector.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 72 checked.

## Rust API: suprnova

### `suprnova::vector::driver`

- [ ] struct `suprnova::VectorItem` · framework/src/vector/driver.rs:9 (also `suprnova::vector::VectorItem`, `suprnova::vector::driver::VectorItem`)
  - Public fields: `id`, `embedding`, `metadata`
  - [ ] fn `suprnova::VectorItem::new` · framework/src/vector/driver.rs:22
- [ ] struct `suprnova::VectorMatch` · framework/src/vector/driver.rs:33 (also `suprnova::vector::VectorMatch`, `suprnova::vector::driver::VectorMatch`)
  - Public fields: `id`, `score`, `metadata`
- [ ] trait `suprnova::VectorDriver` · framework/src/vector/driver.rs:64 (also `suprnova::vector::VectorDriver`, `suprnova::vector::driver::VectorDriver`)
  - Implemented here by: `MariaDbVectorDriver`, `MemoryVectorDriver`, `PineconeVectorDriver`, `QdrantVectorDriver`
  - [ ] fn `suprnova::VectorDriver::upsert` · framework/src/vector/driver.rs:67 (required)
  - [ ] fn `suprnova::VectorDriver::similar` · framework/src/vector/driver.rs:71 (required)
  - [ ] fn `suprnova::VectorDriver::delete` · framework/src/vector/driver.rs:80 (required)
  - [ ] fn `suprnova::VectorDriver::count` · framework/src/vector/driver.rs:85 (required)

### `suprnova::vector::mariadb` (feature: `vector-mariadb`)

- [ ] struct `suprnova::MariaDbVectorDriver` · framework/src/vector/mariadb.rs:159 (also `suprnova::vector::MariaDbVectorDriver`, `suprnova::vector::mariadb::MariaDbVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::MariaDbVectorDriver::from_pool` · framework/src/vector/mariadb.rs:188
  - [ ] fn `suprnova::MariaDbVectorDriver::from_url` · framework/src/vector/mariadb.rs:204
  - [ ] fn `suprnova::MariaDbVectorDriver::with_distance` · framework/src/vector/mariadb.rs:212
  - [ ] fn `suprnova::MariaDbVectorDriver::pool` · framework/src/vector/mariadb.rs:219
  - [ ] fn `suprnova::MariaDbVectorDriver::distance` · framework/src/vector/mariadb.rs:224
  - [ ] fn `suprnova::MariaDbVectorDriver::validate_store_name` · framework/src/vector/mariadb.rs:234
  - [ ] fn `suprnova::MariaDbVectorDriver::ensure_table_sql_for` · framework/src/vector/mariadb.rs:272
  - [ ] fn `suprnova::MariaDbVectorDriver::ensure_table_sql` · framework/src/vector/mariadb.rs:291
  - [ ] fn `suprnova::MariaDbVectorDriver::embedding_to_vec_text` · framework/src/vector/mariadb.rs:321
  - [ ] fn `suprnova::MariaDbVectorDriver::score_from_distance` · framework/src/vector/mariadb.rs:351
- [ ] enum `suprnova::MariaDbDistance` · framework/src/vector/mariadb.rs:129 (also `suprnova::vector::MariaDbDistance`, `suprnova::vector::mariadb::MariaDbDistance`)
  - Variants: `Cosine`, `Euclidean`
  - [ ] fn `suprnova::MariaDbDistance::index_clause` · framework/src/vector/mariadb.rs:140
  - [ ] fn `suprnova::MariaDbDistance::fn_name` · framework/src/vector/mariadb.rs:150

### `suprnova::vector::memory`

- [ ] struct `suprnova::MemoryVectorDriver` · framework/src/vector/memory.rs:19 (also `suprnova::vector::MemoryVectorDriver`, `suprnova::vector::memory::MemoryVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::MemoryVectorDriver::new` · framework/src/vector/memory.rs:27

### `suprnova::vector::pinecone` (feature: `vector-pinecone`, off by default)

- [ ] struct `suprnova::vector::PineconeMatch` · framework/src/vector/pinecone.rs:160 (also `suprnova::vector::pinecone::PineconeMatch`)
  - Public fields: `id`, `score`, `metadata`
- [ ] struct `suprnova::vector::PineconeVector` · framework/src/vector/pinecone.rs:148 (also `suprnova::vector::pinecone::PineconeVector`)
  - Public fields: `id`, `values`, `metadata`
- [ ] struct `suprnova::PineconeVectorDriver` · framework/src/vector/pinecone.rs:174 (also `suprnova::vector::PineconeVectorDriver`, `suprnova::vector::pinecone::PineconeVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::PineconeVectorDriver::from_api_key` · framework/src/vector/pinecone.rs:204
  - [ ] fn `suprnova::PineconeVectorDriver::from_env` · framework/src/vector/pinecone.rs:229
  - [ ] fn `suprnova::PineconeVectorDriver::with_namespace` · framework/src/vector/pinecone.rs:248
  - [ ] fn `suprnova::PineconeVectorDriver::with_control_plane` · framework/src/vector/pinecone.rs:257
  - [ ] fn `suprnova::PineconeVectorDriver::with_api_version` · framework/src/vector/pinecone.rs:263
  - [ ] fn `suprnova::PineconeVectorDriver::with_index_host` · framework/src/vector/pinecone.rs:276
  - [ ] fn `suprnova::PineconeVectorDriver::namespace` · framework/src/vector/pinecone.rs:287
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane` · framework/src/vector/pinecone.rs:292
  - [ ] fn `suprnova::PineconeVectorDriver::api_version` · framework/src/vector/pinecone.rs:297
  - [ ] fn `suprnova::PineconeVectorDriver::metadata_from_json` · framework/src/vector/pinecone.rs:309
  - [ ] fn `suprnova::PineconeVectorDriver::metadata_to_json` · framework/src/vector/pinecone.rs:324
  - [ ] fn `suprnova::PineconeVectorDriver::build_vector` · framework/src/vector/pinecone.rs:332
  - [ ] fn `suprnova::PineconeVectorDriver::decode_match` · framework/src/vector/pinecone.rs:341
  - [ ] fn `suprnova::PineconeVectorDriver::index_host` · framework/src/vector/pinecone.rs:363
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane_get` · framework/src/vector/pinecone.rs:392
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane_post` · framework/src/vector/pinecone.rs:404
  - [ ] fn `suprnova::PineconeVectorDriver::data_plane_post` · framework/src/vector/pinecone.rs:420
- [ ] const `suprnova::vector::DEFAULT_API_VERSION` · framework/src/vector/pinecone.rs:125 (also `suprnova::vector::pinecone::DEFAULT_API_VERSION`)
- [ ] const `suprnova::vector::DEFAULT_CONTROL_PLANE` · framework/src/vector/pinecone.rs:114 (also `suprnova::vector::pinecone::DEFAULT_CONTROL_PLANE`)

### `suprnova::vector::qdrant`

- [ ] struct `suprnova::QdrantVectorDriver` · framework/src/vector/qdrant.rs:104 (also `suprnova::vector::QdrantVectorDriver`, `suprnova::vector::qdrant::QdrantVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::QdrantVectorDriver::from_client` · framework/src/vector/qdrant.rs:115
  - [ ] fn `suprnova::QdrantVectorDriver::from_url` · framework/src/vector/qdrant.rs:126
  - [ ] fn `suprnova::QdrantVectorDriver::from_url_with_api_key` · framework/src/vector/qdrant.rs:135
  - [ ] fn `suprnova::QdrantVectorDriver::with_auto_create` · framework/src/vector/qdrant.rs:148
  - [ ] fn `suprnova::QdrantVectorDriver::with_distance` · framework/src/vector/qdrant.rs:155
  - [ ] fn `suprnova::QdrantVectorDriver::client` · framework/src/vector/qdrant.rs:163
  - [ ] fn `suprnova::QdrantVectorDriver::resolve_point_id` · framework/src/vector/qdrant.rs:170
  - [ ] fn `suprnova::QdrantVectorDriver::build_point` · framework/src/vector/qdrant.rs:189
  - [ ] fn `suprnova::QdrantVectorDriver::decode_match` · framework/src/vector/qdrant.rs:216
- [ ] enum `suprnova::QdrantDistance` · framework/src/vector/qdrant.rs:79 (also `suprnova::vector::QdrantDistance`, `suprnova::vector::qdrant::QdrantDistance`)
  - Variants: `Cosine`, `Euclidean`, `Dot`, `Manhattan`
- [ ] const `suprnova::SUPRNOVA_ID_PAYLOAD_KEY` · framework/src/vector/qdrant.rs:68 (also `suprnova::vector::SUPRNOVA_ID_PAYLOAD_KEY`, `suprnova::vector::qdrant::SUPRNOVA_ID_PAYLOAD_KEY`)

### `suprnova::vector::registry`

- [ ] struct `suprnova::VectorRegistry` · framework/src/vector/registry.rs:19 (also `suprnova::vector::VectorRegistry`, `suprnova::vector::registry::VectorRegistry`)
  - [ ] fn `suprnova::VectorRegistry::install` · framework/src/vector/registry.rs:30
  - [ ] fn `suprnova::VectorRegistry::lookup` · framework/src/vector/registry.rs:45
  - [ ] fn `suprnova::VectorRegistry::names` · framework/src/vector/registry.rs:64
- [ ] struct `suprnova::VectorStore` · framework/src/vector/registry.rs:84 (also `suprnova::vector::VectorStore`, `suprnova::vector::registry::VectorStore`)
  - [ ] fn `suprnova::VectorStore::name` · framework/src/vector/registry.rs:91
  - [ ] fn `suprnova::VectorStore::upsert` · framework/src/vector/registry.rs:96
  - [ ] fn `suprnova::VectorStore::similar` · framework/src/vector/registry.rs:101
  - [ ] fn `suprnova::VectorStore::delete` · framework/src/vector/registry.rs:110
  - [ ] fn `suprnova::VectorStore::count` · framework/src/vector/registry.rs:119

### `suprnova::vector`

- [ ] struct `suprnova::Vector` · framework/src/vector/mod.rs:61 (also `suprnova::vector::Vector`)
  - [ ] fn `suprnova::Vector::register` · framework/src/vector/mod.rs:68
  - [ ] fn `suprnova::Vector::store` · framework/src/vector/mod.rs:75
  - [ ] fn `suprnova::Vector::registered_names` · framework/src/vector/mod.rs:80
