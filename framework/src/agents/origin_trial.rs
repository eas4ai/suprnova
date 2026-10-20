//! The WebMCP Origin Trial token (AGENT-014).
//!
//! A browser runs WebMCP on an origin that presents a valid trial token in
//! an `Origin-Trial` header on its documents. With
//! `WEBMCP_ORIGIN_TRIAL_TOKEN` set, every HTML response carries the token:
//! the server adds it at the one point every response leaves through, so
//! error pages, static `.html` files and plain views carry it as Inertia and
//! Live pages do, whatever middleware the application installs. JSON,
//! redirects, assets and tool answers never do: only a document enables a
//! trial.

use base64::Engine as _;
use hyper::header::{CONTENT_TYPE, HeaderValue};

use crate::error::FrameworkError;

/// The environment variable holding the token.
const TOKEN_VARIABLE: &str = "WEBMCP_ORIGIN_TRIAL_TOKEN";

/// The header a document presents its trial token in.
const ORIGIN_TRIAL: &str = "origin-trial";

/// How the application serves WebMCP: the Origin Trial token its HTML
/// responses carry.
///
/// [`Config::init`](crate::Config::init) reads it from
/// `WEBMCP_ORIGIN_TRIAL_TOKEN` and registers it, so an application needs
/// nothing more than the variable. A test registers one with
/// [`Config::register`](crate::Config::register).
#[derive(Clone, Debug, Default)]
pub struct WebMcpConfig {
    origin_trial_token: Option<HeaderValue>,
}

impl WebMcpConfig {
    /// No token: no response carries `Origin-Trial`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Carry a comma-separated list of trial tokens in every HTML response.
    /// Tokens are trimmed and empty entries are skipped. An empty list is
    /// no token.
    ///
    /// # Errors
    ///
    /// When an entry is not base64, the form browsers issue trial tokens
    /// in, or cannot be a header value. The error names its position, never
    /// its value.
    pub fn with_origin_trial_token(mut self, token: &str) -> Result<Self, FrameworkError> {
        self.origin_trial_token = parse(token)?;
        Ok(self)
    }

    /// The configuration `WEBMCP_ORIGIN_TRIAL_TOKEN` names: no token when it
    /// is unset or empty.
    ///
    /// # Errors
    ///
    /// When the variable holds a value that is not a valid token, naming
    /// the variable, so a mistyped token stops boot instead of silently
    /// disabling the trial.
    pub fn from_env() -> Result<Self, FrameworkError> {
        match crate::config::env::env_optional::<String>(TOKEN_VARIABLE) {
            Some(token) => Self::new().with_origin_trial_token(&token),
            None => Ok(Self::new()),
        }
    }

    /// The tokens joined by `, ` for one header, when any are set.
    pub fn origin_trial_token(&self) -> Option<&str> {
        self.origin_trial_token
            .as_ref()
            .and_then(|value| value.to_str().ok())
    }
}

/// The tokens in one header, or `None` when the list has no tokens.
///
/// The [Origin Trials developer guide] permits comma-separated tokens in
/// one `Origin-Trial` header, so Chrome and Edge tokens can travel together.
///
/// [Origin Trials developer guide]: https://github.com/GoogleChrome/OriginTrials/blob/gh-pages/developer-guide.md#15-can-i-provide-multiple-tokens-on-a-page
fn parse(token: &str) -> Result<Option<HeaderValue>, FrameworkError> {
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let mut tokens = Vec::new();
    for (index, entry) in token.split(',').enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let position = index + 1;
        if engine.decode(entry).is_err() {
            return Err(FrameworkError::internal(format!(
                "{TOKEN_VARIABLE} entry {position} is not an Origin Trial token: a token is \
                 base64, as the browser issues it; copy it again from the trial's registration \
                 page, or leave the variable unset"
            )));
        }
        HeaderValue::from_str(entry).map_err(|_| {
            FrameworkError::internal(format!(
                "{TOKEN_VARIABLE} entry {position} cannot be sent as a header value; copy the \
                 token again, or leave the variable unset"
            ))
        })?;
        tokens.push(entry);
    }
    if tokens.is_empty() {
        return Ok(None);
    }
    HeaderValue::from_str(&tokens.join(", ")).map(Some).map_err(|_| {
        FrameworkError::internal(format!(
            "{TOKEN_VARIABLE} cannot be sent as a header value; copy the tokens again, or leave \
             the variable unset"
        ))
    })
}

/// Add the configured token to `response` when it is an HTML document and
/// does not already carry it.
pub(crate) fn decorate<B>(response: &mut hyper::Response<B>) {
    let html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("text/html")
        });
    if !html {
        return;
    }
    let Some(token) =
        crate::config::Config::get::<WebMcpConfig>().and_then(|config| config.origin_trial_token)
    else {
        return;
    };
    let headers = response.headers_mut();
    if headers
        .get_all(ORIGIN_TRIAL)
        .iter()
        .any(|value| *value == token)
    {
        return;
    }
    headers.append(ORIGIN_TRIAL, token);
}

#[cfg(test)]
mod tests {
    use super::WebMcpConfig;

    #[test]
    fn agent_014_an_empty_token_is_no_token() {
        let config = WebMcpConfig::new()
            .with_origin_trial_token("  ")
            .expect("an empty token is accepted");
        assert_eq!(config.origin_trial_token(), None);
    }

    #[test]
    fn agent_014_a_token_that_is_not_base64_is_refused() {
        let error = WebMcpConfig::new()
            .with_origin_trial_token("not a token!")
            .expect_err("refused");
        assert!(
            error.to_string().contains("WEBMCP_ORIGIN_TRIAL_TOKEN")
                || format!("{error:?}").contains("WEBMCP_ORIGIN_TRIAL_TOKEN"),
            "{error:?}"
        );
    }

    #[test]
    fn agent_014_a_base64_token_is_kept() {
        let config = WebMcpConfig::new()
            .with_origin_trial_token("QUJDRA==")
            .expect("a base64 token");
        assert_eq!(config.origin_trial_token(), Some("QUJDRA=="));
    }
}
