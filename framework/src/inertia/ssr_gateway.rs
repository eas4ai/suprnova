//! The SSR call as a driver (PAR-060): a trait bound in the container,
//! with the HTTP gateway as the default binding, so an application binds
//! its own, as Laravel's `Inertia\Ssr\Gateway` is swappable.
//!
//! The capabilities Laravel splits into `DisablesSsr`, `ExcludesSsrPaths`,
//! `ConfiguresSsrRequests` and `HasHealthCheck` are methods with defaults
//! here: a gateway that lacks one answers `false` or `None`, and the
//! `Inertia` facade reports that instead of pretending the setting took.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::config::SsrConfig;
use super::prop::InertiaRequestExt;
use super::runtime::{SsrCondition, SsrRequestConfigurator};
use super::ssr::SsrResponse;
use crate::error::FrameworkError;

/// How a first visit reaches the SSR worker.
///
/// Bind an implementation with `App::bind::<dyn SsrGateway>(Arc::new(..))`
/// to replace the HTTP gateway for every first visit; nothing bound means
/// [`HttpGateway`]. Each method takes the response's `SsrConfig`, since a
/// response built with its own configuration keeps that configuration's
/// SSR settings.
#[async_trait]
pub trait SsrGateway: Send + Sync {
    /// Render `page` for `request` through the worker. `Ok(None)` renders
    /// on the client, which is also what every failure the configuration
    /// does not turn into an error becomes.
    async fn dispatch(
        &self,
        config: &SsrConfig,
        request: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError>;

    /// Whether the worker answers `GET {url}/health` successfully. `None`
    /// when the gateway has no health check, which `ssr:check` reports as
    /// a failure of its own.
    async fn is_healthy(&self, config: &SsrConfig) -> Option<bool> {
        let _ = config;
        None
    }

    /// Set the condition that turns SSR off, replacing the configuration's
    /// switch (`Inertia::disable_ssr`). `false` when the gateway has no such
    /// capability, so the facade can say the setting took no effect.
    fn disable(&self, condition: SsrCondition) -> bool {
        let _ = condition;
        false
    }

    /// Add paths excluded from SSR with Laravel's `ExcludesPaths` rules
    /// (`Inertia::without_ssr`). `false` when the gateway has no such
    /// capability.
    fn except(&self, paths: Vec<String>) -> bool {
        let _ = paths;
        false
    }

    /// Install the function that adjusts the request sent to the worker
    /// (`Inertia::configure_ssr_request_using`). `false` when the gateway
    /// has no such capability.
    fn configure_request_using(&self, configure: SsrRequestConfigurator) -> bool {
        let _ = configure;
        false
    }
}

/// The default gateway: the worker over HTTP, with the run-time settings
/// the `Inertia` facade keeps on the active container's registry.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpGateway;

#[async_trait]
impl SsrGateway for HttpGateway {
    async fn dispatch(
        &self,
        config: &SsrConfig,
        request: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        super::ssr::render(config, request, page).await
    }

    fn disable(&self, condition: SsrCondition) -> bool {
        crate::App::inertia_registry()
            .runtime()
            .set_ssr_condition(condition);
        true
    }

    fn except(&self, paths: Vec<String>) -> bool {
        crate::App::inertia_registry()
            .runtime()
            .add_ssr_exclusions(paths);
        true
    }

    fn configure_request_using(&self, configure: SsrRequestConfigurator) -> bool {
        crate::App::inertia_registry()
            .runtime()
            .set_ssr_request_configurator(configure);
        true
    }
}

/// The gateway first visits dispatch through: the one the application
/// bound as `dyn SsrGateway`, else [`HttpGateway`].
pub fn gateway() -> Arc<dyn SsrGateway> {
    crate::App::make::<dyn SsrGateway>().unwrap_or_else(|| Arc::new(HttpGateway))
}
