//! AWS SES (SendEmail v2 API) transport with sigv4-signed requests.
//!
//! Plain messages (no attachments) ride the `Content.Simple` JSON path.
//! Anything with attachments is rendered to RFC 5322 MIME via lettre,
//! base64-encoded, and sent as `Content.Raw.Data` — the only SES path
//! that supports attachments.
//!
//! Both paths forward `OutgoingMessage::headers`: `Content.Simple` in its
//! `Headers` list of `{Name, Value}` pairs, the raw path as real MIME
//! header lines. Which path a message takes is decided by whether it has
//! an attachment, which the caller who set `List-Unsubscribe` has no
//! reason to think about — so a header that rode only one of them would
//! vanish the first time somebody attached a PDF.
//!
//! One exception: `X-SES-TENANT-NAME`, `X-SES-CONFIGURATION-SET`, and
//! `X-SES-LIST-MANAGEMENT-OPTIONS` never reach either header list. Those
//! three select `SendEmail`'s `TenantName` / `ConfigurationSetName` /
//! `ListManagementOptions` body fields instead, so forwarding them as
//! ordinary headers too would duplicate the directive and leak it to the
//! recipient.
//!
//! A header name repeated more than once is not handled identically on
//! both shapes: `Content.Simple` forwards every entry, but the raw MIME
//! path keeps only the last value for a given name — the same behaviour
//! `mail/smtp.rs` has, and a limit of the underlying MIME header map
//! rather than a choice made here.

use crate::error::FrameworkError;
use crate::mail::address::Address;
use crate::mail::http_provider::{err, read_error_body, shared_client};
use crate::mail::transport::{MailTransport, OutgoingMessage};
use async_trait::async_trait;
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4::SigningParams;
use lettre::message::header::{HeaderName, HeaderValue};
use lettre::message::{
    Attachment as LettreAttachment, Mailbox, Message, MultiPart, SinglePart, header::ContentType,
};
use serde::Serialize;
use std::time::SystemTime;

/// AWS SES v2 (`SendEmail`) transport. Builds sigv4-signed JSON
/// requests; messages with attachments are rendered to RFC 5322 MIME
/// and sent as `Content.Raw.Data`.
///
/// The three optional send options - tenant, configuration set, and
/// subscription-list attribution - can be pinned per transport here or
/// overridden per message with the `X-SES-*` headers documented on
/// [`Self::tenant_name`]. A per-message header always wins, matching
/// Laravel's `SesV2Transport`, which merges message-derived options over
/// the configured ones.
pub struct SesMailTransport {
    access_key: String,
    secret_key: String,
    region: String,
    endpoint: String,
    tenant_name: Option<String>,
    configuration_set_name: Option<String>,
    list_management: Option<SesListManagementOptions>,
}

impl SesMailTransport {
    /// Build a transport using the region's default outbound endpoint
    /// (`https://email.<region>.amazonaws.com/v2/email/outbound-emails`).
    pub fn new(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        let region_s: String = region.into().to_lowercase();
        let endpoint = format!("https://email.{region_s}.amazonaws.com/v2/email/outbound-emails");
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            region: region_s,
            endpoint,
            tenant_name: None,
            configuration_set_name: None,
            list_management: None,
        }
    }

    /// Build a transport pointing at a custom endpoint (LocalStack,
    /// VPC endpoint, or regional override). The URL is normalized to
    /// include the trailing `/v2/email/outbound-emails` path.
    pub fn with_endpoint(
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
        region: impl Into<String>,
        endpoint: impl AsRef<str>,
    ) -> Self {
        let endpoint = endpoint.as_ref().trim_end_matches('/');
        let url = if endpoint.ends_with("/outbound-emails") {
            endpoint.to_string()
        } else {
            format!("{endpoint}/v2/email/outbound-emails")
        };
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            region: region.into().to_lowercase(),
            endpoint: url,
            tenant_name: None,
            configuration_set_name: None,
            list_management: None,
        }
    }

    /// Pin the SES tenant every message from this transport is
    /// attributed to (`TenantName` on `SendEmail`).
    ///
    /// A single message overrides it with the `X-SES-TENANT-NAME`
    /// header, the same name Laravel's `SesV2Transport` reads - a
    /// multi-tenant app usually wants the transport default and one
    /// per-tenant override, not one transport per tenant.
    pub fn tenant_name(mut self, name: impl Into<String>) -> Self {
        self.tenant_name = Some(name.into());
        self
    }

    /// Pin the SES configuration set that governs event publishing,
    /// dedicated-IP pools, and reputation tracking for these sends
    /// (`ConfigurationSetName` on `SendEmail`).
    ///
    /// Overridable per message with `X-SES-CONFIGURATION-SET`. Laravel
    /// only exposes this through its transport options array; the header
    /// is a Suprnova addition so per-message routing is expressible
    /// without a second transport.
    pub fn configuration_set_name(mut self, name: impl Into<String>) -> Self {
        self.configuration_set_name = Some(name.into());
        self
    }

    /// Attribute sends to an SES subscription list, so SES appends its
    /// managed unsubscribe links and honours the recipient's preference
    /// (`ListManagementOptions` on `SendEmail`).
    ///
    /// Overridable per message with `X-SES-LIST-MANAGEMENT-OPTIONS`,
    /// which takes Laravel's shape: `my-list`,
    /// `contactListName=my-list`, or `my-list; topicName=weekly`.
    pub fn list_management(
        mut self,
        contact_list_name: impl Into<String>,
        topic_name: Option<&str>,
    ) -> Self {
        self.list_management = Some(SesListManagementOptions {
            contact_list_name: contact_list_name.into(),
            topic_name: topic_name.map(str::to_string),
        });
        self
    }
}

#[derive(Serialize)]
struct SesBody {
    #[serde(rename = "FromEmailAddress")]
    from_email_address: String,
    #[serde(rename = "Destination")]
    destination: SesDestination,
    #[serde(rename = "ReplyToAddresses", skip_serializing_if = "Vec::is_empty")]
    reply_to_addresses: Vec<String>,
    #[serde(rename = "Content")]
    content: SesContent,
    #[serde(rename = "EmailTags", skip_serializing_if = "Vec::is_empty")]
    email_tags: Vec<SesTag>,
    #[serde(
        rename = "FeedbackForwardingEmailAddress",
        skip_serializing_if = "Option::is_none"
    )]
    feedback_forwarding_email_address: Option<String>,
    #[serde(
        rename = "ConfigurationSetName",
        skip_serializing_if = "Option::is_none"
    )]
    configuration_set_name: Option<String>,
    #[serde(
        rename = "ListManagementOptions",
        skip_serializing_if = "Option::is_none"
    )]
    list_management_options: Option<SesListManagementOptions>,
    #[serde(rename = "TenantName", skip_serializing_if = "Option::is_none")]
    tenant_name: Option<String>,
}

/// SES tags are key/value pairs. Both Suprnova `tags` (Vec<String>) and
/// `metadata` (BTreeMap) merge here: bare tags become `{Name: "_tag_<idx>", Value: t}`,
/// metadata becomes `{Name: k, Value: v}`. The duplicated keys are
/// disambiguated by Suprnova-side ordering.
#[derive(Serialize)]
struct SesTag {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Value")]
    value: String,
}

/// SES v2 `ListManagementOptions` - the subscription list (and optional
/// topic) a send is attributed to, so SES can append its managed
/// unsubscribe footer and suppress recipients who opted out.
#[derive(Serialize, Clone)]
struct SesListManagementOptions {
    #[serde(rename = "ContactListName")]
    contact_list_name: String,
    #[serde(rename = "TopicName", skip_serializing_if = "Option::is_none")]
    topic_name: Option<String>,
}

/// Per-message SES tenant override. Same header Laravel's
/// `SesV2Transport` reads.
const HEADER_TENANT_NAME: &str = "X-SES-TENANT-NAME";
/// Per-message SES configuration-set override.
const HEADER_CONFIGURATION_SET: &str = "X-SES-CONFIGURATION-SET";
/// Per-message SES subscription-list override.
const HEADER_LIST_MANAGEMENT: &str = "X-SES-LIST-MANAGEMENT-OPTIONS";

/// Read a MIME header off the outgoing message, case-insensitively.
///
/// Header names are case-insensitive per RFC 5322, and `msg.headers` is
/// free-form caller input - matching exactly would make
/// `x-ses-tenant-name` silently do nothing. An empty value is treated as
/// absent, matching Laravel's `?: null`.
fn ses_header<'a>(msg: &'a OutgoingMessage, name: &str) -> Option<&'a str> {
    msg.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// Parse the `X-SES-LIST-MANAGEMENT-OPTIONS` header.
///
/// Accepts the shapes Laravel's regex accepts, without the regex:
/// `my-list`, `contactListName=my-list`, `my-list; topicName=weekly`,
/// `contactListName=my-list;topicName=weekly`. An unparseable or empty
/// list name yields `None`, which falls through to the transport default
/// rather than shipping a malformed option AWS would reject.
fn parse_list_management(raw: &str) -> Option<SesListManagementOptions> {
    let (list_part, topic_part) = match raw.split_once(';') {
        Some((left, right)) => (left, Some(right)),
        None => (raw, None),
    };
    let list_raw = list_part.trim();
    let contact_list_name = match list_raw.strip_prefix("contactListName=") {
        Some(rest) => rest.trim(),
        None => list_raw,
    };
    if contact_list_name.is_empty() {
        return None;
    }
    let topic_name = topic_part.and_then(|raw_topic| {
        let trimmed = raw_topic.trim();
        let value = match trimmed.strip_prefix("topicName=") {
            Some(rest) => rest.trim(),
            None => trimmed,
        };
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    });
    Some(SesListManagementOptions {
        contact_list_name: contact_list_name.to_string(),
        topic_name,
    })
}

/// True when `name` is one of the three SES control headers consumed by
/// [`MailTransport::send`] above to select the tenant, configuration
/// set, or subscription list for the request.
///
/// This transport otherwise forwards every caller-supplied header onto
/// both SES content paths (see the module doc comment). These three are
/// the exception, because each already has a first-class `SendEmail`
/// body field (`TenantName` / `ConfigurationSetName` /
/// `ListManagementOptions`): forwarding them as ordinary headers too
/// would duplicate the directive on the Simple path and leak it into the
/// recipient's MIME on the Raw path.
fn is_ses_control_header(name: &str) -> bool {
    name.eq_ignore_ascii_case(HEADER_TENANT_NAME)
        || name.eq_ignore_ascii_case(HEADER_CONFIGURATION_SET)
        || name.eq_ignore_ascii_case(HEADER_LIST_MANAGEMENT)
}

/// One entry of SES v2's `Content.Simple.Headers` list.
///
/// AWS calls the shape `MessageHeader` and spells its fields `Name` /
/// `Value` — the same pair Postmark uses for its own `Headers` array
/// (`mail/postmark.rs:107-113`), and the same pair `SesTag` above uses.
/// A misspelled field here fails the entire `SendEmail` call, not just the
/// header, so the renames are load-bearing.
#[derive(Serialize)]
struct SesHeader {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Value")]
    value: String,
}

#[derive(Serialize)]
struct SesDestination {
    #[serde(rename = "ToAddresses", skip_serializing_if = "Vec::is_empty")]
    to: Vec<String>,
    #[serde(rename = "CcAddresses", skip_serializing_if = "Vec::is_empty")]
    cc: Vec<String>,
    #[serde(rename = "BccAddresses", skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<String>,
}

#[derive(Serialize)]
enum SesContent {
    #[serde(rename = "Simple")]
    Simple(SesSimple),
    #[serde(rename = "Raw")]
    Raw(SesRaw),
}

#[derive(Serialize)]
struct SesSimple {
    #[serde(rename = "Subject")]
    subject: SesData,
    #[serde(rename = "Body")]
    body: SesBodyContent,
    /// Caller-set MIME headers. Omitted entirely when empty so the request
    /// body stays byte-identical to what it was for messages that set none.
    #[serde(rename = "Headers", skip_serializing_if = "Vec::is_empty")]
    headers: Vec<SesHeader>,
}

#[derive(Serialize)]
struct SesRaw {
    #[serde(rename = "Data")]
    data: String,
}

#[derive(Serialize)]
struct SesData {
    #[serde(rename = "Data")]
    data: String,
}

#[derive(Serialize)]
struct SesBodyContent {
    #[serde(rename = "Html", skip_serializing_if = "Option::is_none")]
    html: Option<SesData>,
    #[serde(rename = "Text", skip_serializing_if = "Option::is_none")]
    text: Option<SesData>,
}

fn addrs_only(a: &[Address]) -> Vec<String> {
    a.iter().map(|x| x.to_string()).collect()
}

/// Reject a caller-supplied header name that is not safe on either SES
/// content path. One rule, not two: a name accepted here is accepted by
/// `custom_header` below (the raw MIME path's `HeaderName::new_from_ascii`)
/// as well, so acceptance never depends on whether the message happens to
/// carry an attachment.
///
/// Two checks make up that one rule:
/// - CR, LF and NUL are the injection characters — a caller-supplied
///   string containing one turns into a second header on the raw path, or
///   corrupts the `Content.Simple.Headers` list. Mirrors the identical
///   guard in `mail/mailgun.rs`, which is the only other transport that
///   has one.
/// - Everything `HeaderName::new_from_ascii` itself rejects — an empty
///   name, a name over 76 bytes, a non-ASCII byte, or a `:` or space in
///   the name — is rejected here too. That check does not see CR/LF/NUL:
///   they are valid ASCII bytes, so the first check is still needed.
///
/// Applied before the content branch, not inside it: attachments are the
/// only thing that switches SES from `Simple` to `Raw`, and a message that
/// is rejected with an attachment but accepted without one would be the
/// worst possible shape for this check.
fn validate_header_name(name: &str) -> Result<(), FrameworkError> {
    if name.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(FrameworkError::param(format!(
            "SES: header name contains illegal character (CR, LF, or NUL): {name:?}"
        )));
    }
    HeaderName::new_from_ascii(name.to_string()).map_err(|_| {
        FrameworkError::param(format!(
            "SES: invalid header name (must be non-empty, at most 76 bytes, ASCII, \
             and free of ':' and spaces): {name:?}"
        ))
    })?;
    Ok(())
}

/// Build a lettre header from a caller-supplied name/value pair for the raw
/// MIME path. `validate_header_name` above enforces the identical rule
/// up front, so `HeaderName::new_from_ascii` here should never actually
/// reject anything it hasn't already rejected — this call exists to
/// produce the `HeaderName` type `raw_header` needs, not as a second gate.
/// Same helper, same shape, as `mail/smtp.rs`.
fn custom_header(name: &str, value: &str) -> Result<HeaderValue, FrameworkError> {
    let header_name = HeaderName::new_from_ascii(name.to_string())
        .map_err(|e| FrameworkError::internal(format!("SES header name {name}: {e}")))?;
    Ok(HeaderValue::new(header_name, value.to_string()))
}

fn uri_host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .map(|u| match u.port() {
            Some(p) => format!("{}:{}", u.host_str().unwrap_or(""), p),
            None => u.host_str().unwrap_or("").to_string(),
        })
        .unwrap_or_default()
}

fn address_to_mailbox(a: &Address) -> Result<Mailbox, FrameworkError> {
    let parsed: lettre::Address = a
        .email
        .parse()
        .map_err(|e| FrameworkError::internal(format!("SES parse address {}: {e}", a.email)))?;
    Ok(Mailbox::new(a.name.clone(), parsed))
}

fn build_mime(msg: &OutgoingMessage) -> Result<Vec<u8>, FrameworkError> {
    let mut builder = Message::builder()
        .from(address_to_mailbox(&msg.from)?)
        .subject(&msg.subject);
    for a in &msg.to {
        builder = builder.to(address_to_mailbox(a)?);
    }
    for a in &msg.cc {
        builder = builder.cc(address_to_mailbox(a)?);
    }
    for a in &msg.bcc {
        builder = builder.bcc(address_to_mailbox(a)?);
    }
    for a in &msg.reply_to {
        builder = builder.reply_to(address_to_mailbox(a)?);
    }

    // Caller-set headers ride the envelope here exactly as they do on the
    // SMTP transport (`mail/smtp.rs:81-83`). `Content.Simple` carries the
    // same list in its `Headers` field; both paths have to forward them, or
    // attaching a file silently drops the caller's `List-Unsubscribe`.
    //
    // SES control headers (`X-SES-TENANT-NAME` and friends) are the one
    // exception: `MailTransport::send` below consumes them into top-level
    // `SendEmail` options, so they must not also ride along as ordinary
    // MIME headers, or the directive would leak into the recipient's inbox.
    for (name, value) in &msg.headers {
        if is_ses_control_header(name) {
            continue;
        }
        builder = builder.raw_header(custom_header(name, value)?);
    }

    let mut alternative = MultiPart::alternative().build();
    if let Some(text) = &msg.text {
        alternative = alternative.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(text.clone()),
        );
    }
    if let Some(html) = &msg.html {
        alternative = alternative.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(html.clone()),
        );
    }

    let mut mixed = MultiPart::mixed().multipart(alternative);
    for att in &msg.attachments {
        let ct: ContentType = att.content_type.parse().map_err(|e| {
            FrameworkError::internal(format!(
                "SES attachment content-type {}: {e}",
                att.content_type
            ))
        })?;
        mixed = mixed
            .singlepart(LettreAttachment::new(att.filename.clone()).body(att.content.clone(), ct));
    }

    let email = builder
        .multipart(mixed)
        .map_err(|e| FrameworkError::internal(format!("SES build mime: {e}")))?;
    Ok(email.formatted())
}

#[async_trait]
impl MailTransport for SesMailTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        // Validate once, ahead of the content branch, so the verdict does not
        // depend on whether this particular message happens to have an
        // attachment. `validate_header_name` enforces the same rule
        // `build_mime`'s `HeaderName::new_from_ascii` would apply on the raw
        // path — one rule for both content shapes, not two.
        for (name, _) in &msg.headers {
            validate_header_name(name)?;
        }

        let content = if msg.attachments.is_empty() {
            SesContent::Simple(SesSimple {
                subject: SesData {
                    data: msg.subject.clone(),
                },
                body: SesBodyContent {
                    html: msg.html.clone().map(|h| SesData { data: h }),
                    text: msg.text.clone().map(|t| SesData { data: t }),
                },
                headers: msg
                    .headers
                    .iter()
                    .filter(|(name, _)| !is_ses_control_header(name))
                    .map(|(name, value)| SesHeader {
                        name: name.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            })
        } else {
            let mime = build_mime(msg)?;
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&mime);
            SesContent::Raw(SesRaw { data: encoded })
        };

        // SES tag rule: Name and Value chars are restricted to
        // `[A-Za-z0-9_-]`. Drop entries that violate the rule rather
        // than ship malformed JSON to AWS (which would reject the
        // whole send call). Log the drop so misconfigured caller code
        // is observable rather than mysteriously losing tags.
        //
        // The synthetic `tag_<i>` keys we generate are by construction
        // valid; only caller-supplied tag VALUES and the full
        // metadata KEY/VALUE pair can violate the allowlist.
        fn ses_tag_valid(s: &str) -> bool {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
        let mut email_tags: Vec<SesTag> = Vec::new();
        for (i, t) in msg.tags.iter().enumerate() {
            if !ses_tag_valid(t) {
                tracing::warn!(
                    tag_index = i,
                    tag_value = %t,
                    "SES: dropping msg.tag at index {i} — value contains chars outside \
                     [A-Za-z0-9_-]; AWS would reject the entire send call"
                );
                continue;
            }
            email_tags.push(SesTag {
                name: format!("tag_{i}"),
                value: t.clone(),
            });
        }
        for (k, v) in &msg.metadata {
            if !ses_tag_valid(k) || !ses_tag_valid(v) {
                tracing::warn!(
                    metadata_key = %k,
                    metadata_value = %v,
                    "SES: dropping msg.metadata entry — key or value contains chars outside \
                     [A-Za-z0-9_-]; AWS would reject the entire send call"
                );
                continue;
            }
            email_tags.push(SesTag {
                name: k.clone(),
                value: v.clone(),
            });
        }

        // Per-message header wins over the transport default, matching
        // Laravel's `SesV2Transport`, which starts from the configured
        // options array and then overwrites the message-derived keys.
        let tenant_name = ses_header(msg, HEADER_TENANT_NAME)
            .map(str::to_string)
            .or_else(|| self.tenant_name.clone());
        let configuration_set_name = ses_header(msg, HEADER_CONFIGURATION_SET)
            .map(str::to_string)
            .or_else(|| self.configuration_set_name.clone());
        let list_management_options = ses_header(msg, HEADER_LIST_MANAGEMENT)
            .and_then(parse_list_management)
            .or_else(|| self.list_management.clone());

        let body = SesBody {
            from_email_address: msg.from.to_string(),
            destination: SesDestination {
                to: addrs_only(&msg.to),
                cc: addrs_only(&msg.cc),
                bcc: addrs_only(&msg.bcc),
            },
            reply_to_addresses: addrs_only(&msg.reply_to),
            content,
            email_tags,
            feedback_forwarding_email_address: msg.return_path.as_ref().map(|a| a.to_string()),
            configuration_set_name,
            list_management_options,
            tenant_name,
        };
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| FrameworkError::internal(format!("SES encode: {e}")))?;

        let identity = aws_credential_types::Credentials::new(
            self.access_key.clone(),
            self.secret_key.clone(),
            None,
            None,
            "suprnova-mail",
        )
        .into();
        let settings = SigningSettings::default();
        let signing_params = SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("ses")
            .time(SystemTime::now())
            .settings(settings)
            .build()
            .map_err(|e| FrameworkError::internal(format!("SES sigv4 params: {e}")))?
            .into();

        let request_builder = http::Request::builder()
            .method("POST")
            .uri(&self.endpoint)
            .header("content-type", "application/json")
            .header("host", uri_host(&self.endpoint));
        let request = request_builder
            .body(body_bytes.clone())
            .map_err(|e| FrameworkError::internal(format!("SES http build: {e}")))?;

        let signable = SignableRequest::new(
            "POST",
            &self.endpoint,
            request
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str(), v.to_str().unwrap_or(""))),
            SignableBody::Bytes(&body_bytes),
        )
        .map_err(|e| FrameworkError::internal(format!("SES signable: {e}")))?;

        let signing_output = sign(signable, &signing_params)
            .map_err(|e| FrameworkError::internal(format!("SES sign: {e}")))?;
        let (signing_instructions, _signature) = signing_output.into_parts();

        let mut rb = shared_client()
            .post(&self.endpoint)
            .header("content-type", "application/json");
        for (name, value) in signing_instructions.headers() {
            rb = rb.header(name, value);
        }

        let resp = rb
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| FrameworkError::internal(format!("SES transport: {e}")))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = read_error_body(resp).await;
            return Err(err("SES", status, body));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ses"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_uses_region_aware_endpoint() {
        let t = SesMailTransport::new("AK", "SK", "us-east-1");
        assert_eq!(
            t.endpoint,
            "https://email.us-east-1.amazonaws.com/v2/email/outbound-emails"
        );
        assert_eq!(t.region, "us-east-1");
    }

    #[test]
    fn new_lowercases_uppercase_region() {
        let t = SesMailTransport::new("AK", "SK", "US-EAST-1");
        assert_eq!(t.region, "us-east-1");
        assert_eq!(
            t.endpoint,
            "https://email.us-east-1.amazonaws.com/v2/email/outbound-emails"
        );
    }

    #[test]
    fn with_endpoint_lowercases_uppercase_region() {
        let t = SesMailTransport::with_endpoint("AK", "SK", "EU-West-2", "https://x.example");
        assert_eq!(t.region, "eu-west-2");
    }

    #[test]
    fn with_endpoint_appends_path_when_missing() {
        let t = SesMailTransport::with_endpoint("AK", "SK", "us-west-2", "https://proxy.example");
        assert_eq!(t.endpoint, "https://proxy.example/v2/email/outbound-emails");
    }

    #[test]
    fn with_endpoint_preserves_full_path() {
        let t = SesMailTransport::with_endpoint(
            "AK",
            "SK",
            "us-west-2",
            "https://proxy.example/v2/email/outbound-emails",
        );
        assert_eq!(t.endpoint, "https://proxy.example/v2/email/outbound-emails");
    }

    #[test]
    fn with_endpoint_appends_for_paths_with_outbound_emails_substring() {
        // Regression: `contains("/outbound-emails")` would have skipped a base
        // URL like `/outbound-emails-archive/api`. `ends_with` is correct.
        let t = SesMailTransport::with_endpoint(
            "AK",
            "SK",
            "us-west-2",
            "https://x.example/outbound-emails-archive/api",
        );
        assert_eq!(
            t.endpoint,
            "https://x.example/outbound-emails-archive/api/v2/email/outbound-emails"
        );
    }

    #[test]
    fn uri_host_preserves_port_when_non_default() {
        assert_eq!(
            uri_host("https://email.us-east-1.amazonaws.com/v2/email/outbound-emails"),
            "email.us-east-1.amazonaws.com"
        );
        assert_eq!(
            uri_host("http://127.0.0.1:38291/v2/email/outbound-emails"),
            "127.0.0.1:38291"
        );
        assert_eq!(
            uri_host("https://example.com:8443/v2/email/outbound-emails"),
            "example.com:8443"
        );
    }
}
