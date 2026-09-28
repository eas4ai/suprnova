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

pub(crate) struct SuprnovaResponseIntentPort;

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

impl SuprnovaResponseIntentPort {
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
                endpoint = endpoint.with_redirect(
                    EndpointNavigationTarget::parse(&target)
                        .map_err(|_| FrameworkError::internal("Live route target was rejected"))?,
                );
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
                    let target = reflected_url(path, intent.query())?;
                    endpoint = endpoint.with_reflected_url(
                        EndpointNavigationTarget::parse(&target).map_err(|_| {
                            FrameworkError::internal("Live reflected target was rejected")
                        })?,
                    );
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

fn reflected_url(path: &str, query: &CanonicalValue) -> Result<String, FrameworkError> {
    let CanonicalValue::Object(query) = query else {
        return Err(FrameworkError::internal("Live URL query was rejected"));
    };
    if query.is_empty() {
        return Ok(path.to_owned());
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
    Ok(format!("{path}?{}", serializer.finish()))
}
