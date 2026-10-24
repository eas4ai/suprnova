//! Welcome controller - Phase 5B Task 20 dogfood.
//!
//! `POST /api/welcome?email=...&name=...` queues a `WelcomeEmail` mailable
//! through the bound transport via `Mail::queue`. Errors propagate as a
//! framework error -> 500; the integration test asserts on the queue
//! envelope instead of round-tripping the HTTP boundary.

use std::collections::HashMap;
use suprnova::config::Environment;
use suprnova::{Config, HttpResponse, Request, Response};

/// An unauthenticated endpoint that mails any address is a spam relay when
/// the application is deployed with a delivering transport. The dogfood demo
/// exists for local work and tests; in every other environment, a custom
/// label included, the route does not exist.
pub fn welcome_route_available(environment: &Environment) -> bool {
    matches!(
        environment,
        Environment::Local | Environment::Development | Environment::Testing
    )
}

pub async fn queue(req: Request) -> Response {
    if !welcome_route_available(&Config::environment()) {
        return Ok(HttpResponse::text("not found").status(404));
    }
    // `Request::query` returns the raw query string (e.g. `email=a&name=b`).
    // Decode it the same way the framework decodes resource fieldsets and
    // include sets - `url::form_urlencoded::parse` URL-decodes pairs and
    // tolerates malformed UTF-8 by substituting U+FFFD.
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> = url::form_urlencoded::parse(raw.as_bytes())
        .into_owned()
        .collect();
    let email = params
        .get("email")
        .map(String::as_str)
        .unwrap_or("guest@example.org");
    let name = params.get("name").map(String::as_str).unwrap_or("Guest");

    crate::mail::welcome::queue_welcome(email, name).await?;
    Ok(HttpResponse::text("queued"))
}
