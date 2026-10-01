//! Localization dogfood - Task 11.
//!
//! `GET /lang-demo` renders a translated greeting via the `__!` macro;
//! `POST /lang-demo` demonstrates a translated validation failure
//! (`validation-required`) when the required `name` field is missing.
//!
//! A new route rather than retrofitting an existing one: `/api/ping` is
//! the app's other minimal GET, but it sits behind `RateLimitMiddleware`
//! (5 requests/60s) - a locale-detection suite firing several requests
//! per run would trip it. Every other simple controller already carries
//! its own dogfood purpose (auth, mail, SSE, pagination, ...); folding
//! localization into one of those would muddy what each one proves.
//! `/lang-demo` keeps this dogfood isolated and repeatable, and is
//! excepted from CSRF in `bootstrap::register_http_stack` for the same
//! reason `/api/ping` and `/api/welcome` are: no session, no cookie,
//! nothing ambient for a cross-site POST to abuse.

use suprnova::{HttpResponse, Request, Response, handler, json_response, request};

/// `GET /lang-demo` - plain-text greeting translated per the request's
/// detected locale.
pub async fn show(_req: Request) -> Response {
    Ok(HttpResponse::text(
        suprnova::__!("welcome", app: "Suprnova"),
    ))
}

/// Body for `POST /lang-demo`. `name` is `Option<String>` (rather than
/// a plain `String`) so it is the `validator` crate's `required` rule -
/// not JSON deserialization - that fails when the field is absent; that
/// is what exercises the translated `validation-required` message
/// (serde deserializes a missing `Option<T>` field to `None` rather
/// than erroring, so the request still reaches the validator).
#[request]
pub struct LangDemoForm {
    #[validate(required)]
    pub name: Option<String>,
}

/// `POST /lang-demo` - echoes `name` back once validated. A missing
/// `name` short-circuits inside the `#[handler]`-generated extraction
/// with a 422 whose `validation-required` message renders translated
/// per the request's detected locale.
#[handler]
pub async fn submit(form: LangDemoForm) -> Response {
    json_response!({ "name": form.name })
}
