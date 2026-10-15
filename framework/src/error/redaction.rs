//! One privacy boundary serves both debug HTML and outbound reports.

use serde_json::Value;
use std::borrow::Cow;

/// What a redacted value is shown as.
pub(crate) const REDACTED: &str = "[redacted]";

/// A header or query parameter whose name contains one of these, in any
/// letter case, has its value redacted.
const SECRET_NAME_PARTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "key",
    "signature",
    "csrf",
    "xsrf",
    "session",
    "cookie",
    "authorization",
];

/// Headers whose values are always redacted.
const CREDENTIAL_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// Whether a header's value is redacted on the page.
pub(crate) fn header_value_is_secret(name: &str) -> bool {
    CREDENTIAL_HEADERS
        .iter()
        .any(|credential| name.eq_ignore_ascii_case(credential))
        || names_a_secret(name)
}

/// Whether `name` contains `token`, `secret`, `password`, `key` or
/// `signature`, in any letter case.
pub(crate) fn names_a_secret(name: &str) -> bool {
    let name = name.to_lowercase();
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// `text` with every secret the page must not show replaced: the value of
/// each parameter or text field whose name names a secret
/// ([`redact_secret_parameters`]), and the password of each URL
/// ([`redact_url_passwords`]).
///
/// Every value the page shows from the request or the error goes through
/// this, not only the query: a `Referer`, an `X-Original-URI` or an
/// error message can carry the same URL.
pub(crate) fn redact_text(text: &str) -> Cow<'_, str> {
    match redact_secret_parameters(text) {
        Cow::Borrowed(text) => redact_url_passwords(text),
        Cow::Owned(text) => Cow::Owned(redact_url_passwords(&text).into_owned()),
    }
}

/// Whether `c` ends a field's name, looking back from its `=` or `:`.
fn ends_parameter_name(c: char) -> bool {
    c.is_ascii_whitespace()
        || matches!(
            c,
            '?' | '&'
                | ';'
                | ','
                | '#'
                | '/'
                | '"'
                | '\''
                | '<'
                | '>'
                | '`'
                | ':'
                | '='
                | '{'
                | '}'
        )
}

/// Query delimiters preserve following fields. Spaces do not safely end a
/// credential, which may contain several words.
fn ends_parameter_value(c: char) -> bool {
    matches!(
        c,
        '\r' | '\n' | '&' | ';' | '#' | '"' | '\'' | '<' | '>' | '`'
    )
}

/// Scrub named credentials in query strings, diagnostics and serialized fields.
///
/// Names are percent-decoded, and quoted names are recognized too. Header
/// values and quoted values have no trustworthy text boundary, so the rest
/// of their line is discarded. Query delimiters still preserve safe fields.
fn redact_secret_parameters(text: &str) -> Cow<'_, str> {
    let mut redacted = String::new();
    let mut copied = 0;
    for (at, separator) in text.match_indices(['=', ':']) {
        if at < copied {
            continue;
        }
        let before = text[..at].trim_end_matches([' ', '\t']);
        let before = before.trim_end_matches(['"', '\'']);
        let name_start = before.rfind(ends_parameter_name).map_or(0, |at| at + 1);
        let name = decoded_parameter_name(&before[name_start..]);
        if !names_a_secret(&name) && !pii_name(&name) {
            continue;
        }
        let value_and_rest = text[at + 1..].trim_start_matches([' ', '\t']);
        let value_start = text.len() - value_and_rest.len();
        let value_end = if separator == ":" || value_and_rest.starts_with(['"', '\'']) {
            value_and_rest.find(['\r', '\n'])
        } else {
            value_and_rest.find(ends_parameter_value)
        }
        .unwrap_or(value_and_rest.len());
        redacted.push_str(&text[copied..value_start]);
        redacted.push_str(REDACTED);
        copied = value_start + value_end;
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    redacted.push_str(&text[copied..]);
    Cow::Owned(redacted)
}

/// A parameter name, percent-decoded the way a query string is.
fn decoded_parameter_name(raw: &str) -> String {
    url::form_urlencoded::parse(raw.as_bytes())
        .next()
        .map(|(name, _)| name.into_owned())
        .unwrap_or_default()
}

/// `text` with the password of every `scheme://user:password@host` URL
/// replaced, and the rest of the URL kept readable.
///
/// A URL runs from `://` to the next whitespace or quote. Its user info
/// ends at the last `@` in it and the password starts after the first `:`
/// in the user info, so a password holding an unencoded `/`, `?` or `#`,
/// as generated secrets often do, is still found. A URL whose path holds
/// both `:` and `@` loses more than its password; that over-redaction is
/// the safe side.
pub(crate) fn redact_url_passwords(text: &str) -> Cow<'_, str> {
    if !text.contains("://") {
        return Cow::Borrowed(text);
    }
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        redacted.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let credentials = url
            .rfind('@')
            .and_then(|at_sign| url[..at_sign].find(':').map(|colon| (colon, at_sign)));
        match credentials {
            Some((colon, at_sign)) => {
                redacted.push_str(&url[..=colon]);
                redacted.push_str(REDACTED);
                redacted.push_str(&url[at_sign..]);
            }
            None => redacted.push_str(url),
        }
        rest = &tail[end..];
    }
    redacted.push_str(rest);
    Cow::Owned(redacted)
}

/// Redact structured fields recursively before any driver sees them.
pub(crate) fn scrub(value: &mut Value, send_pii: bool) {
    if let Some(request) = value.get_mut("request").and_then(Value::as_object_mut) {
        for field in ["data", "body", "env"] {
            request.remove(field);
        }
    }
    scrub_at(value, send_pii, 0);
}

fn scrub_at(value: &mut Value, send_pii: bool, depth: usize) {
    if depth > 12 {
        *value = Value::String(REDACTED.into());
        return;
    }
    match value {
        Value::Object(fields) => {
            for (name, field) in fields {
                let lower = name.to_ascii_lowercase();
                if header_value_is_secret(name)
                    || lower == "hidden"
                    || (!send_pii && pii_name(&lower))
                {
                    if field.as_object().is_some_and(|object| {
                        object.contains_key("type") && object.contains_key("value")
                    }) {
                        *field = serde_json::json!({"type":"string","value":REDACTED});
                    } else {
                        *field = Value::String(REDACTED.into());
                    }
                } else {
                    scrub_at(field, send_pii, depth + 1);
                }
            }
        }
        Value::Array(values) => {
            values.truncate(128);
            for value in values {
                scrub_at(value, send_pii, depth + 1);
            }
        }
        Value::String(text) => {
            *text = redact_text(text).into_owned();
            if text.len() > 16_384 {
                let mut end = 16_384;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
        }
        _ => {}
    }
}

fn pii_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("email")
        || name.ends_with("forwarded-for")
        || matches!(name.as_str(), "forwarded" | "from" | "ip_address")
        || name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| matches!(part, "ip" | "ipv6" | "ipaddress"))
}

#[cfg(test)]
mod tests {
    use super::redact_text;

    #[test]
    fn report_004_text_credentials_include_colon_fields_and_quoted_or_multiword_values() {
        for (text, secrets) in [
            ("Authorization: Bearer private-auth", vec!["private-auth"]),
            (
                "Cookie: SID=private-cookie; theme=private-theme",
                vec!["private-cookie", "private-theme"],
            ),
            (
                r#"upstream rejected {"password":"private-json second-word"}"#,
                vec!["private-json", "second-word"],
            ),
            (
                r#"password="private-quoted second-word""#,
                vec!["private-quoted", "second-word"],
            ),
            (
                "password=private-first second-word",
                vec!["private-first", "second-word"],
            ),
        ] {
            let output = redact_text(text);
            for secret in secrets {
                assert!(!output.contains(secret), "text leaked {secret}: {output}");
            }
        }
    }

    #[test]
    fn report_004_text_scrubbing_keeps_safe_diagnostics_and_query_fields() {
        let ordinary = "database connection failed: retry after 5 seconds\nstatus=503";
        assert_eq!(redact_text(ordinary), ordinary);
        assert_eq!(
            redact_text("request /orders?%70assword=private-query&page=7"),
            "request /orders?%70assword=[redacted]&page=7"
        );
        assert_eq!(
            redact_text(
                "upstream rejected\nAuthorization: Bearer private-auth\nretry after 5 seconds"
            ),
            "upstream rejected\nAuthorization: [redacted]\nretry after 5 seconds"
        );
    }
}
