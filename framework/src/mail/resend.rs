//! Resend HTTP transport. POSTs JSON to <https://api.resend.com/emails> with
//! `Authorization: Bearer <api-key>`.

use crate::error::FrameworkError;
use crate::mail::http_provider::{err, read_error_body, shared_client};
use crate::mail::transport::{MailTransport, OutgoingMessage};
use crate::mail::wire;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::BTreeMap;

const DEFAULT_ENDPOINT: &str = "https://api.resend.com/emails";

/// Resend HTTP transport. Authenticates via a bearer API key and POSTs
/// JSON to the Resend `/emails` endpoint.
pub struct ResendMailTransport {
    api_key: String,
    endpoint: String,
}

impl ResendMailTransport {
    /// Build a transport pointing at Resend's production endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: DEFAULT_ENDPOINT.into(),
        }
    }

    /// Build a transport pointing at a custom endpoint (test/staging
    /// or regional mirror). The endpoint is normalized to include the
    /// trailing `/emails` path component.
    pub fn with_endpoint(api_key: impl Into<String>, endpoint: impl AsRef<str>) -> Self {
        // Trim trailing slash first so `https://x.example/emails/` is detected
        // as already-terminated and we don't double-append.
        let e = endpoint.as_ref().trim_end_matches('/');
        // `ends_with` (not `contains`) - a base URL like `/emails-archive/api`
        // only *contains* the substring but is not the Resend endpoint, so we
        // must still append.
        let url = if e.ends_with("/emails") {
            e.to_string()
        } else {
            format!("{e}/emails")
        };
        Self {
            api_key: api_key.into(),
            endpoint: url,
        }
    }
}

#[derive(Serialize)]
struct RsBody<'a> {
    from: String,
    to: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reply_to: Vec<String>,
    subject: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<RsAttachment<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<RsTag<'a>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct RsAttachment<'a> {
    filename: &'a str,
    content: String, // base64
    content_type: &'a str,
}

/// Resend requires both `name` and `value` on every tag and rejects the
/// whole email over a tag that lacks either.
#[derive(Serialize)]
struct RsTag<'a> {
    name: String,
    value: &'a str,
}

/// Resend tag rule: a tag name and value hold only ASCII letters, digits,
/// `_` and `-`, at most 256 characters, and Resend rejects the whole email
/// over one that does not.
fn resend_tag_valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Map the plain-string `tags` onto Resend's `{name, value}` pairs the way
/// SES maps them: a bare tag becomes `{name: "tag_<i>", value: tag}`, whose
/// name is valid by construction. A tag Resend cannot carry is refused here,
/// so the caller gets the error instead of a queued mail Resend rejects on
/// every retry.
fn resend_tags(msg: &OutgoingMessage) -> Result<Vec<RsTag<'_>>, FrameworkError> {
    msg.tags
        .iter()
        .enumerate()
        .map(|(i, tag)| {
            if resend_tag_valid(tag) {
                Ok(RsTag {
                    name: format!("tag_{i}"),
                    value: tag,
                })
            } else {
                Err(FrameworkError::internal(format!(
                    "Resend: tag {tag:?} cannot be sent: a Resend tag value holds only \
                     [A-Za-z0-9_-], at most 256 characters"
                )))
            }
        })
        .collect()
}

#[async_trait]
impl MailTransport for ResendMailTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        use base64::Engine;
        wire::check_message("Resend", msg)?;
        let attachments: Vec<RsAttachment> = msg
            .attachments
            .iter()
            .map(|a| RsAttachment {
                filename: &a.filename,
                content: base64::engine::general_purpose::STANDARD.encode(&a.content),
                content_type: &a.content_type,
            })
            .collect();

        // Resend tags are a list of `{name, value}` objects; the Suprnova
        // model carries plain strings, mapped by `resend_tags`. Metadata
        // maps to provider headers (Resend has no first-class metadata
        // field - `headers` is the standard pass-through). Caller-set
        // custom headers union over metadata.
        let tags = resend_tags(msg)?;
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        for (k, v) in &msg.metadata {
            // The metadata key becomes part of a header name, so it is held
            // to the header-name rule. Caller headers were checked above.
            let name = format!("X-Metadata-{k}");
            wire::check_header("Resend", &name, v)?;
            headers.insert(name, v.clone());
        }
        for (k, v) in &msg.headers {
            headers.insert(k.clone(), v.clone());
        }
        if let Some(p) = msg.priority {
            headers.insert("X-Priority".into(), p.to_string());
        }

        let body = RsBody {
            from: wire::mailbox_text("Resend", &msg.from)?,
            to: wire::mailbox_texts("Resend", &msg.to)?,
            cc: wire::mailbox_texts("Resend", &msg.cc)?,
            bcc: wire::mailbox_texts("Resend", &msg.bcc)?,
            reply_to: wire::mailbox_texts("Resend", &msg.reply_to)?,
            subject: &msg.subject,
            html: msg.html.as_deref(),
            text: msg.text.as_deref(),
            attachments,
            tags,
            headers,
        };

        let resp = shared_client()
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| FrameworkError::internal(format!("Resend transport: {e}")))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = read_error_body(resp).await;
            return Err(err("Resend", status, body));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "resend"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_uses_default_endpoint() {
        let t = ResendMailTransport::new("k");
        assert_eq!(t.endpoint, DEFAULT_ENDPOINT);
    }

    #[test]
    fn with_endpoint_appends_emails_path_when_missing() {
        let t = ResendMailTransport::with_endpoint("k", "https://proxy.example.com");
        assert_eq!(t.endpoint, "https://proxy.example.com/emails");
    }

    #[test]
    fn with_endpoint_preserves_terminal_emails_path() {
        let t = ResendMailTransport::with_endpoint("k", "https://proxy.example.com/emails");
        assert_eq!(t.endpoint, "https://proxy.example.com/emails");
    }

    #[test]
    fn with_endpoint_trims_trailing_slash_before_suffix_check() {
        // `https://x/emails/` must be detected as already-terminal (after
        // trim), not double-appended.
        let t = ResendMailTransport::with_endpoint("k", "https://proxy.example.com/emails/");
        assert_eq!(t.endpoint, "https://proxy.example.com/emails");
    }

    #[test]
    fn with_endpoint_appends_for_paths_with_emails_substring() {
        // Regression: `contains("/emails")` would have skipped a base URL
        // like `/emails-archive/api`. `ends_with` is correct.
        let t = ResendMailTransport::with_endpoint("k", "https://x.example/emails-archive/api");
        assert_eq!(t.endpoint, "https://x.example/emails-archive/api/emails");
    }
}
