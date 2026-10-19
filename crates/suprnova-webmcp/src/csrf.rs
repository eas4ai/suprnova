//! The header that carries the page's CSRF token on a server tool call.
//!
//! The framework's `CsrfMiddleware` accepts the token in `X-CSRF-TOKEN`,
//! as the root template's `csrf-token` meta tag holds it, or in
//! `X-XSRF-TOKEN`, as the `XSRF-TOKEN` cookie holds it. The stream client in
//! the kits reads the two in the same order (`frontend/stream/core.ts`).

use percent_encoding::percent_decode_str;

/// The cookie `CsrfMiddleware` writes the token to.
const XSRF_COOKIE: &str = "XSRF-TOKEN";

/// The header name and value for a page whose `csrf-token` meta tag holds
/// `meta` and whose `document.cookie` is `cookie`: the meta tag's token,
/// else the `XSRF-TOKEN` cookie's, decoded. `None` when the page has
/// neither, and the server then answers 419.
pub(crate) fn header(meta: Option<&str>, cookie: &str) -> Option<(&'static str, String)> {
    if let Some(token) = meta.filter(|token| !token.is_empty()) {
        return Some(("X-CSRF-TOKEN", token.to_string()));
    }
    cookie.split(';').find_map(|entry| {
        let value = entry.trim().strip_prefix(XSRF_COOKIE)?.strip_prefix('=')?;
        // A value that is not percent-encoded UTF-8 is not a token the
        // middleware wrote, so it is not sent.
        let token = percent_decode_str(value).decode_utf8().ok()?;
        Some(("X-XSRF-TOKEN", token.into_owned()))
    })
}
