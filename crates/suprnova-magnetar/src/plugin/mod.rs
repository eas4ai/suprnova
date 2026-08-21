//! Generic, framework-neutral plugin SDK.

pub mod context;
pub mod effects;
pub mod error;
pub mod hooks;
pub mod registry;
pub mod routes;
pub mod wire;

pub use crate::abuse::{AbuseLimiter, AbusePolicy, Permit};
pub use crate::auth::FactorGate;
pub use context::{
    AuthStorage, BearerCredential, BeforeRequest, Encryptor, HookContext, HttpRequest,
    HttpResponse, HttpTransport, InitContext, LinkGenerator, MailDriver, MailMessage,
    PluginContext, RequestContext,
};
pub use effects::{Effect, EffectResponse, PluginResponse, ResponseEffect};
pub use error::{PluginError, PluginResult};
pub use hooks::{DurableLifecycleDelivery, LifecycleEvent, LifecycleEventKind, LifecycleHook};
pub use registry::{ErasedPluginFacade, Plugin, PluginRegistry, PluginRegistryBuilder};
pub use routes::RouteDescriptor;
pub use wire::{HttpMethod, Method, WireBody, WireRequest, WireResponse};
