//! Opt-in validation requests and pre-checks that can answer before dispatch.

use std::future::Future;

use crate::{FrameworkError, HttpResponse, Middleware, Next, Request, Response, async_trait};

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
}
