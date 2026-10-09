//! Merge validation sources without handing placeholders to application handlers.

use std::borrow::Cow;

use serde::de::DeserializeOwned;

use super::nested::{Nested, Node};
use super::{Collector, form, record_missing_fields, struct_field_names};
use crate::{FrameworkError, Request};

/// Keep the ordinary input readers and upload limits in selected validation.
pub(crate) async fn parse_precognitive<T: DeserializeOwned>(
    request: Request,
    max_body_bytes: usize,
    route_inputs: serde_json::Map<String, serde_json::Value>,
    only: Option<&[String]>,
) -> Result<T, FrameworkError> {
    let fields = struct_field_names::<T>();
    let query = if request.is_method("GET") || request.is_method("DELETE") {
        request.query().unwrap_or("").to_owned()
    } else {
        String::new()
    };
    let mut input = Nested::from_urlencoded(query.as_bytes(), fields);
    for node in input.names.values_mut() {
        node.decode_query_objects();
    }
    let body = read_body(request, max_body_bytes, fields).await?;
    input.names.extend(body.names);
    input.untracked = body.untracked.or(input.untracked);
    for (name, value) in route_inputs {
        input.names.insert(Cow::Owned(name), Node::Json(value));
    }
    fill_unlisted_missing::<T>(&mut input, fields, only);
    form::read_nested_selected(input, fields, only)
        .map_err(|error| error.into_framework_error("Failed to parse validation input"))
}

/// Read each body shape under the same limits used by ordinary form extraction.
async fn read_body(
    request: Request,
    max_body_bytes: usize,
    fields: Option<&'static [&'static str]>,
) -> Result<Nested<'static>, FrameworkError> {
    let content_type = request.content_type().unwrap_or("").to_owned();
    if super::super::body::is_multipart_form_data(&content_type) {
        let payload = crate::http::upload::parse_multipart_streaming_with_limits(
            request,
            crate::http::upload::MultipartLimits {
                max_body_bytes,
                max_parts: crate::http::upload::global_max_multipart_parts(),
                spill_threshold: crate::http::upload::global_upload_spill_threshold(),
                per_field_max_counts: &[],
            },
            |_, _, _| Ok(()),
        )
        .await?;
        Ok(Nested::from_multipart(payload, fields))
    } else {
        let (_, bytes) = request.body_bytes_with_cap(max_body_bytes).await?;
        if super::super::body::is_form_urlencoded(&content_type) {
            // The body outlives this read through owned decoded pairs.
            let mut body = Nested::new(fields);
            for (name, value) in url::form_urlencoded::parse(&bytes) {
                body.insert(
                    Cow::Owned(name.into_owned()),
                    Node::Text(Cow::Owned(value.into_owned())),
                );
            }
            Ok(body)
        } else {
            let mut body = Nested::new(fields);
            if !bytes.is_empty() {
                let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
                    FrameworkError::domain(format!("Failed to parse JSON body: {error}"), 422)
                })?;
                let serde_json::Value::Object(values) = value else {
                    return Err(FrameworkError::domain(
                        "The validation body must be a JSON object",
                        422,
                    ));
                };
                for (name, value) in values {
                    body.names.insert(Cow::Owned(name), Node::Json(value));
                }
            }
            Ok(body)
        }
    }
}

/// Fill only missing required fields so aliases and defaults retain their meaning.
fn fill_unlisted_missing<T: DeserializeOwned>(
    input: &mut Nested<'_>,
    fields: Option<&'static [&'static str]>,
    only: Option<&[String]>,
) {
    if let (Some(only), Some(fields)) = (only, fields) {
        // Ask serde which required names are absent, so aliases and defaults
        // retain their ordinary meaning. Only absent unselected fields get null.
        let missing = Collector::default();
        record_missing_fields::<T>(&missing, fields, input.present(fields));
        for name in missing.into_errors().errors.into_keys() {
            if !only
                .iter()
                .any(|wanted| crate::error::field_covers(wanted, &name))
            {
                input
                    .names
                    .insert(Cow::Owned(name), Node::Text(Cow::Borrowed("")));
            }
        }
    }
}
