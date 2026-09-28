//! Bearer-token middleware backed by Magnetar sessions.

use async_trait::async_trait;

use crate::Request;
use crate::auth::request_state::set_bearer_user_id;
use crate::http::Response;
use crate::middleware::{Middleware, Next};

/// Authenticate `Authorization: Bearer` credentials through Magnetar.
pub struct BearerTokenMiddleware;

#[async_trait]
impl Middleware for BearerTokenMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(auth_header) = request.header("Authorization")
            && let Some(token) = strip_bearer_scheme(auth_header)
        {
            let token = token.trim();
            if !token.is_empty()
                && let Some(engine) = super::optional_factor_engine()
                && let Ok(Some(user_id)) = engine.bearer_user_id(token).await
            {
                set_bearer_user_id(&user_id);
            }
        }
        next(request).await
    }
}

fn strip_bearer_scheme(header: &str) -> Option<&str> {
    const SCHEME: &str = "Bearer";
    if header.len() <= SCHEME.len() {
        return None;
    }
    let (scheme, rest) = header.split_at(SCHEME.len());
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    if !matches!(rest.as_bytes().first(), Some(b' ' | b'\t')) {
        return None;
    }
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::strip_bearer_scheme;

    #[test]
    fn accepts_case_insensitive_bearer_with_whitespace() {
        assert_eq!(strip_bearer_scheme("Bearer abc"), Some(" abc"));
        assert_eq!(strip_bearer_scheme("bearer\tabc"), Some("\tabc"));
        assert_eq!(strip_bearer_scheme("BeArEr   abc"), Some("   abc"));
    }

    #[test]
    fn rejects_wrong_or_unseparated_schemes() {
        assert_eq!(strip_bearer_scheme("Basic abc"), None);
        assert_eq!(strip_bearer_scheme("Bearertoken"), None);
        assert_eq!(strip_bearer_scheme("Bearer"), None);
        assert_eq!(strip_bearer_scheme(""), None);
    }
}
