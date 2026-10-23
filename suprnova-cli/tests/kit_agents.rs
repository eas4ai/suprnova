//! AGENT-017 and AGENT-007 for the kits: every new project carries its own
//! agent auth signing key, and every kit wires the MCP endpoint, the OAuth
//! server and agent auth with its own consent pages.
//!
//! The binary's root declares each module with a `#[path]` line: a test
//! crate's root resolves `mod shared;` beside itself, not under
//! `kit_agents/`. Each kit family's module sits in that directory.
//!
//! Nothing here compiles a scaffold. The `agents` mechanism runs this binary
//! with `-E 'test(/agent_017_/)'`.

#[path = "kit_agents/api.rs"]
mod api;
#[path = "kit_agents/inertia.rs"]
mod inertia;
#[path = "kit_agents/shared.rs"]
mod shared;

#[path = "kit_agents/live.rs"]
mod live;

#[path = "kit_agents/tools.rs"]
mod tools;
