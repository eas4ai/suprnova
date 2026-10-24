//! Self-contained toolbar markup. Record text is never interpreted as markup or JavaScript.

use bytes::Bytes;
use hyper::HeaderMap;
use hyper::header::{self, HeaderValue};

use super::{WireResponse, collector::Record};

#[derive(Clone)]
pub(crate) struct SkipInjection;

pub(super) fn injectable(response: &WireResponse) -> bool {
    let headers = response.headers();
    !response.status().is_redirection()
        && response.status() != hyper::StatusCode::NO_CONTENT
        && response.status() != hyper::StatusCode::NOT_MODIFIED
        && response.extensions().get::<SkipInjection>().is_none()
        && !headers.contains_key(header::CONTENT_DISPOSITION)
        && !headers.contains_key("x-inertia")
        && !headers.contains_key(header::CONTENT_ENCODING)
        && headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
            })
}

pub(super) fn inject(
    bytes: &Bytes,
    record: &Record,
    headers: &mut HeaderMap,
    keep: usize,
) -> Bytes {
    let Ok(document) = std::str::from_utf8(bytes) else {
        return bytes.clone();
    };
    let lower = document.to_ascii_lowercase();
    let at = lower
        .rfind("</body>")
        .or_else(|| lower.rfind("</html>"))
        .unwrap_or(document.len());
    let nonce = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .or_else(|| headers.get(header::CONTENT_SECURITY_POLICY_REPORT_ONLY))
        .and_then(|value| value.to_str().ok())
        .and_then(|policy| policy.split("'nonce-").nth(1))
        .and_then(|tail| tail.split('\'').next());
    let nonce = nonce
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    for name in [
        header::CONTENT_SECURITY_POLICY,
        header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
    ] {
        let policies: Vec<_> = headers
            .get_all(&name)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(|policy| allow_toolbar(policy, &nonce))
            .collect();
        if !policies.is_empty() {
            headers.remove(&name);
            for policy in policies {
                if let Ok(value) = HeaderValue::from_str(&policy) {
                    headers.append(name.clone(), value);
                }
            }
        }
    }
    let nonce = format!(" nonce=\"{}\"", escape(&nonce));
    let data = record
        .json()
        .to_string()
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let markup = format!(
        "<aside id=\"suprnova-debugbar\" data-keep=\"{keep}\" aria-label=\"Suprnova request diagnostics\"><nav aria-label=\"Diagnostic panels\"></nav><section hidden><h2></h2><pre></pre></section><output aria-live=\"polite\"></output></aside>\
         <style{nonce}>{}</style><script{nonce} type=\"application/json\" id=\"suprnova-debugbar-record\" data-endpoint=\"{}\">{data}</script><script{nonce}>{}</script>",
        include_str!("toolbar.css"),
        escape(&record.endpoint),
        include_str!("toolbar.js"),
    );
    for name in [
        header::CONTENT_LENGTH,
        header::ETAG,
        header::LAST_MODIFIED,
        header::ACCEPT_RANGES,
    ] {
        headers.remove(name);
    }
    Bytes::from(format!("{}{}{}", &document[..at], markup, &document[at..]))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// A nonce permits only the diagnostic elements. Retain the response's other
// sources; policies without an explicit element directive inherit their default.
fn allow_toolbar(policy: &str, nonce: &str) -> String {
    let mut directives: Vec<String> = policy
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    let default = directive(&directives, "default-src")
        .and_then(|index| {
            directives[index]
                .split_once(' ')
                .map(|(_, sources)| sources)
        })
        .unwrap_or("*")
        .to_owned();
    for (source, element) in [
        ("script-src", "script-src-elem"),
        ("style-src", "style-src-elem"),
    ] {
        allow_nonce(&mut directives, source, &default, nonce);
        if directive(&directives, element).is_some() {
            allow_nonce(&mut directives, element, &default, nonce);
        }
    }
    // The framework's isolated development error page otherwise inherits none.
    if directive(&directives, "connect-src").is_none() && default == "'none'" {
        directives.push("connect-src 'self'".into());
    }
    directives.join("; ")
}

fn directive(directives: &[String], name: &str) -> Option<usize> {
    directives
        .iter()
        .position(|value| value.split_whitespace().next() == Some(name))
}

fn allow_nonce(directives: &mut Vec<String>, name: &str, inherited: &str, nonce: &str) {
    let index = directive(directives, name);
    let policy = index.map_or_else(
        || format!("{name} {inherited}"),
        |index| directives[index].clone(),
    );
    let allows_inline = policy.contains("'unsafe-inline'")
        && !policy.contains("'nonce-")
        && !policy.contains("'sha");
    if allows_inline
        || policy
            .split_whitespace()
            .any(|source| source == format!("'nonce-{nonce}'"))
    {
        return;
    }
    let sources = policy
        .split_whitespace()
        .filter(|source| *source != "'none'")
        .collect::<Vec<_>>()
        .join(" ");
    let policy = format!("{sources} 'nonce-{nonce}'");
    match index {
        Some(index) => directives[index] = policy,
        None => directives.push(policy),
    }
}
