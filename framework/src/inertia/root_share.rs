//! The `root` shared prop: the public root every page builds its URLs from.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;

use super::prop::{InertiaRequestExt, Prop};
use super::shared::InertiaSharedData;
use crate::error::FrameworkError;

/// Inertia shared-data provider for the public root - the `root` prop a
/// frontend builds every URL it posts to, visits or links from (PFX-012).
///
/// Behind a reverse proxy that serves the application under `/billing` and
/// strips that path, a page that posts to `/login` reaches the proxy's
/// `/login`, not the application's. A page that posts to
/// `` `${root}/login` `` reaches the application at every root, so one
/// frontend build runs at `/` and under `/billing`.
///
/// The prop is [`crate::url::root`] for the request: the empty string at
/// the host root, `/billing` under a trusted `X-Forwarded-Prefix: /billing`.
///
/// An application registers one shared-data provider, so `RootShare` can
/// carry another and share its props too:
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use suprnova::{App, LocaleShare, RootShare};
/// # fn ex() {
/// App::register_inertia_shared(Arc::new(RootShare::around(Arc::new(LocaleShare))));
/// # }
/// ```
///
/// A page then reads it from its props, for example with Vue's
/// `usePage().props.root`.
#[derive(Clone, Default)]
pub struct RootShare {
    inner: Option<Arc<dyn InertiaSharedData>>,
}

impl RootShare {
    /// A provider that shares the `root` prop and nothing else.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A provider that shares the `root` prop and every prop `inner`
    /// shares. `root` is written last, so an `inner` prop of the same name
    /// cannot replace it.
    #[must_use]
    pub fn around(inner: Arc<dyn InertiaSharedData>) -> Self {
        Self { inner: Some(inner) }
    }
}

#[async_trait]
impl InertiaSharedData for RootShare {
    async fn share(
        &self,
        req: &dyn InertiaRequestExt,
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut shared = match &self.inner {
            Some(inner) => inner.share(req, component).await?,
            None => IndexMap::new(),
        };
        shared.insert(
            "root".to_owned(),
            Prop::eager(serde_json::Value::String(crate::routing::url::root())),
        );
        Ok(shared)
    }
}
