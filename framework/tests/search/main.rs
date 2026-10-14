//! Search core behavior and driver conformance share one integration binary.
#[cfg(feature = "testing")]
mod builder;
mod core;
#[cfg(feature = "testing")]
mod derive;
mod embedder;
mod engine;
mod expansion;
mod fusion;
mod highlight;
#[cfg(feature = "testing")]
mod indexing;
mod query;
