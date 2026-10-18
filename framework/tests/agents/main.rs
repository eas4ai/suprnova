//! Integration tests for the `agents` module: tools, the MCP endpoint, the
//! callers and the testing surface (AGENT-001 to AGENT-004, AGENT-018). The
//! `agents` mechanism runs this binary.

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/own_process.rs"]
#[allow(dead_code)]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

mod fixture;

mod auth;
mod build;
mod callers;
mod dispatch;
mod endpoint;
mod marker;
mod registry;
mod schema;
mod surface;
