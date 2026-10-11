//! Opt-in validation requests and pre-checks that can answer before dispatch.

use std::future::Future;

use crate::{
    FrameworkError, HttpResponse, Middleware, Next, Request, Response, ValidationErrors,
    async_trait,
};

tokio::task_local! {
    /// The fields a precognitive request lists in
    /// `Precognition-Validate-Only`, installed while a form's
    /// after-validation hooks run. Request-scoped, so a concurrent request
    /// never sees another request's selection.
    static HOOK_SELECTION: Option<HookSelection>;
}

/// A request's field selection as the hooks see it.
#[derive(Clone)]
struct HookSelection {
    /// The input names the request listed.
    only: Vec<String>,
    /// Maps a key the hook writes in Rust field names to the input name
    /// the request listed, the mapping the hook's errors go through.
    input_key: fn(&str) -> String,
}

impl HookSelection {
    fn covers(&self, field: &str) -> bool {
        let input = (self.input_key)(field);
        self.only.iter().any(|wanted| {
            crate::error::field_covers(wanted, field) || crate::error::field_covers(wanted, &input)
        })
    }
}

fn hook_selection(only: Option<&[String]>, input_key: fn(&str) -> String) -> Option<HookSelection> {
    only.map(|only| HookSelection {
        only: only.to_vec(),
        input_key,
    })
}

/// Run a synchronous after-validation hook with the request's field
/// selection installed, so [`Precognition::should_validate`] and the
/// database rules answer it. `only` is the request's
/// `Precognition-Validate-Only` list, `None` to select every field;
/// `input_key` maps a Rust field key to its input name.
///
/// Called by `FormRequest::extract` and the code
/// `#[derive(MultipartRequest)]` generates; applications do not call it.
#[doc(hidden)]
pub fn with_hook_selection<R>(
    only: Option<&[String]>,
    input_key: fn(&str) -> String,
    hook: impl FnOnce() -> R,
) -> R {
    HOOK_SELECTION.sync_scope(hook_selection(only, input_key), hook)
}

/// The asynchronous form of [`with_hook_selection`], for
/// `after_validation_async`.
#[doc(hidden)]
pub async fn with_hook_selection_async<F: Future>(
    only: Option<&[String]>,
    input_key: fn(&str) -> String,
    hook: F,
) -> F::Output {
    HOOK_SELECTION
        .scope(hook_selection(only, input_key), hook)
        .await
}

/// Enable Precognition on a route or group so validation cannot run its handler body.
///
/// Add this before middleware whose failures a validation client needs to read.
/// Every response varies on `Precognition`, including ordinary submissions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Precognitive;

#[async_trait]
impl Middleware for Precognitive {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        if request.is_attempting_precognition() {
            request.set_precognitive();
        }
        let precognitive = request.is_precognitive();
        next(request)
            .await
            .map(|response| decorate(response, precognitive))
            .map_err(|response| decorate(response, precognitive))
    }
}

fn decorate(mut response: HttpResponse, precognitive: bool) -> HttpResponse {
    let vary = response
        .header_values("Vary")
        .collect::<Vec<_>>()
        .join(", ");
    if !vary
        .split(',')
        .any(|value| value.trim().eq_ignore_ascii_case("Precognition"))
    {
        let vary = if vary.is_empty() {
            "Precognition".to_string()
        } else {
            format!("{vary}, Precognition")
        };
        response = response.replace_header("Vary", vary);
    }
    if precognitive {
        response = response.replace_header("Precognition", "true");
    }
    response
}

/// A pre-check's early response, with an optional answer for validation clients.
///
/// Return this from a [`Precognition::precognitive`] closure to stop its work.
pub struct Bail {
    default: Box<Response>,
    precognition: Option<Box<Response>>,
}

impl Bail {
    /// Use the same early response for validation clients and ordinary requests.
    pub fn with(default: Response) -> Self {
        Self {
            default: Box::new(default),
            precognition: None,
        }
    }

    /// Give a validation client a different early answer from a real submission.
    pub fn precognition(default: Response, precognition: Response) -> Self {
        Self {
            default: Box::new(default),
            precognition: Some(Box::new(precognition)),
        }
    }

    fn response(self, precognitive: bool) -> Response {
        if precognitive {
            *self.precognition.unwrap_or(self.default)
        } else {
            *self.default
        }
    }
}

/// Run middleware pre-checks with the same early-exit semantics as Precognition dispatch.
pub struct Precognition;

impl Precognition {
    /// Run a pre-check, choose its bail response, or finish validation with `204`.
    ///
    /// A marked request returns `204` after a successful closure. An ordinary
    /// request receives the closure's value. A bail uses its Precognition answer
    /// only when the request is marked, otherwise its default answer.
    ///
    /// Use this in middleware after [`Precognitive`]. Dispatch skips the body of
    /// a marked handler, so a pre-check that must run belongs in middleware.
    /// On an ordinary route the raw header is only an attempt; you can inspect
    /// [`Request::is_attempting_precognition`] without enabling the protocol.
    ///
    /// ```rust
    /// use suprnova::{Bail, HttpResponse, Precognition, Request, Response};
    ///
    /// async fn check(request: &Request) -> Response {
    ///     Precognition::precognitive(request, || async {
    ///         if request.header("Authorization").is_none() {
    ///             return Err(Bail::precognition(
    ///                 Err(HttpResponse::text("Sign in").status(401)),
    ///                 Err(HttpResponse::text("Cannot validate").status(403)),
    ///             ));
    ///         }
    ///         Ok(suprnova::text("Continue"))
    ///     }).await
    /// }
    /// # suprnova::tokio::runtime::Runtime::new().expect("runtime").block_on(async {
    /// # let request = Request::for_test("POST", "/form");
    /// # let Err(response) = check(&request).await else { panic!("missing authorization must bail") };
    /// # assert_eq!(response.status_code(), 401);
    /// # });
    /// ```
    pub async fn precognitive<T, F, Fut>(request: &Request, check: F) -> Response
    where
        T: Into<Response>,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Bail>>,
    {
        match check().await {
            Err(bail) => bail.response(request.is_precognitive()),
            Ok(_) if request.is_precognitive() => Err(FrameworkError::PrecognitionSuccess.into()),
            Ok(value) => value.into(),
        }
    }

    /// Turn an error bag you built yourself into the Precognition answer,
    /// as Laravel's `Precognition::afterValidationHook` does for any
    /// validator.
    ///
    /// Use it where validation does not run through a form request: in
    /// middleware, or in a handler that checks input by hand. On a
    /// precognitive request an empty bag answers `204` with
    /// `Precognition-Success: true` and a non-empty one `422` with the
    /// bag. On another request an empty bag returns `Ok(())`, so the
    /// request goes on, and a non-empty one is the ordinary validation
    /// error: a redirect back with the errors for an HTML form, `422`
    /// otherwise.
    ///
    /// The bag is used as given. To leave out the fields a precognitive
    /// request did not list, check them only when
    /// [`Request::should_validate`] answers `true`.
    ///
    /// ```rust
    /// use suprnova::{Middleware, Next, Precognition, Request, Response, ValidationErrors, async_trait};
    ///
    /// struct RequireName;
    ///
    /// #[async_trait]
    /// impl Middleware for RequireName {
    ///     async fn handle(&self, request: Request, next: Next) -> Response {
    ///         let mut errors = ValidationErrors::new();
    ///         if request.should_validate("name") && request.header("X-Name").is_none() {
    ///             errors.add("name", "The name field is required.");
    ///         }
    ///         Precognition::after_validation(&request, errors)?;
    ///         next(request).await
    ///     }
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::PrecognitionSuccess`] for an empty bag on
    /// a precognitive request, [`FrameworkError::PrecognitionFailure`] for
    /// a non-empty one, and the ordinary validation error for a non-empty
    /// bag on another request.
    pub fn after_validation(
        request: &Request,
        errors: ValidationErrors,
    ) -> Result<(), FrameworkError> {
        match (request.is_precognitive(), errors.is_empty()) {
            (true, true) => Err(FrameworkError::PrecognitionSuccess),
            (true, false) => Err(FrameworkError::PrecognitionFailure(errors)),
            (false, true) => Ok(()),
            (false, false) => Err(Request::validation_failure(
                errors,
                request.validation_redirect_target(),
                serde_json::Value::Null,
            )),
        }
    }

    /// Whether the field is one the current precognitive request asked to
    /// validate.
    ///
    /// Inside the after-validation hooks of a form request, a data object
    /// or a multipart form, while a precognitive request lists fields in
    /// `Precognition-Validate-Only`, this answers whether `field` is
    /// listed, by the match [`Request::should_validate`] applies: each `*`
    /// matches one non-empty dotted segment, so `tags.*` selects `tags.3`
    /// and `tags` selects only `tags`. A field is matched both as written
    /// and under the input name it is renamed to. Everywhere else it is
    /// `true`, so ordinary requests check every field.
    ///
    /// The built-in database rules ([`AsyncRule::check_async`],
    /// [`Exists::check_value`] and [`Exists::check_each`]) ask it before
    /// they query, so a hook does not query or fail for a field the
    /// request did not ask about, as Laravel removes the rules of
    /// unlisted fields before validating. Ask it yourself before other
    /// work a hook does per field. It narrows the checks a hook runs,
    /// never the errors a hook returns.
    ///
    /// [`AsyncRule::check_async`]: crate::AsyncRule::check_async
    /// [`Exists::check_value`]: crate::Exists::check_value
    /// [`Exists::check_each`]: crate::Exists::check_each
    pub fn should_validate(field: &str) -> bool {
        HOOK_SELECTION
            .try_with(|selection| {
                selection
                    .as_ref()
                    .is_none_or(|selection| selection.covers(field))
            })
            .unwrap_or(true)
    }
}
