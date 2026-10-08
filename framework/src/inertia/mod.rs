//! [Inertia.js](https://inertiajs.com/) server adapter.
//!
//! Lets controllers return a typed page payload - component name plus
//! props - that Inertia turns into a full SPA page on initial load and a
//! JSON visit on subsequent navigations. Handles props, partial reloads,
//! deferred / lazy / encrypted history props, shared data, asset
//! versioning, flash messages, and SSR.

mod config;
mod conversion_middleware;
mod dotted;
mod encrypt_middleware;
mod error_page_middleware;
mod exceptions;
mod facade;
pub(crate) mod flash;
mod headers_middleware;
mod hooks;
mod manifest;
mod pages;
mod prop;
mod providers;
mod query_string;
mod response;
mod root_share;
mod root_template;
mod runtime;
mod shared;
pub(crate) mod ssr;
mod ssr_gateway;
mod validation_redirect_middleware;
mod version_middleware;
pub(crate) mod visit;

pub use config::{Frontend, InertiaConfig, MANIFEST_VERSION_FALLBACK, SsrConfig, VersionResolver};
pub use conversion_middleware::Inertia303Middleware;
pub use encrypt_middleware::EncryptHistoryMiddleware;
pub use error_page_middleware::InertiaErrorPageMiddleware;
pub(crate) use error_page_middleware::ServerErrorDecision;
pub use exceptions::InertiaErrorResponse;
pub use facade::Inertia;
pub use flash::FlashKey;
pub use headers_middleware::InertiaHeadersMiddleware;
pub use hooks::{
    DefaultInertiaHooks, InertiaMiddleware, InertiaMiddlewareHooks, InertiaVisit, PageUrlResolver,
};
pub use manifest::{ManifestEntry, ResolvedAssets, ViteManifest};
pub(crate) use prop::header_is_truthy;
pub use prop::{
    DeferOptions, InertiaRequestExt, MatchOnFields, MergeMode, MergeStrategy, OnceOptions,
    PartialFilter, Prop, PropFuture, PropResolver, ProvidesScrollMetadata, ScrollMetadata,
    Visibility,
};
pub use prop::{MergePaths, OnceUntil};
pub use providers::{
    PropertyContext, ProvidesInertiaProperties, ProvidesInertiaProperty, RenderContext,
};
pub(crate) use response::escape_html_attr;
pub use response::{InertiaLocation, InertiaResponse, IntoInertiaData, PropEntry};
pub use root_share::RootShare;
pub use root_template::{
    InertiaRoot, InertiaRootBody, InertiaRootHead, InertiaRootParts, InertiaRootTemplate,
    InertiaRootTitle, InertiaViewData, InertiaViewValue,
};
pub use runtime::{SsrCondition, SsrDisabledWhen, SsrRequestConfigurator};
pub use shared::SharedOnceProp;
pub use shared::{InertiaRegistry, InertiaSharedData};
pub use ssr::{
    CONVENTIONAL_BUNDLE_PATHS, SsrRequest, SsrResponse, detect_bundle as detect_ssr_bundle,
};
pub use ssr_gateway::{HttpGateway, SsrGateway, gateway as ssr_gateway};
pub use validation_redirect_middleware::InertiaValidationRedirectMiddleware;
pub use version_middleware::InertiaVersionMiddleware;

// Test helpers for setting up a flash scope outside of a real server.
// Production code never calls these - the flash scope is set up
// automatically by `Server::handle_request`.
#[doc(hidden)]
pub fn flash_new_bag_for_test()
-> std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>> {
    flash::new_bag()
}

#[doc(hidden)]
pub async fn flash_scope_for_test<F: std::future::Future>(
    bag: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>>,
    fut: F,
) -> F::Output {
    flash::FLASH_BAG.scope(bag, fut).await
}
