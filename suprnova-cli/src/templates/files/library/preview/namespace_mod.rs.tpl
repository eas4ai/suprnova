//! The `{namespace}` library's Live components, compiled from where they sit
//! in the library: each `#[path]` names one component's Rust file under
//! `../components/`, so nothing is copied into the preview and the next
//! build compiles whatever the file holds. Add a line for each Rust file a
//! new component carries.

#[path = "../../../../components/counter/counter.rs"]
pub mod counter;
