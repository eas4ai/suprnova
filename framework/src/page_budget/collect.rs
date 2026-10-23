//! Fetch a finite asset graph with one collector for both request paths.

use super::{PageAudit, PageBudget, Resource, discover, same_origin};
use crate::{FrameworkError, http_client::Http, testing::TestClient};
use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use url::Url;

pub(super) struct Capture {
    pub page: Url,
    pub inertia: Option<crate::InertiaConfig>,
    pub resources: Vec<Resource>,
    pub hosts: BTreeSet<String>,
    pub alternatives: BTreeSet<String>,
    pub image_groups: Vec<Vec<String>>,
    reference_urls: BTreeSet<String>,
    work_limit: usize,
    work_used: usize,
}

impl Capture {
    pub(super) fn new(page: Url, work_limit: usize) -> Self {
        Self {
            reference_urls: BTreeSet::from([page.to_string()]),
            work_limit,
            work_used: 0,
            page,
            inertia: crate::App::inertia_registry().installed_config(),
            resources: Vec::new(),
            hosts: BTreeSet::new(),
            alternatives: BTreeSet::new(),
            image_groups: Vec::new(),
        }
    }

    pub(super) fn references(
        &mut self,
        resource: &Resource,
        maximum: usize,
    ) -> Result<discover::References, FrameworkError> {
        let Some(bytes) = &resource.body else {
            return Ok(discover::References::default());
        };
        let limits = discover::Limits {
            resources: maximum,
            tokens: self.work_limit,
            used_tokens: self.work_used,
            known: &self.reference_urls,
        };
        let mut refs = if resource.document && resource.mime().eq_ignore_ascii_case("text/html") {
            discover::html(&resource.url, &String::from_utf8_lossy(bytes), limits)?
        } else if resource.mime().eq_ignore_ascii_case("text/css")
            || resource.url.path().ends_with(".css")
        {
            discover::css(&resource.url, &String::from_utf8_lossy(bytes), limits)?
        } else if resource.manifest {
            discover::manifest(&resource.url, bytes, limits)?
        } else {
            discover::References::default()
        };
        self.work_used = self.work_used.saturating_add(refs.work_done);
        self.register_references(&mut refs, maximum)?;
        Ok(refs)
    }

    // Track the complete graph before any of its references enter the queue.
    fn register_references(
        &mut self,
        refs: &mut discover::References,
        maximum: usize,
    ) -> Result<(), FrameworkError> {
        for link in &refs.links {
            if !self.reference_urls.contains(link.url.as_str()) {
                if self.reference_urls.len() >= maximum {
                    return Err(FrameworkError::bad_request(format!(
                        "page budget discovery exceeds {maximum} resources at {}",
                        link.url
                    )));
                }
                self.reference_urls.insert(link.url.to_string());
            }
        }
        self.alternatives.append(&mut refs.alternatives);
        for group in refs.image_groups.drain(..) {
            if !self.image_groups.contains(&group) {
                self.image_groups.push(group);
            }
        }
        Ok(())
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
            .flat_map(|name| {
                response
                    .header_values(name)
                    .into_iter()
                    .map(move |value| (name.into(), value))
            })
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
        let mut capture = Capture::new(page.clone(), budget.max_discovery_tokens);
        let mut origin = page.clone();
        if budget.max_resources == 0 {
            return Err(FrameworkError::bad_request(
                "page budget resource limit is zero",
            ));
        }
        let mut scheduled = BTreeSet::from([page.to_string()]);
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
                enqueue(
                    &mut queue,
                    &mut scheduled,
                    &mut capture,
                    (next, document, manifest_icon, manifest),
                    budget.max_resources,
                )?;
            } else if (200..300).contains(&resource.status) {
                let refs = capture.references(&resource, budget.max_resources)?;
                for link in refs.links {
                    enqueue(
                        &mut queue,
                        &mut scheduled,
                        &mut capture,
                        (link.url, false, link.manifest_icon, link.manifest),
                        budget.max_resources,
                    )?;
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

// Deduplicate before queueing, including off-origin references, to bound pending work.
fn enqueue(
    queue: &mut VecDeque<(Url, bool, bool, bool)>,
    scheduled: &mut BTreeSet<String>,
    capture: &mut Capture,
    entry: (Url, bool, bool, bool),
    maximum: usize,
) -> Result<(), FrameworkError> {
    if scheduled.contains(entry.0.as_str()) {
        if let Some(pending) = queue.iter_mut().find(|item| item.0 == entry.0) {
            pending.1 |= entry.1;
            pending.2 |= entry.2;
            pending.3 |= entry.3;
        }
        if let Some(resource) = capture
            .resources
            .iter_mut()
            .find(|resource| resource.url == entry.0)
        {
            resource.manifest_icon |= entry.2;
            resource.manifest |= entry.3;
        }
        return Ok(());
    }
    if scheduled.len() >= maximum {
        return Err(FrameworkError::bad_request(format!(
            "page budget exceeds {maximum} resources at {}",
            entry.0
        )));
    }
    scheduled.insert(entry.0.to_string());
    queue.push_back(entry);
    Ok(())
}
