//! Recommendation contracts run against an isolated database.
#[cfg(feature = "graph-hydradb")]
#[path = "../support/env_lock.rs"]
mod env_lock;
#[cfg(feature = "graph-hydradb")]
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[cfg(feature = "graph-hydradb")]
#[path = "../support/own_process.rs"]
mod own_process;

mod budget;
mod events;
mod graph;
mod graph_drivers;
mod graph_ranking;
mod graph_reads;
mod publication;
mod schema;
mod starter;
mod starter_reads;
mod starter_similarity;
mod support;
mod vectors;
