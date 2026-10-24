//! Memgraph persistence with its own lookup indexes and shared Bolt operations.
use super::{bolt::BoltGraphStore, cypher};
use crate::FrameworkError;

/// Persists recommendation facts in Memgraph while the framework owns scope and ranking.
/// Clones share a pool isolated by endpoint, database and credentials.
#[derive(Clone)]
pub struct MemgraphGraphStore {
    inner: BoltGraphStore,
}

impl MemgraphGraphStore {
    /// Verifies authentication and installs the engine's lookup indexes before use.
    ///
    /// # Errors
    /// Returns an error for an invalid URL or a failed connection, login or index creation.
    pub async fn connect(
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        Ok(Self {
            inner: BoltGraphStore::connect(
                "memgraph",
                "memgraph",
                cypher::MEMGRAPH_INDEXES,
                url,
                username,
                password,
            )
            .await?,
        })
    }
}

super::bolt::delegate_graph_store!(MemgraphGraphStore);
