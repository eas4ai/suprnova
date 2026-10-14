//! The controllers of the kit's pages. Each one opens the request's Live
//! document, mounts the page's islands from `live::mounts()`, and renders
//! the page's view (see `crate::views`).

pub mod auth;
pub mod dashboard;
pub mod email_verification;
pub mod errors;
pub mod home;
pub mod notes;
pub mod password_reset;
pub mod profile;
