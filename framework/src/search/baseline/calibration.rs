//! REST calibration uses isolated resources, bounded requests and atomic floor publication.
use super::{BaselineFloors, Fixture, Measurement, Rival, RivalTrial, Rivals, measure, tree_hash};
use crate::FrameworkError;
use reqwest::{
    Method, Url,
    header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::time::Duration;

struct Service {
    name: &'static str,
    base: Url,
    client: reqwest::Client,
}
impl Service {
    fn new(
        name: &'static str,
        variable: &str,
        url: &str,
        key: &str,
        header: &'static str,
    ) -> Result<Self, FrameworkError> {
        let invalid = || {
            FrameworkError::param(format!(
                "{variable}: requires an HTTP(S) URL without credentials, query or fragment"
            ))
        };
        let mut base = Url::parse(url).map_err(|_| invalid())?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(invalid());
        }
        base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        if key.trim().is_empty() {
            return Err(FrameworkError::param(format!(
                "{}: requires {name} key",
                variable.replace("_URL", "_KEY")
            )));
        }
        let value = if header == AUTHORIZATION {
            format!("Bearer {key}")
        } else {
            key.into()
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| {
            FrameworkError::param(format!(
                "{}: invalid header value",
                variable.replace("_URL", "_KEY")
            ))
        })?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(header, value);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| {
                FrameworkError::internal(format!(
                    "search baseline {name}: cannot construct REST client"
                ))
            })?;
        Ok(Self { name, base, client })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        params: &[(&str, &str)],
        body: Option<String>,
        json: bool,
    ) -> Result<String, FrameworkError> {
        let fail = |reason: &str| {
            FrameworkError::internal(format!("search baseline {} {path}: {reason}", self.name))
        };
        let url = self
            .base
            .join(path)
            .map_err(|_| fail("invalid REST path"))?;
        let mut request = self.client.request(method, url).query(params);
        if let Some(body) = body {
            request = request
                .header(
                    CONTENT_TYPE,
                    if json {
                        "application/json"
                    } else {
                        "text/plain"
                    },
                )
                .body(body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| fail("request failed or timed out"))?;
        if !response.status().is_success() {
            return Err(fail(&format!("HTTP {}", response.status().as_u16())));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| fail("response body failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > 8_000_000 {
                return Err(fail("response exceeds 8 MB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| fail("response is not UTF-8"))
    }
    async fn json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, FrameworkError> {
        let body = self
            .request(method, path, &[], body.map(|v| v.to_string()), true)
            .await?;
        self.decode(path, &body)
    }
    fn decode(&self, path: &str, body: &str) -> Result<Value, FrameworkError> {
        serde_json::from_str(body).map_err(|_| {
            FrameworkError::internal(format!(
                "search baseline {} {path}: invalid JSON response",
                self.name
            ))
        })
    }
    async fn task(&self, task: Value) -> Result<(), FrameworkError> {
        let uid = task.get("taskUid").and_then(Value::as_u64).ok_or_else(|| {
            FrameworkError::internal("search baseline Meilisearch: missing taskUid")
        })?;
        let poll = async {
            loop {
                let task = self
                    .json(Method::GET, &format!("tasks/{uid}"), None)
                    .await?;
                match task.get("status").and_then(Value::as_str) {
                    Some("succeeded") => return Ok(()),
                    Some("enqueued" | "processing") => {
                        tokio::time::sleep(Duration::from_millis(50)).await
                    }
                    Some("failed" | "canceled") => {
                        return Err(FrameworkError::internal(format!(
                            "search baseline Meilisearch task {uid}: failed or canceled"
                        )));
                    }
                    _ => {
                        return Err(FrameworkError::internal(format!(
                            "search baseline Meilisearch task {uid}: invalid status"
                        )));
                    }
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(300), poll)
            .await
            .map_err(|_| {
                FrameworkError::internal(format!(
                    "search baseline Meilisearch task {uid}: 300 second deadline exceeded"
                ))
            })?
    }
    async fn meili_task(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(), FrameworkError> {
        self.task(self.json(method, path, body).await?).await
    }
}

/// Explicit service configuration permits calibration tests without global environment mutation.
pub struct Calibration {
    meilisearch: Service,
    typesense: Service,
}
impl Calibration {
    /// Validate all URLs and keys before sending either service any request.
    pub fn new(
        meilisearch_url: &str,
        meilisearch_key: &str,
        typesense_url: &str,
        typesense_key: &str,
    ) -> Result<Self, FrameworkError> {
        Ok(Self {
            meilisearch: Service::new(
                "Meilisearch",
                "SEARCH_BASELINE_MEILISEARCH_URL",
                meilisearch_url,
                meilisearch_key,
                "authorization",
            )?,
            typesense: Service::new(
                "Typesense",
                "SEARCH_BASELINE_TYPESENSE_URL",
                typesense_url,
                typesense_key,
                "x-typesense-api-key",
            )?,
        })
    }
    /// Missing service variables fail locally before any network operation begins.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let variable = crate::config::try_env_required::<String>;
        Self::new(
            &variable("SEARCH_BASELINE_MEILISEARCH_URL")?,
            &variable("SEARCH_BASELINE_MEILISEARCH_KEY")?,
            &variable("SEARCH_BASELINE_TYPESENSE_URL")?,
            &variable("SEARCH_BASELINE_TYPESENSE_KEY")?,
        )
    }
    /// Only successful measurement and cleanup of both rivals produces calibrated floors.
    pub async fn run(
        &self,
        fixture: &Fixture,
        previous: &BaselineFloors,
        tolerance: f64,
    ) -> Result<BaselineFloors, FrameworkError> {
        previous.validate()?;
        if !tolerance.is_finite() || !(0.0..=1.0).contains(&tolerance) {
            return Err(FrameworkError::param(
                "SEARCH_BASELINE_TOLERANCE: requires a finite number between 0 and 1",
            ));
        }
        let meilisearch = self.meilisearch(fixture).await?;
        let typesense = self.typesense(fixture).await?;
        let mut floors = previous.clone();
        floors.calibrated = true;
        floors.fixture_sha256.clone_from(&fixture.sha256);
        floors.fixture_license =
            "SciFact claims and judgments: CC BY 4.0; S2ORC abstracts: ODC-By 1.0; see LICENSE"
                .into();
        floors.tolerance = tolerance;
        floors.rivals = Rivals {
            meilisearch,
            typesense,
        };
        floors.calibrated_at = Some(crate::clock::now().to_rfc3339());
        let budget = floors.rival_budget()?;
        for value in floors.p95_budgets_ms.values_mut() {
            *value = budget;
        }
        let meili_rival = &floors.rivals.meilisearch;
        let typesense_rival = &floors.rivals.typesense;
        let meili = meili_rival.lexical.as_ref().ok_or_else(|| {
            FrameworkError::internal("search baseline Meilisearch: missing measurement")
        })?;
        let typesense = typesense_rival.lexical.as_ref().ok_or_else(|| {
            FrameworkError::internal("search baseline Typesense: missing measurement")
        })?;
        floors.budget_comment = format!(
            "Calibrated {}: Meilisearch {} settings {} p95 {:.6} ms; Typesense {} settings {} p95 {:.6} ms. Every engine budget is two times the slower rival's p95, measured over the same queries as nDCG@10 after a full warm-up pass.",
            floors.calibrated_at.as_deref().unwrap_or_default(),
            meili_rival.version.as_deref().unwrap_or_default(),
            meili_rival.settings.as_ref().ok_or_else(|| {
                FrameworkError::internal("search baseline Meilisearch: missing chosen settings")
            })?,
            meili.latency.p95_ms,
            typesense_rival.version.as_deref().unwrap_or_default(),
            typesense_rival.settings.as_ref().ok_or_else(|| {
                FrameworkError::internal("search baseline Typesense: missing chosen settings")
            })?,
            typesense.latency.p95_ms,
        );
        floors.tree_hash = tree_hash();
        floors.validate()?;
        Ok(floors)
    }
    async fn meilisearch(&self, fixture: &Fixture) -> Result<Rival, FrameworkError> {
        let service = &self.meilisearch;
        let uid = format!("suprnova_baseline_{}", uuid::Uuid::new_v4().simple());
        let path = format!("indexes/{uid}");
        let result = async {
            service
                .meili_task(
                    Method::POST,
                    "indexes",
                    Some(json!({"uid":uid,"primaryKey":"id"})),
                )
                .await?;
            service
                .meili_task(
                    Method::PATCH,
                    &format!("{path}/settings"),
                    Some(json!({"searchableAttributes":["title","text"]})),
                )
                .await?;
            for chunk in fixture.corpus.chunks(500) {
                service
                    .meili_task(
                        Method::POST,
                        &format!("{path}/documents"),
                        Some(json!(chunk)),
                    )
                    .await?;
            }
            let version = service.json(Method::GET, "version", None).await?;
            let version = version_string(&version, "pkgVersion", "Meilisearch")?;
            let mut trials = Vec::new();
            for strategy in ["last", "frequency", "all"] {
                let settings = json!({
                    "matchingStrategy":strategy, "limit":10, "attributesToRetrieve":["id"]
                });
                let lexical = measure(fixture, |q| {
                    let path = format!("{path}/search");
                    let mut body = settings.clone();
                    body["q"] = json!(q.text);
                    async move {
                        let result = service.json(Method::POST, &path, Some(body)).await?;
                        hit_ids(&result, false, "Meilisearch")
                    }
                })
                .await?;
                trials.push((settings, lexical));
            }
            select_rival(version, trials)
        }
        .await;
        let cleanup = service.meili_task(Method::DELETE, &path, None).await;
        finish(result, cleanup, "Meilisearch")
    }
    async fn typesense(&self, fixture: &Fixture) -> Result<Rival, FrameworkError> {
        let service = &self.typesense;
        let uid = format!("suprnova_baseline_{}", uuid::Uuid::new_v4().simple());
        let path = format!("collections/{uid}");
        let result = async {
            service.json(Method::POST, "collections", Some(json!({"name":uid,"fields":[{"name":"title","type":"string"},{"name":"text","type":"string"}]}))).await?;
            for chunk in fixture.corpus.chunks(500) {
                let body = chunk.iter().map(|d| serde_json::to_string(d).map_err(|_| FrameworkError::internal("search baseline Typesense: cannot encode document"))).collect::<Result<Vec<_>,_>>()?.join("\n") + "\n";
                let imported = service.request(Method::POST, &format!("{path}/documents/import"), &[("action","create")], Some(body), false).await?;
                let rows = imported.lines().collect::<Vec<_>>();
                if rows.len() != chunk.len() { return Err(FrameworkError::internal("search baseline Typesense import: result count differs from document count")); }
                for row in rows {
                    if service.decode("import", row)?.get("success").and_then(Value::as_bool) != Some(true) { return Err(FrameworkError::internal("search baseline Typesense import: document failed")); }
                }
            }
            let version = service.json(Method::GET, "debug", None).await?;
            let version = version_string(&version, "version", "Typesense")?;
            let mut trials = Vec::new();
            for threshold in [1, 10] {
                let threshold_string = threshold.to_string();
                let threshold_param = threshold_string.as_str();
                for match_type in ["max_score", "sum_score"] {
                    let settings = json!({
                        "drop_tokens_threshold":threshold, "text_match_type":match_type,
                        "query_by":"title,text", "per_page":10, "include_fields":"id"
                    });
                    let lexical = measure(fixture, |q| {
                        let path = format!("{path}/documents/search");
                        async move {
                            let body = service
                                .request(
                                    Method::GET,
                                    &path,
                                    &[
                                        ("q", &q.text),
                                        ("query_by", "title,text"),
                                        ("per_page", "10"),
                                        ("include_fields", "id"),
                                        ("drop_tokens_threshold", threshold_param),
                                        ("text_match_type", match_type),
                                    ],
                                    None,
                                    true,
                                )
                                .await
                                .map_err(|error| FrameworkError::internal(format!(
                                    "{error}; Typesense search parameters: drop_tokens_threshold={threshold}, text_match_type={match_type}"
                                )))?;
                            hit_ids(&service.decode(&path, &body)?, true, "Typesense")
                        }
                    })
                    .await?;
                    trials.push((settings, lexical));
                }
            }
            select_rival(version, trials)
        }.await;
        let cleanup = service.json(Method::DELETE, &path, None).await.map(|_| ());
        finish(result, cleanup, "Typesense")
    }
}
fn select_rival(
    version: String,
    trials: Vec<(Value, Measurement)>,
) -> Result<Rival, FrameworkError> {
    let (settings, lexical) = trials
        .iter()
        .max_by(|(_, left), (_, right)| {
            left.ndcg_at_10
                .total_cmp(&right.ndcg_at_10)
                .then_with(|| right.latency.p95_ms.total_cmp(&left.latency.p95_ms))
        })
        .ok_or_else(|| FrameworkError::internal("search baseline: empty rival calibration grid"))?;
    Ok(Rival {
        version: Some(version),
        lexical: Some(lexical.clone()),
        settings: Some(settings.clone()),
        grid: trials
            .into_iter()
            .map(|(settings, measurement)| RivalTrial {
                settings,
                ndcg_at_10: measurement.ndcg_at_10,
                p95_ms: measurement.latency.p95_ms,
            })
            .collect(),
    })
}
fn version_string(value: &Value, key: &str, service: &str) -> Result<String, FrameworkError> {
    match value.get(key) {
        Some(Value::String(version)) if !version.trim().is_empty() => Ok(version.clone()),
        _ => Err(FrameworkError::internal(format!(
            "search baseline {service}: version endpoint lacks {key}"
        ))),
    }
}
fn hit_ids(value: &Value, nested: bool, service: &str) -> Result<Vec<String>, FrameworkError> {
    value
        .get("hits")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            FrameworkError::internal(format!("search baseline {service}: missing hits array"))
        })?
        .iter()
        .map(|hit| {
            let hit = if nested { &hit["document"] } else { hit };
            hit.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "search baseline {service}: hit lacks string document id"
                    ))
                })
        })
        .collect()
}
fn finish<T>(
    result: Result<T, FrameworkError>,
    cleanup: Result<(), FrameworkError>,
    service: &str,
) -> Result<T, FrameworkError> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(FrameworkError::internal(format!(
            "{error}; {service} baseline cleanup: {cleanup}"
        ))),
    }
}
