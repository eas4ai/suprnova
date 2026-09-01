//! Lossless projection of engine-owned endpoint intent into Suprnova HTTP.

use suprnova_live::endpoint::LiveEndpointResponse;

use crate::{FrameworkError, HttpResponse};

pub(crate) struct SuprnovaResponseIntentPort;

impl SuprnovaResponseIntentPort {
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
