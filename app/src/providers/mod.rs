//! Service providers for the application
//!
//! This module contains service providers that configure and bind services
//! to the application container.

pub mod auth_provider;

pub use auth_provider::DatabaseUserProvider;
