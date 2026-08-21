//! Explicit plugin composition, validation, and dispatch.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::FutureExt;

use super::context::{BearerCredential, BeforeRequest, InitContext, PluginContext, RequestContext};
use super::error::{PluginError, PluginResult};
use super::hooks::{LifecycleEvent, LifecycleHook};
use super::routes::RouteDescriptor;
use super::wire::{WireRequest, WireResponse};
use crate::schema::AuthSchema;
use crate::sessions::WebSessionBinding;
/// Object-safe facade consumed by framework adapters that cannot name `S`.
#[async_trait]
pub trait ErasedPluginFacade: Send + Sync {
    /// Run the global before-request chain.
    async fn before_request(&self, request: &WireRequest) -> PluginResult<BeforeRequest>;
    /// Dispatch one request through the composed routes.
    async fn handle(&self, request: WireRequest) -> PluginResult<WireResponse>;
    /// Dispatch with an opaque host credential resolved through SessionQueries.
    async fn handle_bound(
        &self,
        request: WireRequest,
        credential: Option<BearerCredential>,
    ) -> PluginResult<WireResponse>;
    /// Dispatch after host web-binding resolution.
    async fn handle_web_binding(
        &self,
        request: WireRequest,
        binding: &WebSessionBinding,
    ) -> PluginResult<WireResponse>;
    /// Deliver one post-commit lifecycle event.
    async fn dispatch_lifecycle(&self, event: LifecycleEvent) -> PluginResult<()>;
    /// Return enabled route names.
    fn route_names(&self) -> Vec<String>;
}

/// Public plugin contract. The schema is monomorphized per host while the
/// registry stores object-safe `dyn Plugin<S>` values.
#[async_trait]
pub trait Plugin<S: AuthSchema>: Send + Sync {
    /// Stable registry key and provider route segment.
    fn name(&self) -> &str;
    /// Routes owned by this plugin. Disabled descriptors are omitted at mount.
    fn routes(&self) -> Vec<RouteDescriptor>;
    /// Validate config and warm plugin state at host boot.
    async fn init(&self, _context: InitContext<'_, S>) -> PluginResult<()> {
        Ok(())
    }
    /// Run in global middleware before route dispatch.
    async fn before_request(&self, _context: RequestContext<'_, S>) -> PluginResult<BeforeRequest> {
        Ok(BeforeRequest::Continue)
    }
    /// Handle a request selected by one of this plugin's route descriptors.
    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse>;
    /// Return post-commit lifecycle callbacks owned by this plugin.
    fn lifecycle_hooks(&self) -> Vec<Arc<dyn LifecycleHook<S>>> {
        Vec::new()
    }
}

struct RegisteredPlugin<S: AuthSchema> {
    plugin: Arc<dyn Plugin<S>>,
    routes: Vec<RouteDescriptor>,
}

/// Immutable, validated plugin engine handle.
pub struct PluginRegistry<S: AuthSchema> {
    context: PluginContext<S>,
    plugins: Vec<RegisteredPlugin<S>>,
    lifecycle_errors: Mutex<Vec<PluginError>>,
}
impl<S: AuthSchema> PluginRegistry<S> {
    /// Start explicit plugin composition.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(context: PluginContext<S>) -> PluginRegistryBuilder<S> {
        PluginRegistryBuilder {
            context,
            plugins: Vec::new(),
        }
    }

    /// Construct a registry from an already collected plugin list.
    pub async fn from_plugins(
        context: PluginContext<S>,
        plugins: Vec<Arc<dyn Plugin<S>>>,
    ) -> PluginResult<Self> {
        let mut builder = Self::new(context);
        for plugin in plugins {
            builder.plugins.push(plugin);
        }
        builder.build().await
    }

    /// Run all plugin initialization hooks.
    pub async fn init(&self) -> PluginResult<()> {
        for entry in &self.plugins {
            entry.plugin.init(InitContext::new(&self.context)).await?;
        }
        Ok(())
    }

    /// Run the global before-request chain in registration order.
    pub async fn before_request(&self, request: &WireRequest) -> PluginResult<BeforeRequest> {
        for entry in &self.plugins {
            let context = RequestContext::new(&self.context, request);
            match entry.plugin.before_request(context).await? {
                BeforeRequest::Continue => {}
                outcome => return Ok(outcome),
            }
        }
        Ok(BeforeRequest::Continue)
    }

    /// Dispatch a request to its unique enabled route without middleware.
    pub async fn handle(&self, request: WireRequest) -> PluginResult<WireResponse> {
        self.handle_bound(request, None).await
    }

    /// Dispatch a request with the host's optional session channel.
    pub async fn handle_bound(
        &self,
        request: WireRequest,
        credential: Option<BearerCredential>,
    ) -> PluginResult<WireResponse> {
        let session = match credential {
            Some(credential) => Some(
                self.context
                    .sessions()
                    .verify_bearer(credential.as_str())
                    .await?,
            ),
            None => None,
        };
        self.handle_with_session(request, session.as_ref()).await
    }

    /// Dispatch a web request after host binding resolution.
    pub async fn handle_web_binding(
        &self,
        request: WireRequest,
        binding: &WebSessionBinding,
    ) -> PluginResult<WireResponse> {
        let session = self.context.resolve_web_binding(binding).await?;
        self.handle_with_session(request, Some(&session)).await
    }

    async fn handle_with_session(
        &self,
        mut request: WireRequest,
        session: Option<&crate::sessions::VerifiedSession>,
    ) -> PluginResult<WireResponse> {
        for entry in &self.plugins {
            for route in &entry.routes {
                if route.method != request.method {
                    continue;
                }
                if let Some(captures) = route.match_path(&request.path) {
                    request.path_params.extend(captures);
                    let context = RequestContext::with_session(&self.context, &request, session);
                    return entry.plugin.handle(context).await;
                }
            }
        }
        Err(PluginError::RouteNotFound { path: request.path })
    }

    /// Deliver one committed event to every registered lifecycle hook.
    ///
    /// Delivery is post-commit and at-least-once. Every duplicate reaches the
    /// hook; idempotency belongs to the hook/host using `mutation_id`. Hook
    /// failures are recorded without undoing the committed mutation.
    pub async fn dispatch_lifecycle(&self, event: LifecycleEvent) -> PluginResult<()> {
        let mut first_error = None;
        for entry in &self.plugins {
            let hooks = entry.plugin.lifecycle_hooks();
            for (hook_index, hook) in hooks.iter().enumerate() {
                let result = std::panic::AssertUnwindSafe(hook.on_event(
                    super::context::HookContext::new(&self.context),
                    event.clone(),
                ))
                .catch_unwind()
                .await;
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some(PluginError::LifecyclePanic {
                        plugin: entry.plugin.name().to_owned(),
                        hook: hook_index,
                    }),
                };
                if let Some(error) = error {
                    if first_error.is_none() {
                        first_error = Some(error.clone());
                    }
                    self.lifecycle_errors
                        .lock()
                        .expect("lifecycle error lock poisoned")
                        .push(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Drain recorded lifecycle failures for host logging/metrics.
    pub fn take_lifecycle_errors(&self) -> Vec<PluginError> {
        std::mem::take(
            &mut *self
                .lifecycle_errors
                .lock()
                .expect("lifecycle error lock poisoned"),
        )
    }

    /// Return enabled route names for host mounting and diagnostics.
    pub fn route_names(&self) -> Vec<String> {
        self.plugins
            .iter()
            .flat_map(|entry| entry.routes.iter().map(|route| route.name.clone()))
            .collect()
    }
}

/// Mutable builder that validates an engine before returning an immutable handle.
pub struct PluginRegistryBuilder<S: AuthSchema> {
    context: PluginContext<S>,
    plugins: Vec<Arc<dyn Plugin<S>>>,
}

impl<S: AuthSchema> PluginRegistryBuilder<S> {
    /// Register one plugin instance.
    pub fn register<P>(mut self, plugin: P) -> Self
    where
        P: Plugin<S> + 'static,
    {
        self.plugins.push(Arc::new(plugin));
        self
    }

    /// Register an already erased plugin instance.
    pub fn register_arc(mut self, plugin: Arc<dyn Plugin<S>>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Validate names and routes, initialize no user state, and freeze the set.
    pub async fn build(self) -> PluginResult<PluginRegistry<S>> {
        let mut plugin_names = HashSet::new();
        let mut route_names = HashSet::new();
        let mut entries: Vec<RegisteredPlugin<S>> = Vec::with_capacity(self.plugins.len());
        for plugin in self.plugins {
            let name = plugin.name().to_owned();
            if name.trim().is_empty() {
                return Err(PluginError::InvalidComposition {
                    plugin: name,
                    message: "plugin name must not be empty".to_owned(),
                });
            }
            if !plugin_names.insert(name.clone()) {
                return Err(PluginError::InvalidComposition {
                    plugin: name,
                    message: "duplicate plugin name".to_owned(),
                });
            }
            let mut enabled_routes = Vec::new();
            for route in plugin.routes() {
                if !route.enabled {
                    continue;
                }
                if route.name.trim().is_empty() {
                    return Err(PluginError::InvalidComposition {
                        plugin: name.clone(),
                        message: "route name must not be empty".to_owned(),
                    });
                }
                if enabled_routes
                    .iter()
                    .any(|existing: &RouteDescriptor| existing.overlaps(&route))
                    || entries.iter().any(|entry| {
                        entry
                            .routes
                            .iter()
                            .any(|existing| existing.overlaps(&route))
                    })
                {
                    return Err(PluginError::InvalidComposition {
                        plugin: name.clone(),
                        message: format!("overlapping route template at {}", route.path),
                    });
                }
                if !route_names.insert(route.name.clone()) {
                    return Err(PluginError::InvalidComposition {
                        plugin: name.clone(),
                        message: format!("duplicate route name {}", route.name),
                    });
                }
                enabled_routes.push(route);
            }
            entries.push(RegisteredPlugin {
                plugin,
                routes: enabled_routes,
            });
        }
        Ok(PluginRegistry {
            context: self.context,
            plugins: entries,
            lifecycle_errors: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl<S: AuthSchema> ErasedPluginFacade for PluginRegistry<S> {
    async fn before_request(&self, request: &WireRequest) -> PluginResult<BeforeRequest> {
        PluginRegistry::before_request(self, request).await
    }

    async fn handle(&self, request: WireRequest) -> PluginResult<WireResponse> {
        PluginRegistry::handle(self, request).await
    }
    async fn handle_bound(
        &self,
        request: WireRequest,
        credential: Option<BearerCredential>,
    ) -> PluginResult<WireResponse> {
        PluginRegistry::handle_bound(self, request, credential).await
    }
    async fn handle_web_binding(
        &self,
        request: WireRequest,
        binding: &WebSessionBinding,
    ) -> PluginResult<WireResponse> {
        PluginRegistry::handle_web_binding(self, request, binding).await
    }
    async fn dispatch_lifecycle(&self, event: LifecycleEvent) -> PluginResult<()> {
        PluginRegistry::dispatch_lifecycle(self, event).await
    }

    fn route_names(&self) -> Vec<String> {
        PluginRegistry::route_names(self)
    }
}
