//! Integration tests for the `http` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

#[path = "../support/common.rs"]
mod common;

pub mod multipart_limits;
pub mod precognition;
pub mod redirect;
pub mod redirect_helpers;
pub mod request_accessors;
pub mod request_body_cap;
pub mod request_peer_ip;
pub mod response_headers;
pub mod streamed_responses;
pub mod uploads;
