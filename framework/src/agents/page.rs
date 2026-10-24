//! What the framework's default agent pages share: the approval and
//! connected-agents pages of the OAuth server and the claim page of agent
//! auth (AGENT-017).
//!
//! Every page requires the signed-in user, refuses to be framed, is never
//! cached and sends no referrer. Markup comes only from `&'static str`;
//! every value is escaped on its way in (see [`Html`]), so one escaper
//! serves every page.

use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request};

/// Send a guest to `sign_in_url`, to come back to the page `request`
/// asked for: its path and query, relative to the application's public
/// root, in the query parameter `return_to_parameter`.
pub(crate) fn sign_in_redirect(
    sign_in_url: &str,
    return_to_parameter: &str,
    request: &Request,
) -> HttpResponse {
    let return_to = match request.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", request.path()),
        _ => request.path().to_owned(),
    };
    let separator = if sign_in_url.contains('?') { '&' } else { '?' };
    let pair = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(return_to_parameter, &return_to)
        .finish();
    guarded(HttpResponse::new().status(302))
        .header("Location", format!("{sign_in_url}{separator}{pair}"))
}

/// A short page that says one thing.
pub(crate) fn notice(status: u16, title: &'static str, message: &'static str) -> HttpResponse {
    let mut html = Html::default();
    html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
        .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>")
        .raw(title)
        .raw("</title>\n</head>\n<body>\n<main>\n<h1>")
        .raw(title)
        .raw("</h1>\n<p>")
        .raw(message)
        .raw("</p>\n</main>\n</body>\n</html>\n");
    guarded(HttpResponse::html(html.into_string()).status(status))
}

/// The headers every answer of these pages carries: never framed, so no
/// page can overlay a button and trick the user into pressing it; never
/// cached; and no referrer, since the address names a pending request.
pub(crate) fn guarded(response: HttpResponse) -> HttpResponse {
    response
        .header("Cache-Control", "no-store")
        .header("X-Frame-Options", "DENY")
        .header("Content-Security-Policy", "frame-ancestors 'none'")
        .header("Referrer-Policy", "no-referrer")
}

/// `response` with an error answer guarded too: a page's failures, such as
/// a store that cannot answer, carry the same headers as its pages.
pub(crate) fn guard_errors(response: crate::http::Response) -> crate::http::Response {
    response.map_err(guarded)
}

/// The session's CSRF token for a page's forms.
pub(crate) fn csrf_token() -> Result<String, FrameworkError> {
    crate::session::get_csrf_token().ok_or_else(|| {
        FrameworkError::internal(
            "the framework's agent pages post their forms with the session's CSRF token; \
             install the session middleware",
        )
    })
}

/// An HTML document under construction. Markup only comes in as
/// `&'static str`, through [`Self::raw`]; every value the page shows goes
/// through [`Self::text`], which escapes it for both text and quoted
/// attributes. So no client-supplied name, host or address can become
/// markup. A value a party other than the application chose goes through
/// [`Self::client`], which also isolates its direction, so whatever
/// reordering it carries stays inside it.
#[derive(Default)]
pub(crate) struct Html(String);

impl Html {
    /// Append fixed markup.
    pub(crate) fn raw(&mut self, markup: &'static str) -> &mut Self {
        self.0.push_str(markup);
        self
    }

    /// Append `value`, a text a client, an agent or a provider chose,
    /// escaped and inside `<bdi>`. Magnetar already removes the control
    /// and format characters from a client's text; the isolation holds for
    /// any it still carries, such as right-to-left letters, so they cannot
    /// reorder the page around them.
    pub(crate) fn client(&mut self, value: &str) -> &mut Self {
        self.raw("<bdi>").text(value).raw("</bdi>")
    }

    /// Append `value` as text, escaped.
    pub(crate) fn text(&mut self, value: &str) -> &mut Self {
        for ch in value.chars() {
            match ch {
                '&' => self.0.push_str("&amp;"),
                '<' => self.0.push_str("&lt;"),
                '>' => self.0.push_str("&gt;"),
                '"' => self.0.push_str("&quot;"),
                '\'' => self.0.push_str("&#39;"),
                _ => self.0.push(ch),
            }
        }
        self
    }

    /// The document.
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_017_html_escapes_every_value() {
        let mut html = Html::default();
        html.raw("<p title=\"").text("\"><script>'&").raw("\">");
        assert_eq!(
            html.into_string(),
            "<p title=\"&quot;&gt;&lt;script&gt;&#39;&amp;\">"
        );
    }

    #[test]
    fn agent_017_client_text_is_isolated_and_escaped() {
        let mut html = Html::default();
        html.client("<b>\u{202e}evil");
        assert_eq!(html.into_string(), "<bdi>&lt;b&gt;\u{202e}evil</bdi>");
    }
}
