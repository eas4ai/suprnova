//! A helper that reads the session, called from the view.

/// Reads the session's CSRF token.
pub fn token() -> Option<String> {
    suprnova::csrf_token()
}
