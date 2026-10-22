//! Fetch a finite asset graph with one collector for both request paths.

use super::{PageAudit, PageBudget, Resource, discover, same_origin};
use crate::{FrameworkError, http_client::Http, testing::TestClient};
use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use url::Url;

pub(super) struct Capture {
    pub page: Url,
    pub resources: Vec<Resource>,
    pub hosts: BTreeSet<String>,
    pub alternatives: BTreeSet<String>,
    pub image_groups: Vec<Vec<String>>,
}

impl Capture {
    pub(super) fn new(page: Url) -> Self {
        Self {
            page,
            resources: Vec::new(),
            hosts: BTreeSet::new(),
            alternatives: BTreeSet::new(),
            image_groups: Vec::new(),
        }
    }

    pub(super) fn references(
        &mut self,
        resource: &Resource,
    ) -> Result<discover::References, FrameworkError> {
        let Some(bytes) = &resource.body else {
            return Ok(discover::References::default());
        };
        let mut refs = if resource.document && resource.mime().eq_ignore_ascii_case("text/html") {
            discover::html(&resource.url, &String::from_utf8_lossy(bytes))
        } else if resource.mime().eq_ignore_ascii_case("text/css")
            || resource.url.path().ends_with(".css")
        {
            discover::css(&resource.url, &String::from_utf8_lossy(bytes))
        } else if resource.manifest {
            discover::manifest(&resource.url, bytes)?
        } else {
            discover::References::default()
        };
        self.alternatives.append(&mut refs.alternatives);
        self.image_groups.append(&mut refs.image_groups);
        Ok(refs)
    }
}

pub(super) async fn audit_http(
    budget: &PageBudget,
    page: Url,
) -> Result<PageAudit, FrameworkError> {
    Http::without_global_configuration(|| async {
        run(budget, page, |url| async move {
            let response = Http::get(url.to_string())
                .no_redirects()
                .timeout(budget.request_timeout)
                .connect_timeout(budget.request_timeout)
                .max_response_bytes(budget.max_response_bytes)
                .header("Accept-Encoding", "identity")
                .send()
                .await?;
            let status = response.status();
            let headers = [
                "Content-Type",
                "Cache-Control",
                "Set-Cookie",
                "Location",
                "Content-Length",
            ]
            .into_iter()
            .filter_map(|name| response.header(name).map(|value| (name.into(), value)))
            .collect();
            let body = response.bytes().await?.to_vec();
            Ok(Resource {
                url,
                status,
                headers,
                transfer: body.len() as u64,
                decoded: body.len() as u64,
                body: Some(body),
                request_cookie: false,
                document: false,
                manifest_icon: false,
                manifest: false,
            })
        })
        .await
    })
    .await
}

pub(crate) async fn audit_test(
    client: &TestClient,
    budget: &PageBudget,
    page: Url,
) -> Result<PageAudit, FrameworkError> {
    run(budget, page, |url| {
        client.budget_resource(url, budget.max_response_bytes, budget.request_timeout)
    })
    .await
}

async fn run<F, Fut>(budget: &PageBudget, page: Url, fetch: F) -> Result<PageAudit, FrameworkError>
where
    F: Fn(Url) -> Fut,
    Fut: Future<Output = Result<Resource, FrameworkError>>,
{
    super::fingerprint_regex(&budget.fingerprint_pattern)?;
    let collect = async {
        let mut capture = Capture::new(page.clone());
        let mut origin = page.clone();
        let mut queue = VecDeque::from([(page, true, false, false)]);
        let mut seen = BTreeSet::new();
        while let Some((mut url, document, manifest_icon, manifest)) = queue.pop_front() {
            url.set_fragment(None);
            if !same_origin(&origin, &url) {
                if let Some(host) = url.host_str() {
                    capture.hosts.insert(host.to_owned());
                }
                continue;
            }
            if !seen.insert(url.to_string()) {
                if manifest_icon
                    && let Some(resource) = capture.resources.iter_mut().find(|r| r.url == url)
                {
                    resource.manifest_icon = true;
                }
                continue;
            }
            if seen.len() > budget.max_resources {
                return Err(FrameworkError::bad_request(format!(
                    "page budget exceeds {} resources at {url}",
                    budget.max_resources
                )));
            }
            let mut resource = fetch(url.clone()).await?;
            if resource
                .body
                .as_ref()
                .is_some_and(|b| b.len() > budget.max_response_bytes)
            {
                return Err(FrameworkError::bad_request(format!(
                    "page budget response {url} exceeds {} bytes",
                    budget.max_response_bytes
                )));
            }
            resource.document = document;
            resource.manifest_icon = manifest_icon;
            resource.manifest = manifest;
            if (300..400).contains(&resource.status)
                && let Some(location) = resource.header("location")
            {
                let next = url.join(location).map_err(|e| {
                    FrameworkError::bad_request(format!("page budget redirect {url}: {e}"))
                })?;
                // The initial upgrade is collected so the original HAR's HTTP entry
                // and its HTTPS page remain one audit. It still fails first-redirect.
                if document
                    && url.scheme() == "http"
                    && next.scheme() == "https"
                    && next.host_str() == url.host_str()
                    && url.port().is_none()
                    && next.port().is_none()
                {
                    origin = next.clone();
                }
                if seen.contains(next.as_str()) {
                    return Err(FrameworkError::bad_request(format!(
                        "page budget redirect cycle at {next}"
                    )));
                }
                queue.push_back((next, document, manifest_icon, manifest));
            } else if (200..300).contains(&resource.status) {
                let refs = capture.references(&resource)?;
                for link in refs.links {
                    queue.push_back((link.url, false, link.manifest_icon, link.manifest));
                }
            }
            capture.resources.push(resource);
        }
        Ok(capture)
    };
    let capture = tokio::time::timeout(budget.collection_timeout, collect)
        .await
        .map_err(|_| {
            FrameworkError::bad_request(format!(
                "page budget collection exceeded {:?}",
                budget.collection_timeout
            ))
        })??;
    let budget = budget.clone();
    tokio::task::spawn_blocking(move || budget.evaluate(capture))
        .await
        .map_err(|e| FrameworkError::internal(format!("page budget evaluator failed: {e}")))?
}
