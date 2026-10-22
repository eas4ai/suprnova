//! A method that reaches the session through another method, called from the view.

/// Holds nothing.
pub struct Panel;

impl Panel {
    /// Reads the session's CSRF token.
    fn token(&self) -> Option<String> {
        suprnova::csrf_token()
    }

    /// Says whether a token exists, through `token`.
    pub fn signed_in(&self) -> bool {
        self.token().is_some()
    }
}
