//! Read transfer measurements without guessing that omitted bodies are empty.

use super::{PageBudget, Resource, collect::Capture, parse_url, same_origin};
use crate::FrameworkError;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

#[derive(Deserialize)]
struct Har {
    log: Log,
}
#[derive(Deserialize)]
struct Log {
    entries: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    request: Request,
    response: Response,
}
#[derive(Deserialize)]
struct Request {
    url: String,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    cookies: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    status: u16,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    cookies: Vec<serde_json::Value>,
    #[serde(default)]
    body_size: Option<i64>,
    content: Content,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Content {
    size: u64,
    #[serde(default)]
    mime_type: String,
    text: Option<String>,
    encoding: Option<String>,
}

pub(super) fn read(json: &str, budget: &PageBudget) -> Result<Capture, FrameworkError> {
    if json.len() > budget.max_response_bytes.saturating_mul(4) {
        return Err(FrameworkError::bad_request(
            "page budget HAR exceeds input limit",
        ));
    }
    let har: Har = serde_json::from_str(json)
        .map_err(|e| FrameworkError::bad_request(format!("page budget HAR: {e}")))?;
    if har.log.entries.len() > budget.max_resources {
        return Err(FrameworkError::bad_request(
            "page budget HAR exceeds resource limit",
        ));
    }
    let first = har
        .log
        .entries
        .first()
        .ok_or_else(|| FrameworkError::bad_request("page budget HAR has no entries"))?;
    let page = parse_url(&first.request.url)?;
    let mut capture = Capture::new(page.clone());
    let mut origin = page.clone();
    let mut documents = std::collections::BTreeSet::from([page.to_string()]);
    for (index, entry) in har.log.entries.into_iter().enumerate() {
        let url = parse_url(&entry.request.url)?;
        let mime = entry.response.content.mime_type;
        let document = index == 0 || documents.contains(url.as_str());
        if document && (300..400).contains(&entry.response.status) {
            for header in &entry.response.headers {
                if header.name.eq_ignore_ascii_case("location") {
                    if let Ok(next) = url.join(&header.value) {
                        documents.insert(next.to_string());
                    }
                }
            }
        }
        if document && url.host_str() == page.host_str() {
            origin = url.clone();
        }
        let mut headers: Vec<_> = entry
            .response
            .headers
            .into_iter()
            .map(|h| (h.name, h.value))
            .collect();
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".into(), mime));
        }
        if !entry.response.cookies.is_empty()
            && !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        {
            headers.push(("Set-Cookie".into(), "present in HAR cookie list".into()));
        }
        let body = match (
            entry.response.content.text,
            entry.response.content.encoding.as_deref(),
        ) {
            (Some(text), Some("base64")) => {
                if text.len()
                    > budget
                        .max_response_bytes
                        .saturating_mul(4)
                        .saturating_div(3)
                        .saturating_add(4)
                {
                    return Err(FrameworkError::bad_request(format!(
                        "page budget HAR body {url} exceeds response limit"
                    )));
                }
                Some(STANDARD.decode(text).map_err(|e| {
                    FrameworkError::bad_request(format!("page budget HAR body {url}: {e}"))
                })?)
            }
            (Some(text), None | Some("")) => Some(text.into_bytes()),
            (Some(_), Some(encoding)) => {
                return Err(FrameworkError::bad_request(format!(
                    "page budget HAR body {url}: unsupported encoding {encoding}"
                )));
            }
            (None, _) => None,
        };
        if body
            .as_ref()
            .is_some_and(|b| b.len() > budget.max_response_bytes)
        {
            return Err(FrameworkError::bad_request(format!(
                "page budget HAR body {url} exceeds response limit"
            )));
        }
        let transfer = entry
            .response
            .body_size
            .and_then(|size| u64::try_from(size).ok())
            .unwrap_or(entry.response.content.size);
        capture.resources.push(Resource {
            url,
            status: entry.response.status,
            headers,
            body,
            transfer,
            decoded: entry.response.content.size,
            request_cookie: !entry.request.cookies.is_empty()
                || entry
                    .request
                    .headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("cookie") && !h.value.is_empty()),
            document,
            manifest_icon: false,
            manifest: false,
        });
    }
    // Resolve fallback pairs and linked manifests when the capture includes bodies.
    // A bodyless export still carries all measured entries, not just parseable links.
    let mut manifest_urls = std::collections::BTreeSet::new();
    let mut icon_urls = std::collections::BTreeSet::new();
    let mut resources = std::mem::take(&mut capture.resources);
    for resource in &resources {
        if !same_origin(&origin, &resource.url)
            && !(resource.document && resource.url.host_str() == page.host_str())
            && let Some(host) = resource.url.host_str()
        {
            capture.hosts.insert(host.into());
        }
        for link in capture.references(resource, budget.max_resources)?.links {
            if !same_origin(&origin, &link.url)
                && let Some(host) = link.url.host_str()
            {
                capture.hosts.insert(host.into());
            }
            if link.manifest {
                manifest_urls.insert(link.url.to_string());
            }
        }
    }
    for resource in &mut resources {
        if manifest_urls.contains(resource.url.as_str()) {
            resource.manifest = true;
            if resource.body.is_some() {
                for link in capture.references(resource, budget.max_resources)?.links {
                    if !same_origin(&origin, &link.url)
                        && let Some(host) = link.url.host_str()
                    {
                        capture.hosts.insert(host.into());
                    }
                    if link.manifest_icon {
                        icon_urls.insert(link.url.to_string());
                    }
                }
            }
        }
    }
    for resource in &mut resources {
        resource.manifest_icon = icon_urls.contains(resource.url.as_str());
    }
    capture.resources = resources;
    Ok(capture)
}
