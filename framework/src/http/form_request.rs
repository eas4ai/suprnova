//! FormRequest trait for validated request data
//!
//! Provides Laravel-like FormRequest pattern with automatic body parsing,
//! validation, and authorization.

use super::Request;
use super::body::{parse_form, parse_json};
use super::extract::FromRequest;
use crate::error::{FrameworkError, ValidationErrors};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use validator::Validate;

/// Trait for validated form/JSON request data
///
/// Implement this trait on request structs to enable automatic:
/// - Body parsing (JSON, form-urlencoded or multipart based on Content-Type)
/// - Validation using the `validator` crate
/// - Authorization checks
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::FormRequest;
/// use serde::Deserialize;
/// use validator::Validate;
///
/// #[derive(FormRequest)]  // Auto-derives Deserialize, Validate, and FormRequest impl
/// pub struct CreateUserRequest {
///     #[validate(email)]
///     pub email: String,
///
///     #[validate(length(min = 8))]
///     pub password: String,
/// }
///
/// // In controller:
/// #[handler]
/// pub async fn store(form: CreateUserRequest) -> Response {
///     // `form` is already validated - returns 422 if invalid
///     json_response!({ "email": form.email })
/// }
/// ```
///
/// # Authorization
///
/// Override `authorize()` to add authorization logic:
///
/// ```rust,no_run
/// # use suprnova::{FormRequest, Request};
/// # use serde::Deserialize;
/// # use validator::Validate;
/// # #[derive(Deserialize, Validate)]
/// # struct CreateUserRequest { email: String }
/// impl FormRequest for CreateUserRequest {
///     fn authorize(_req: &Request) -> bool {
///         // Check if user is authenticated
///         true
///     }
/// }
/// ```
#[async_trait]
pub trait FormRequest: Sized + DeserializeOwned + Validate + Send + Sync {
    /// Prepares input before authorization so validation sees normalized text.
    ///
    /// Register deferred changes with `Request::transform_input` to keep the body unread.
    fn prepare_for_validation(_req: &mut Request) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Check if the request is authorized
    ///
    /// Override this method to add authorization logic.
    /// Returns `true` by default (all requests authorized).
    ///
    /// Returning `false` will result in a 403 Forbidden response.
    fn authorize(_req: &Request) -> bool {
        true
    }

    /// Cross-field validation hook, called after the derived rules on real requests.
    /// Its errors are merged with rule errors. Return `Err(ValidationErrors)` to
    /// surface additional errors (e.g. "passwords must match",
    /// "end_date must be after start_date").
    ///
    /// The default implementation returns `Ok(())`.
    ///
    /// This hook runs in both normal and Precognition flows. In
    /// Precognition mode every hook error is retained and surfaces as
    /// `FrameworkError::PrecognitionFailure`.
    /// In the standard flow they surface as `FrameworkError::Validation`
    /// (HTTP 422).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{FormRequest, ValidationErrors};
    /// # use serde::Deserialize;
    /// # use validator::Validate;
    /// # #[derive(Deserialize, Validate)]
    /// # struct UpdatePasswordRequest { new_password: String, confirmation: String }
    /// impl FormRequest for UpdatePasswordRequest {
    ///     fn after_validation(&self) -> Result<(), ValidationErrors> {
    ///         if self.new_password != self.confirmation {
    ///             let mut errs = ValidationErrors::new();
    ///             errs.add("confirmation", "passwords do not match");
    ///             return Err(errs);
    ///         }
    ///         Ok(())
    ///     }
    /// }
    /// ```
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }

    /// Async cross-field validation hook. This is where database-backed
    /// and other `.await`-ing rules - most notably the built-in
    /// [`Unique`] rule - participate in automatic request validation.
    ///
    /// The synchronous [`validate!`] macro cannot weave in `.await`
    /// points, and [`after_validation`] is synchronous, so without this
    /// hook an async rule like `Unique` could only run if every app
    /// hand-wrote the same plumbing in its handler. `extract` calls this
    /// method as the final validation stage, so overriding it is all an
    /// app needs:
    ///
    /// ```rust,no_run
    /// # use suprnova::{FormRequest, ValidationErrors, Unique, AsyncRule};
    /// # use serde::Deserialize;
    /// # use validator::Validate;
    /// # use async_trait::async_trait;
    /// # #[derive(Deserialize, Validate)]
    /// # struct CreateUserRequest { email: String }
    /// #[async_trait]
    /// impl FormRequest for CreateUserRequest {
    ///     async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
    ///         let mut errs = ValidationErrors::new();
    ///         Unique::new("users", "email")
    ///             .check_async(&self.email, &mut errs, "email")
    ///             .await;
    ///         errs.into_result()
    ///     }
    /// }
    /// ```
    ///
    /// # Ordering and bail behavior
    ///
    /// Real requests run derived validation and both hooks in order, merging
    /// their messages. An empty error bag succeeds. Precognition keeps its
    /// stage gates: selected derived errors stop the hooks, and synchronous
    /// hook errors stop this hook.
    ///
    /// The default implementation returns `Ok(())`.
    ///
    /// [`Unique`]: crate::validation::rule::async_rules::Unique
    /// [`validate!`]: crate::validate
    /// [`after_validation`]: Self::after_validation
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }

    /// Maximum request body size (in bytes) accepted by this FormRequest.
    /// It caps a JSON, url-encoded and multipart body alike.
    ///
    /// Defaults to the process-global cap
    /// ([`crate::http::body::global_max_request_body_bytes`]), which is
    /// itself derived from
    /// [`crate::http::body::DEFAULT_MAX_REQUEST_BODY_BYTES`] (8 MiB) unless
    /// the application has called
    /// [`crate::http::body::set_global_max_request_body_bytes`] at boot.
    ///
    /// Override this for endpoints that accept legitimately large JSON
    /// payloads (analytics ingest, bulk import, etc.):
    ///
    /// ```rust,no_run
    /// # use suprnova::FormRequest;
    /// # use serde::Deserialize;
    /// # use validator::Validate;
    /// # #[derive(Deserialize, Validate)]
    /// # struct ImportPayload { rows: Vec<String> }
    /// impl FormRequest for ImportPayload {
    ///     fn max_body_bytes() -> usize { 64 * 1024 * 1024 } // 64 MiB
    /// }
    /// ```
    ///
    /// Lower it for endpoints that should never receive large bodies
    /// (login, search query strings, etc.) to fail fast on abuse.
    fn max_body_bytes() -> usize {
        crate::http::body::global_max_request_body_bytes()
    }

    /// Supply trusted route values before parsing, so path parameters win over body input.
    ///
    /// The `Data` derive implements this for `from_route_param` fields. Keeping
    /// these values here lets every form request use the same parsing,
    /// authorization, validation and Precognition pipeline.
    fn route_inputs(
        _req: &Request,
    ) -> Result<serde_json::Map<String, serde_json::Value>, FrameworkError> {
        Ok(serde_json::Map::new())
    }

    /// Extract and validate data from the request
    ///
    /// This method:
    /// 1. Prepares input and checks authorization before reading the body
    /// 2. Parses the request body (JSON, form or multipart based on
    ///    Content-Type)
    /// 3. Validates the parsed data
    ///
    /// Returns `Err(FrameworkError)` on authorization failure, parse error,
    /// or validation failure.
    async fn extract(mut req: Request) -> Result<Self, FrameworkError> {
        Self::prepare_for_validation(&mut req)?;
        // Authorization still precedes every body read.
        if !Self::authorize(&req) {
            return Err(FrameworkError::Unauthorized);
        }

        // Only the route middleware opts into validation without dispatch.
        let is_precognition = req.is_precognitive();
        let validate_only = is_precognition.then(|| req.validate_only()).flatten();
        let query_input = is_precognition && (req.is_method("GET") || req.is_method("DELETE"));
        // Get the content type before consuming the body and decide how to
        // parse from it. Strip any parameters (`; charset=...`), trim, and
        // lowercase so `Application/JSON; charset=utf-8` classifies the same
        // as `application/json`.
        let media_type = req.content_type().map(|ct| {
            ct.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });

        // Three body shapes are understood: form-urlencoded, multipart (the
        // body the Inertia client sends for a form with a file), and JSON
        // (`application/json` or any `application/*+json` suffix type). Every
        // other content type is rejected with 415. A marked GET or DELETE
        // may omit Content-Type and validate query input. The check
        // runs BEFORE the body is read so an unsupported request never streams.
        let is_form = media_type.as_deref() == Some("application/x-www-form-urlencoded");
        let is_multipart = media_type.as_deref() == Some("multipart/form-data");
        let is_json = media_type
            .as_deref()
            .is_some_and(|mt| mt == "application/json" || mt.ends_with("+json"));
        if !is_form && !is_multipart && !is_json && !(query_input && media_type.is_none()) {
            return Err(FrameworkError::UnsupportedMediaType);
        }
        let route_inputs = Self::route_inputs(&req)?;
        let target = req.validation_redirect_target();
        let transform = req.prepared_input();
        let has_preparation = req.has_prepared_input();
        let mut old_input = serde_json::Value::Null;

        // Collect and parse body. Honor the per-struct cap, a multipart body
        // included; `body_bytes_with_cap` and the multipart parser read
        // `Content-Length` from headers and pre-reject oversized requests
        // with 413 before consuming any body bytes.
        let parsed = if is_precognition && (validate_only.is_some() || query_input) {
            super::input::parse_precognitive::<Self>(
                req,
                Self::max_body_bytes(),
                route_inputs,
                validate_only.as_deref(),
            )
            .await
        } else if is_multipart {
            let payload = super::upload::parse_multipart_streaming_with_limits(
                req,
                super::upload::MultipartLimits {
                    max_body_bytes: Self::max_body_bytes(),
                    max_parts: super::upload::global_max_multipart_parts(),
                    spill_threshold: super::upload::global_upload_spill_threshold(),
                    per_field_max_counts: &[],
                },
                |_, _, _| Ok(()),
            )
            .await?;
            if target.is_some() {
                old_input = Request::multipart_old_input(&payload)?;
            }
            super::input::parse_multipart_with_route_inputs(payload, route_inputs)
                .map_err(|error| error.into_framework_error("Failed to parse multipart body"))
        } else {
            let (_, bytes) = req.body_bytes_with_cap(Self::max_body_bytes()).await?;
            let bytes = if has_preparation {
                prepare_body(bytes, is_form, &transform)?
            } else {
                bytes
            };
            if target.is_some() {
                old_input = if is_form {
                    super::input::parse_form_input(&bytes).map_err(|error| {
                        error.into_framework_error("Failed to retain form input")
                    })?
                } else {
                    serde_json::from_slice(&bytes).unwrap_or_default()
                };
            }
            if route_inputs.is_empty() && is_form {
                parse_form(&bytes)
            } else if is_form {
                super::input::parse_form_with_route_inputs(&bytes, route_inputs)
                    .map_err(|error| error.into_framework_error("Failed to parse form body"))
            } else if !route_inputs.is_empty() {
                parse_json_with_route_inputs(&bytes, route_inputs)
            } else {
                parse_json(&bytes)
            }
        };
        let data: Self = match parsed {
            Ok(data) => data,
            Err(FrameworkError::Validation(errors)) if is_precognition => {
                return Err(FrameworkError::PrecognitionFailure(errors));
            }
            Err(FrameworkError::Validation(errors)) => {
                return Err(Request::validation_failure(errors, target, old_input));
            }
            Err(error) => return Err(error),
        };

        // Run validation. Precognition runs the same validators as a
        // real submission - we just decide what to do with the result.
        // Errors are keyed by the names the input used: validation reports
        // Rust field names, and the derives register what serde renames
        // them to (see `crate::data::input_names`).
        let validation_result = data.validate();

        if is_precognition {
            let mut bag = match validation_result {
                Err(errors) => ValidationErrors::from_validator_keyed(
                    errors,
                    crate::data::input_names::input_key::<Self>,
                ),
                Ok(()) => ValidationErrors::new(),
            };
            if let Some(only) = &validate_only {
                bag = bag.retain_fields(only);
            }
            if !bag.is_empty() {
                return Err(precognition_outcome(bag));
            }
            if let Err(errors) = data.after_validation() {
                let bag = errors.rename_keys(crate::data::input_names::input_key::<Self>);
                if !bag.is_empty() {
                    return Err(precognition_outcome(bag));
                }
            }
            if let Err(errors) = data.after_validation_async().await {
                return Err(precognition_outcome(
                    errors.rename_keys(crate::data::input_names::input_key::<Self>),
                ));
            }
            return Err(FrameworkError::PrecognitionSuccess);
        }

        let mut errors = match validation_result {
            Err(errors) => ValidationErrors::from_validator_keyed(
                errors,
                crate::data::input_names::input_key::<Self>,
            ),
            Ok(()) => ValidationErrors::new(),
        };
        if let Err(hook_errors) = data.after_validation() {
            errors.merge(hook_errors.rename_keys(crate::data::input_names::input_key::<Self>));
        }
        if let Err(hook_errors) = data.after_validation_async().await {
            errors.merge(hook_errors.rename_keys(crate::data::input_names::input_key::<Self>));
        }
        if !errors.is_empty() {
            return Err(Request::validation_failure(errors, target, old_input));
        }

        Ok(data)
    }
}

/// Applies preparation after authorization while retaining each parser's coercion rules.
pub(crate) fn prepare_body(
    bytes: bytes::Bytes,
    is_form: bool,
    transform: &impl Fn(&str, String) -> String,
) -> Result<bytes::Bytes, FrameworkError> {
    if is_form {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for (name, text) in url::form_urlencoded::parse(&bytes) {
            form.append_pair(&name, &transform(&name, text.into_owned()));
        }
        return Ok(bytes::Bytes::from(form.finish()));
    }
    let mut input: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(input) => input,
        Err(_) => return Ok(bytes),
    };
    prepare_json(&mut input, "", transform);
    serde_json::to_vec(&input)
        .map(bytes::Bytes::from)
        .map_err(|error| FrameworkError::internal(format!("Failed to prepare JSON input: {error}")))
}

fn prepare_json(
    input: &mut serde_json::Value,
    path: &str,
    transform: &impl Fn(&str, String) -> String,
) {
    match input {
        serde_json::Value::String(text) => *text = transform(path, std::mem::take(text)),
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                let key = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                prepare_json(value, &key, transform);
            }
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter_mut().enumerate() {
                prepare_json(value, &format!("{path}.{index}"), transform);
            }
        }
        _ => {}
    }
}

fn parse_json_with_route_inputs<T: DeserializeOwned>(
    bytes: &bytes::Bytes,
    route_inputs: serde_json::Map<String, serde_json::Value>,
) -> Result<T, FrameworkError> {
    let mut input = if bytes.is_empty() {
        serde_json::Map::new()
    } else {
        match serde_json::from_slice(bytes) {
            Ok(serde_json::Value::Object(input)) => input,
            Ok(_) => {
                return Err(FrameworkError::domain(
                    "Failed to parse JSON body: the body must be a JSON object",
                    422,
                ));
            }
            Err(error) => {
                return Err(FrameworkError::domain(
                    format!("Failed to parse JSON body: {error}"),
                    422,
                ));
            }
        }
    };
    input.extend(route_inputs);
    let merged = serde_json::to_vec(&input).map_err(|error| {
        FrameworkError::internal(format!("Failed to merge route input: {error}"))
    })?;
    parse_json(&bytes::Bytes::from(merged))
}

/// Answer from the errors of the stage that actually failed.
fn precognition_outcome(bag: ValidationErrors) -> FrameworkError {
    if bag.is_empty() {
        FrameworkError::PrecognitionSuccess
    } else {
        FrameworkError::PrecognitionFailure(bag)
    }
}

impl Request {
    /// Read the selected input names so rules can use the extractor's selection.
    ///
    /// An unmarked request or an absent header returns `None`, so real
    /// requests validate every field. On a marked request, a present empty
    /// header returns an empty list. Names are comma-separated, trimmed,
    /// and empty names are dropped.
    pub fn validate_only(&self) -> Option<Vec<String>> {
        if !self.is_precognitive() {
            return None;
        }
        self.header("Precognition-Validate-Only").map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
    }

    /// Match an input key so your own checks agree with Precognition's rules.
    ///
    /// An unmarked request or an absent selection includes every key, so
    /// real requests keep every check. Each `*` matches one non-empty dotted
    /// segment. `tags.*` includes `tags.3`; `tags` includes only `tags`.
    pub fn should_validate(&self, field: &str) -> bool {
        self.validate_only().is_none_or(|only| {
            only.iter()
                .any(|name| crate::error::field_covers(name, field))
        })
    }

    /// Extract validated input inline so raw-request handlers share form validation.
    ///
    /// Ordinary requests return the typed value. A marked request returns
    /// `PrecognitionSuccess` or `PrecognitionFailure` for the response layer.
    ///
    /// ```rust
    /// use suprnova::{Request, Response, request};
    ///
    /// #[request]
    /// struct Signup {
    ///     #[validate(email)]
    ///     email: String,
    /// }
    ///
    /// async fn signup(request: Request) -> Response {
    ///     let input = request.validate::<Signup>().await?;
    ///     suprnova::text(input.email)
    /// }
    /// ```
    pub async fn validate<T: FormRequest>(self) -> Result<T, FrameworkError> {
        T::extract(self).await
    }
}

/// Blanket implementation of FromRequest for all FormRequest types
#[async_trait]
impl<T: FormRequest> FromRequest for T {
    async fn from_request(req: Request) -> Result<Self, FrameworkError> {
        T::extract(req).await
    }
}
