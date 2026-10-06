//! Lossless projection of engine-owned endpoint intent into Suprnova HTTP.

use std::sync::{Arc, Mutex};

use suprnova_live::action::ActionOutcome;
use suprnova_live::canonical::{CanonicalValue, to_canonical_bytes};
use suprnova_live::endpoint::{
    EndpointNavigationTarget, EndpointResponseIntents, LiveEndpointResponse,
};
use suprnova_live::execution::{
    HostError, HostErrorKind, ResponseIntentPreparationPort, ResponseIntentPreparationRequest,
};
use suprnova_live::limits::InputLimits;

use crate::{FrameworkError, HttpResponse};

pub(crate) struct SuprnovaResponseIntentPort {
    /// The configured redirect size (`LIVE_MAX_REDIRECT_BYTES`).
    max_redirect_bytes: usize,
}

pub(crate) struct PreparedResponseIntents {
    endpoint: EndpointResponseIntents,
    flash: PreparedFlashBatch,
}

impl PreparedResponseIntents {
    pub(crate) fn into_parts(self) -> (EndpointResponseIntents, PreparedFlashBatch) {
        (self.endpoint, self.flash)
    }
}

struct PreparedFlash {
    key: String,
    value: serde_json::Value,
}

pub(crate) struct PreparedFlashBatch(Vec<PreparedFlash>);

#[derive(Default)]
pub(crate) struct PreparedResponseCompletion {
    flash: Mutex<Option<Vec<PreparedFlash>>>,
}

pub(crate) struct RequestResponseIntentPort {
    resolver: Arc<SuprnovaResponseIntentPort>,
    completion: Arc<PreparedResponseCompletion>,
}

impl PreparedResponseCompletion {
    pub(crate) fn stage(&self, prepared: PreparedFlashBatch) -> Result<(), FrameworkError> {
        if prepared.0.is_empty() {
            return Ok(());
        }
        let mut staged = self
            .flash
            .lock()
            .map_err(|_| FrameworkError::internal("Live response completion was unavailable"))?;
        if staged.is_some() {
            return Err(FrameworkError::internal(
                "Live response completion was already staged",
            ));
        }
        *staged = Some(prepared.0);
        Ok(())
    }

    pub(crate) fn commit(&self) -> Result<(), FrameworkError> {
        let flash = self
            .flash
            .lock()
            .map_err(|_| FrameworkError::internal("Live response completion was unavailable"))?
            .take();
        let Some(flash) = flash else {
            return Ok(());
        };
        crate::session::session_mut(move |session| {
            for item in flash {
                session.flash(&item.key, item.value);
            }
        })
        .ok_or_else(|| {
            FrameworkError::internal("Live flash response requires an active session scope")
        })
    }
}

/// One redirect or reflected URL as a navigation target, refused with the
/// setting to raise when it is longer than the configured redirect size.
fn navigation_target(
    target: &str,
    max_redirect_bytes: usize,
    rejected: &'static str,
) -> Result<EndpointNavigationTarget, FrameworkError> {
    if target.len() > max_redirect_bytes {
        let limit = crate::live::LiveLimitExceeded::redirect_bytes(
            target.len() as u64,
            max_redirect_bytes as u64,
        );
        // The engine sees only a closed failure, so the setting to raise is
        // recorded here for the developer.
        tracing::warn!(limit = %limit, "{rejected}");
        return Err(FrameworkError::internal(limit.to_string()));
    }
    EndpointNavigationTarget::parse(target).map_err(|_| FrameworkError::internal(rejected))
}

impl SuprnovaResponseIntentPort {
    pub(crate) const fn new(max_redirect_bytes: usize) -> Self {
        Self { max_redirect_bytes }
    }

    pub(crate) fn bind(
        self: &Arc<Self>,
        completion: Arc<PreparedResponseCompletion>,
    ) -> RequestResponseIntentPort {
        RequestResponseIntentPort {
            resolver: Arc::clone(self),
            completion,
        }
    }

    pub(crate) fn resolve(
        &self,
        result: &suprnova_live::action::ActionResult,
        document_path: Option<&str>,
        protocol_version: u16,
    ) -> Result<PreparedResponseIntents, FrameworkError> {
        let metadata = result.metadata();
        let flash = metadata
            .flash()
            .iter()
            .map(|intent| {
                let encoded = to_canonical_bytes(intent.value(), &InputLimits::default())
                    .map_err(|_| FrameworkError::internal("Live flash value was rejected"))?;
                let value = serde_json::from_slice(&encoded)
                    .map_err(|_| FrameworkError::internal("Live flash value was rejected"))?;
                Ok(PreparedFlash {
                    key: intent.key().as_str().to_owned(),
                    value,
                })
            })
            .collect::<Result<Vec<_>, FrameworkError>>()?;

        let mut endpoint = EndpointResponseIntents::default();
        match result.outcome() {
            ActionOutcome::Redirect(intent) => {
                let target =
                    crate::routing::resolve_live_route(intent.route(), intent.parameters())
                        .map_err(|_| {
                            FrameworkError::internal("Live route intent could not be resolved")
                        })?;
                // The browser follows the target as given, so it carries
                // the public root as `route()` does (PFX-004).
                let target = crate::routing::root::prefixed(&target);
                endpoint = endpoint.with_redirect(navigation_target(
                    &target,
                    self.max_redirect_bytes,
                    "Live route target was rejected",
                )?);
            }
            ActionOutcome::Render | ActionOutcome::NoRender => {
                if let Some(intent) = metadata.url() {
                    if protocol_version != 2 {
                        return Err(FrameworkError::internal(
                            "Live URL reflection requires protocol v2",
                        ));
                    }
                    let path = document_path.ok_or_else(|| {
                        FrameworkError::internal("Live document path authority was unavailable")
                    })?;
                    let target = reflected_target(path, intent.query())?;
                    endpoint = endpoint.with_reflected_url(navigation_target(
                        &target,
                        self.max_redirect_bytes,
                        "Live reflected target was rejected",
                    )?);
                }
            }
        }
        Ok(PreparedResponseIntents {
            endpoint,
            flash: PreparedFlashBatch(flash),
        })
    }

    pub(crate) fn project(
        &self,
        response: LiveEndpointResponse,
    ) -> Result<HttpResponse, FrameworkError> {
        let mut projected = HttpResponse::bytes_body(response.body, "application/octet-stream")
            .without_header("content-type")
            .status(response.status.as_u16());
        for (name, value) in &response.headers {
            let value = value.to_str().map_err(|_| {
                FrameworkError::internal("Live endpoint response header was rejected")
            })?;
            projected = projected.header(name.as_str(), value);
        }
        Ok(projected)
    }
}

impl ResponseIntentPreparationPort for RequestResponseIntentPort {
    fn prepare<'a>(
        &'a self,
        request: ResponseIntentPreparationRequest<'a>,
    ) -> suprnova_live::component::LiveFuture<'a, Result<EndpointResponseIntents, HostError>> {
        Box::pin(async move {
            let authority = request.authority();
            let prepared = self
                .resolver
                .resolve(
                    request.result(),
                    authority.mounted_document_path(),
                    authority.protocol_version(),
                )
                .map_err(|_| HostError::new(HostErrorKind::ResponseIntent))?;
            let (endpoint, flash) = prepared.into_parts();
            if !flash.0.is_empty() && crate::session::session_mut(|_| ()).is_none() {
                return Err(HostError::new(HostErrorKind::ResponseIntent));
            }
            self.completion
                .stage(flash)
                .map_err(|_| HostError::new(HostErrorKind::ResponseIntent))?;
            Ok(endpoint)
        })
    }
}

/// The URL a Live action reflects for its document: the public root, the
/// document path, and the query.
///
/// The snapshot records the path the application received; the browser
/// compares the reflected URL with the path it shows, which carries the
/// public root (PFX-006). At the host root a document with no query reflects
/// its path itself, borrowed; otherwise the root, the path and the query are
/// written into one buffer, so the path is never copied on its own first
/// (MEM-003).
fn reflected_target<'a>(
    path: &'a str,
    query: &CanonicalValue,
) -> Result<std::borrow::Cow<'a, str>, FrameworkError> {
    let CanonicalValue::Object(query) = query else {
        return Err(FrameworkError::internal("Live URL query was rejected"));
    };
    let root = crate::routing::root::current();
    if query.is_empty() {
        if root.is_empty() {
            return Ok(std::borrow::Cow::Borrowed(path));
        }
        let mut target = String::with_capacity(root.len() + path.len());
        target.push_str(&root);
        target.push_str(path);
        return Ok(std::borrow::Cow::Owned(target));
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        let value = match value {
            CanonicalValue::String(value) => value.clone(),
            CanonicalValue::Bool(value) => value.to_string(),
            CanonicalValue::Number(_) => {
                let encoded = to_canonical_bytes(value, &InputLimits::default())
                    .map_err(|_| FrameworkError::internal("Live URL query was rejected"))?;
                String::from_utf8(encoded)
                    .map_err(|_| FrameworkError::internal("Live URL query was rejected"))?
            }
            CanonicalValue::Null | CanonicalValue::Array(_) | CanonicalValue::Object(_) => {
                return Err(FrameworkError::internal("Live URL query was rejected"));
            }
        };
        serializer.append_pair(key, &value);
    }
    Ok(std::borrow::Cow::Owned(format!(
        "{root}{path}?{}",
        serializer.finish()
    )))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use suprnova_live::canonical::CanonicalValue;

    use super::{navigation_target, reflected_target};

    /// MEM-003: at the host root the reflected URL of a document with no
    /// query is its path itself, not a copy of it; under a root it is one
    /// new buffer, the root followed by the path.
    #[tokio::test]
    async fn mem_audit_a_reflected_url_at_the_host_root_borrows_the_document_path() {
        let path = format!("/docs/{}", "a".repeat(4_000));
        let empty = CanonicalValue::Object(BTreeMap::new());
        crate::routing::root::scope(Arc::from(""), async {
            let target = reflected_target(&path, &empty).expect("a reflected URL");
            assert_eq!(target, path.as_str());
            assert!(
                std::ptr::eq(target.as_ptr(), path.as_ptr()),
                "the document path was copied"
            );
        })
        .await;
        crate::routing::root::scope(Arc::from("/billing"), async {
            let target = reflected_target(&path, &empty).expect("a reflected URL");
            assert_eq!(target, format!("/billing{path}"));
            match &target {
                std::borrow::Cow::Owned(target) => {
                    assert_eq!(target.capacity(), target.len(), "one exact buffer");
                }
                std::borrow::Cow::Borrowed(_) => panic!("the root is missing"),
            }
        })
        .await;
        let query = CanonicalValue::Object(BTreeMap::from([(
            "q".to_owned(),
            CanonicalValue::String("red shoes".to_owned()),
        )]));
        for root in ["", "/billing"] {
            crate::routing::root::scope(Arc::from(root), async {
                let target = reflected_target("/docs/a", &query).expect("a reflected URL");
                assert_eq!(target, format!("{root}/docs/a?q=red+shoes"));
            })
            .await;
        }
    }

    #[test]
    fn a_target_is_bounded_by_the_configured_redirect_size() {
        // 2,048 bytes was a fixed cap; 64 KiB is the configured default.
        let target = format!("/reports?{}", "q=x&".repeat(2_500));
        let parsed = navigation_target(&target, 64 * 1024, "rejected").expect("a 10,000-byte URL");
        assert_eq!(parsed.as_str(), target);

        let message = navigation_target(&target, 4_096, "rejected")
            .expect_err("over the configured size")
            .to_string();
        assert!(
            message.contains(&format!(
                "Suprnova Live redirect URL size limit exceeded: measured {} bytes, configured \
                 4096 bytes. Raise LIVE_MAX_REDIRECT_BYTES in the application's .env file",
                target.len()
            )),
            "{message}"
        );
    }
}
