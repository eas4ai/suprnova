//! Lossless projection of engine-owned Live HTTP intent into Suprnova responses.

use std::error::Error;
use std::fmt;

use suprnova_live::endpoint::LiveEndpointResponse;

use crate::HttpResponse;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LiveResponseProjectionErrorKind {
    MissingHeaderName,
    NonTextHeaderValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveResponseProjectionError {
    kind: LiveResponseProjectionErrorKind,
}

impl fmt::Display for LiveResponseProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            LiveResponseProjectionErrorKind::MissingHeaderName => {
                "live_response_header_name_is_missing"
            }
            LiveResponseProjectionErrorKind::NonTextHeaderValue => {
                "live_response_header_value_is_not_text"
            }
        })
    }
}

impl Error for LiveResponseProjectionError {}

pub(crate) fn project(
    response: LiveEndpointResponse,
) -> Result<HttpResponse, LiveResponseProjectionError> {
    let mut projected = HttpResponse::bytes_body(response.body, "application/octet-stream")
        .without_header("Content-Type")
        .status(response.status.as_u16());
    let mut current_name = None;
    for (name, value) in response.headers {
        if let Some(name) = name {
            current_name = Some(name);
        }
        let name = current_name.as_ref().ok_or(LiveResponseProjectionError {
            kind: LiveResponseProjectionErrorKind::MissingHeaderName,
        })?;
        let value = value.to_str().map_err(|_| LiveResponseProjectionError {
            kind: LiveResponseProjectionErrorKind::NonTextHeaderValue,
        })?;
        projected = projected.header(name.as_str(), value);
    }
    Ok(projected)
}
