//! The Cloudflare driver: rules on the zone for an R2 custom domain, and purges.

use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::{
    S3Config, Storage,
    static_publish::{
        PageObject, PublishConfig, Publisher,
        cdn::{CdnDriver, CloudflareDriver, Purge},
    },
};

const TOKEN: &str = "cloudflare-test-token";
const HOST: &str = "www.example.com";

struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    fn ok() -> Self {
        Self {
            status: 200,
            body: json!({"success": true, "errors": []}),
        }
    }
    fn not_found() -> Self {
        Self {
            status: 404,
            body: json!({"success": false, "errors": [{"code": 10000, "message": "not found"}]}),
        }
    }
    fn entrypoint(id: &str, rules: Vec<Value>) -> Self {
        Self {
            status: 200,
            body: json!({"success": true, "errors": [], "result": {"id": id, "rules": rules}}),
        }
    }
}

#[derive(Clone)]
struct Request {
    method: http::Method,
    path: String,
    headers: http::HeaderMap,
    body: Value,
}

struct Mock {
    endpoint: String,
    requests: Arc<Mutex<Vec<Request>>>,
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl Mock {
    /// Answer from `replies` in order; `None` answers every GET with 404 and the
    /// rest with success, a zone that has no rules yet.
    async fn start(replies: Option<Vec<Reply>>) -> Self {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", socket.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(replies.map(VecDeque::from)));
        let shared = requests.clone();
        let listener = tokio::spawn(async move {
            while let Ok((stream, _)) = socket.accept().await {
                let requests = shared.clone();
                let script = script.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |incoming: hyper::Request<Incoming>| {
                        let requests = requests.clone();
                        let script = script.clone();
                        async move {
                            let (parts, body) = incoming.into_parts();
                            let bytes = body.collect().await.unwrap().to_bytes();
                            let method = parts.method.clone();
                            requests.lock().unwrap().push(Request {
                                method: parts.method,
                                path: parts.uri.path().into(),
                                headers: parts.headers,
                                body: if bytes.is_empty() {
                                    Value::Null
                                } else {
                                    serde_json::from_slice(&bytes).unwrap()
                                },
                            });
                            let reply = match script.lock().unwrap().as_mut() {
                                Some(queue) => queue.pop_front().unwrap_or(Reply {
                                    status: 500,
                                    body: json!({"success": false, "errors": [{"code": 1000, "message": "unexpected request"}]}),
                                }),
                                None if method == http::Method::GET => Reply::not_found(),
                                None => Reply::ok(),
                            };
                            Ok::<_, Infallible>(
                                http::Response::builder()
                                    .status(reply.status)
                                    .header("content-type", "application/json")
                                    .body(Full::new(Bytes::from(reply.body.to_string())))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            endpoint,
            requests,
            listener,
        }
    }

    async fn bare() -> Self {
        Self::start(None).await
    }

    fn driver(&self) -> CloudflareDriver {
        driver(&S3Config::default())
            .with_mock_endpoint(&self.endpoint)
            .unwrap()
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.requests()
            .into_iter()
            .map(|r| (r.method.to_string(), r.path))
            .collect()
    }
}

fn driver(config: &S3Config) -> CloudflareDriver {
    CloudflareDriver::new("zone123", HOST, TOKEN, config).unwrap()
}

fn zone(path: &str) -> String {
    format!("/client/v4/zones/zone123/{path}")
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

const CSP: &str = "default-src 'none'; frame-ancestors 'none'";

/// The rules a bare zone gets on the first switch for `HOST` with no disk root.
fn index_slash_rule() -> Value {
    json!({
        "ref": "suprnova-static-publication-index-slash",
        "description": "Suprnova static publication: a path ending in / serves its index.html",
        "expression": "(http.host eq \"www.example.com\" and ends_with(http.request.uri.path, \"/\"))",
        "action": "rewrite",
        "action_parameters": {"uri": {"path": {"expression": "concat(\"\", http.request.uri.path, \"index.html\")"}}},
    })
}

fn index_rule() -> Value {
    json!({
        "ref": "suprnova-static-publication-index",
        "description": "Suprnova static publication: a page path serves its index.html",
        "expression": "(http.host eq \"www.example.com\" and not ends_with(http.request.uri.path, \"/\") and not http.request.uri.path contains \".\" and not starts_with(http.request.uri.path, \"/_pub/\"))",
        "action": "rewrite",
        "action_parameters": {"uri": {"path": {"expression": "concat(\"\", http.request.uri.path, \"/index.html\")"}}},
    })
}

fn headers_rule() -> Value {
    json!({
        "ref": "suprnova-static-publication-headers",
        "description": "Suprnova static publication: the published response headers",
        "expression": "(http.host eq \"www.example.com\")",
        "action": "rewrite",
        "action_parameters": {"headers": {"content-security-policy": {"operation": "set", "value": CSP}}},
    })
}

fn cache_rule() -> Value {
    json!({
        "ref": "suprnova-static-publication-cache",
        "description": "Suprnova static publication: responses that say they are cacheable are cached for the time they say",
        "expression": "(http.host eq \"www.example.com\")",
        "action": "set_cache_settings",
        "action_parameters": {
            "cache": true,
            "edge_ttl": {"mode": "bypass_by_default"},
            "browser_ttl": {"mode": "respect_origin"},
        },
    })
}

/// An existing rule as the API returns it: the driver's fields plus an id.
fn stored(id: &str, rule: Value) -> Value {
    let mut rule = rule;
    rule["id"] = json!(id);
    rule["version"] = json!("1");
    rule
}

#[tokio::test]
async fn cloudflare_validates_zone_host_token_root_and_mock_endpoint() {
    for host in ["", "-bad.example.com", "www.example.com/path", "a..b"] {
        assert!(
            CloudflareDriver::new("zone123", host, TOKEN, &S3Config::default()).is_err(),
            "{host}"
        );
    }
    assert!(CloudflareDriver::new("../zone", HOST, TOKEN, &S3Config::default()).is_err());
    assert!(CloudflareDriver::new("zone123", HOST, "", &S3Config::default()).is_err());
    assert!(CloudflareDriver::new("zone123", HOST, "with space", &S3Config::default()).is_err());
    let bad_root = S3Config {
        root: Some("a/../b".into()),
        ..S3Config::default()
    };
    assert!(CloudflareDriver::new("zone123", HOST, TOKEN, &bad_root).is_err());
    for endpoint in [
        "https://127.0.0.1:1/",
        "http://example.com/",
        "http://127.0.0.1:1/path",
        "http://user:pw@127.0.0.1:1/",
    ] {
        assert!(
            driver(&S3Config::default())
                .with_mock_endpoint(endpoint)
                .is_err(),
            "{endpoint}"
        );
    }
}

#[tokio::test]
async fn cloudflare_urls_use_bearer_and_batches_of_one_hundred() {
    let mock = Mock::bare().await;
    mock.driver()
        .purge(Purge::Urls(
            (0..230).map(|i| format!("/page/{i}")).collect(),
        ))
        .await
        .unwrap();
    let requests = mock.requests();
    assert_eq!(requests.len(), 3);
    let mut files = Vec::new();
    for (request, size) in requests.iter().zip([100, 100, 30]) {
        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.path, zone("purge_cache"));
        assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
        let batch = request.body["files"].as_array().unwrap();
        assert_eq!(batch.len(), size);
        files.extend(batch.iter().cloned());
    }
    assert_eq!(
        files,
        (0..230)
            .map(|i| json!(format!("https://www.example.com/page/{i}")))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn cloudflare_purges_by_prefix_and_by_tag_when_asked() {
    let mock = Mock::bare().await;
    let driver = mock.driver();
    let capabilities = driver.capabilities();
    // No Cache-Tag header is published, so the publisher is steered to URLs; a
    // caller that sets the header itself can still purge by tag.
    assert!(capabilities.urls && capabilities.prefix && !capabilities.tags);
    driver.purge(Purge::Prefix("/blog/".into())).await.unwrap();
    driver
        .purge(Purge::Tags(vec!["posts".into(), "authors".into()]))
        .await
        .unwrap();
    let requests = mock.requests();
    assert_eq!(
        requests[0].body,
        json!({"prefixes": ["www.example.com/blog/"]})
    );
    assert_eq!(requests[1].body, json!({"tags": ["posts", "authors"]}));
    for path in [
        "relative",
        "//host/x",
        "/with#fragment",
        "/wild*",
        "/ctl\u{1}",
    ] {
        assert!(
            mock.driver()
                .purge(Purge::Urls(vec!["/valid".into(), path.into()]))
                .await
                .is_err(),
            "{path}"
        );
    }
    assert!(
        mock.driver()
            .purge(Purge::Prefix("/path?query".into()))
            .await
            .is_err()
    );
    assert!(
        mock.driver()
            .purge(Purge::Tags(vec!["".into()]))
            .await
            .is_err()
    );
    assert_eq!(mock.requests().len(), 2, "refused purges send nothing");
}

#[tokio::test]
async fn cloudflare_reports_envelope_and_http_errors_without_token() {
    let leak = format!("token {TOKEN} rejected");
    let mock = Mock::start(Some(vec![
        Reply {
            status: 200,
            body: json!({"success": false, "errors": [{"code": 10000, "message": leak}]}),
        },
        Reply {
            status: 403,
            body: json!({"success": false, "errors": [{"code": 9109, "message": "forbidden"}]}),
        },
        Reply {
            status: 200,
            body: json!("not an envelope"),
        },
    ]))
    .await;
    let driver = mock.driver();
    let error = driver
        .purge(Purge::Urls(vec!["/a".into()]))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("code 10000"), "{error}");
    assert!(!error.contains(TOKEN), "{error}");
    assert!(error.contains("[redacted]"), "{error}");
    let error = driver
        .purge(Purge::Urls(vec!["/a".into()]))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("HTTP 403") && error.contains("code 9109"),
        "{error}"
    );
    let error = driver
        .purge(Purge::Urls(vec!["/a".into()]))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unreadable Cloudflare envelope"), "{error}");
}

#[tokio::test]
async fn cloudflare_validates_headers_for_the_response_header_rule() {
    let driver = driver(&S3Config::default());
    for name in [
        "set-cookie",
        "cf-ray",
        "cf-cache-status",
        "host",
        "transfer-encoding",
    ] {
        assert!(
            driver.validate_headers(&headers(&[(name, "x")])).is_err(),
            "{name}"
        );
    }
    assert!(
        driver
            .validate_headers(&headers(&[("bad header", "x")]))
            .is_err()
    );
    assert!(
        driver
            .validate_headers(&headers(&[("x-frame-options", "bad\nvalue")]))
            .is_err()
    );
    driver
        .validate_headers(&headers(&[
            ("Content-Security-Policy", CSP),
            ("cache-control", "public, s-maxage=60"),
        ]))
        .unwrap();
}

#[tokio::test]
async fn cloudflare_first_switch_creates_the_rules_on_a_bare_zone() {
    let mock = Mock::bare().await;
    let driver = mock.driver();
    driver
        .validate_headers(&headers(&[
            ("content-security-policy", CSP),
            ("content-type", "text/html; charset=utf-8"),
            ("cache-control", "public, s-maxage=60"),
        ]))
        .unwrap();
    assert_eq!(driver.selected_version().await.unwrap(), None);
    driver.switch_version("v1").await.unwrap();
    let requests = mock.requests();
    assert_eq!(
        mock.calls(),
        vec![
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_transform/entrypoint")
            ),
            (
                "PUT".to_string(),
                zone("rulesets/phases/http_request_transform/entrypoint")
            ),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_response_headers_transform/entrypoint")
            ),
            (
                "PUT".to_string(),
                zone("rulesets/phases/http_response_headers_transform/entrypoint")
            ),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
            (
                "PUT".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
        ]
    );
    assert_eq!(
        requests[1].headers["authorization"],
        format!("Bearer {TOKEN}")
    );
    assert_eq!(
        requests[1].body,
        json!({"rules": [index_slash_rule(), index_rule()]})
    );
    // Headers R2 serves from the object itself are not repeated by the rule.
    assert_eq!(requests[3].body, json!({"rules": [headers_rule()]}));
    assert_eq!(requests[5].body, json!({"rules": [cache_rule()]}));
}

#[tokio::test]
async fn cloudflare_root_prefix_rewrites_every_path_below_the_disk_root() {
    let mock = Mock::bare().await;
    let config = S3Config {
        root: Some("pages".into()),
        ..S3Config::default()
    };
    let driver = driver(&config).with_mock_endpoint(&mock.endpoint).unwrap();
    driver.switch_version("v1").await.unwrap();
    let requests = mock.requests();
    let rules = requests[1].body["rules"].as_array().unwrap().clone();
    assert_eq!(rules.len(), 3);
    assert_eq!(
        rules[0]["action_parameters"]["uri"]["path"]["expression"],
        "concat(\"/pages\", http.request.uri.path, \"index.html\")"
    );
    assert_eq!(
        rules[1]["action_parameters"]["uri"]["path"]["expression"],
        "concat(\"/pages\", http.request.uri.path, \"/index.html\")"
    );
    assert_eq!(rules[2]["ref"], "suprnova-static-publication-objects");
    assert_eq!(
        rules[2]["expression"],
        "(http.host eq \"www.example.com\" and (http.request.uri.path contains \".\" or starts_with(http.request.uri.path, \"/_pub/\")))"
    );
    assert_eq!(
        rules[2]["action_parameters"]["uri"]["path"]["expression"],
        "concat(\"/pages\", http.request.uri.path)"
    );
    // No validated headers: no header rule, no call for its phase.
    assert_eq!(mock.calls().len(), 4);
}

#[tokio::test]
async fn cloudflare_later_switch_corrects_and_adds_only_its_own_rules() {
    let mut stale_index = index_rule();
    stale_index["expression"] = json!("(http.host eq \"old.example.com\")");
    let foreign = json!({
        "id": "r-foreign", "ref": "app-redirects", "expression": "(http.request.uri.path eq \"/old\")",
        "action": "rewrite", "action_parameters": {"uri": {"path": {"value": "/new"}}},
    });
    let mock = Mock::start(Some(vec![
        Reply::entrypoint(
            "rs-transform",
            vec![
                foreign.clone(),
                stored("r-slash", index_slash_rule()),
                stored("r-index", stale_index),
            ],
        ),
        Reply::ok(),
        Reply::entrypoint("rs-headers", vec![foreign.clone()]),
        Reply::ok(),
        Reply::entrypoint("rs-cache", vec![stored("r-cache", cache_rule())]),
    ]))
    .await;
    let driver = mock.driver();
    driver
        .validate_headers(&headers(&[("content-security-policy", CSP)]))
        .unwrap();
    driver.switch_version("v2").await.unwrap();
    assert_eq!(
        mock.calls(),
        vec![
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_transform/entrypoint")
            ),
            (
                "PATCH".to_string(),
                zone("rulesets/rs-transform/rules/r-index")
            ),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_response_headers_transform/entrypoint")
            ),
            ("POST".to_string(), zone("rulesets/rs-headers/rules")),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
        ]
    );
    let requests = mock.requests();
    assert_eq!(requests[1].body, index_rule());
    assert_eq!(requests[3].body, headers_rule());
}

#[tokio::test]
async fn cloudflare_for_origin_keeps_only_the_cache_rule() {
    let mock = Mock::bare().await;
    let driver = CloudflareDriver::for_origin("zone123", HOST, TOKEN)
        .unwrap()
        .with_mock_endpoint(&mock.endpoint)
        .unwrap();
    driver
        .validate_headers(&headers(&[("content-security-policy", CSP)]))
        .unwrap();
    driver.switch_version("v1").await.unwrap();
    assert_eq!(
        mock.calls(),
        vec![
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
            (
                "PUT".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
        ]
    );
    assert_eq!(mock.requests()[1].body, json!({"rules": [cache_rule()]}));
}

#[tokio::test]
async fn cloudflare_rollback_of_a_first_publication_deletes_only_its_rules() {
    let foreign = json!({"id": "r-foreign", "ref": "app-redirects", "expression": "true", "action": "rewrite"});
    let mock = Mock::start(Some(vec![
        Reply::entrypoint(
            "rs-transform",
            vec![
                foreign.clone(),
                stored("r-slash", index_slash_rule()),
                stored("r-index", index_rule()),
            ],
        ),
        Reply::ok(),
        Reply::ok(),
        Reply::not_found(),
        Reply::entrypoint("rs-cache", vec![stored("r-cache", cache_rule()), foreign]),
        Reply::ok(),
    ]))
    .await;
    let driver = mock.driver();
    driver.rollback_version(Some("v1")).await.unwrap();
    assert!(mock.calls().is_empty(), "a later rollback touches nothing");
    driver.rollback_version(None).await.unwrap();
    assert_eq!(
        mock.calls(),
        vec![
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_transform/entrypoint")
            ),
            (
                "DELETE".to_string(),
                zone("rulesets/rs-transform/rules/r-slash")
            ),
            (
                "DELETE".to_string(),
                zone("rulesets/rs-transform/rules/r-index")
            ),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_response_headers_transform/entrypoint")
            ),
            (
                "GET".to_string(),
                zone("rulesets/phases/http_request_cache_settings/entrypoint")
            ),
            (
                "DELETE".to_string(),
                zone("rulesets/rs-cache/rules/r-cache")
            ),
        ]
    );
}

#[tokio::test]
async fn cloudflare_publication_creates_the_rules_and_purges_the_page_urls() {
    let mock = Mock::bare().await;
    let name = format!("cloudflare-publication-{}", uuid::Uuid::new_v4());
    Storage::register_memory(&name);
    let publisher = Publisher::new(&name, "v1", PublishConfig::default())
        .unwrap()
        .with_cdn(Arc::new(mock.driver()));
    let page = PageObject::new(
        "/about",
        Bytes::from_static(b"<html>published</html>"),
        Bytes::from_static(br#"{"component":"About","props":{},"url":"/about","version":"v1"}"#),
    )
    .unwrap();
    let outcome = publisher.publish(vec![page]).await.unwrap();
    let manifest = publisher.manifest().await.unwrap();
    assert_eq!(manifest.live, outcome.version);
    assert!(outcome.prune.error.is_none(), "{:?}", outcome.prune.error);
    let calls = mock.calls();
    assert!(
        calls.contains(&(
            "PUT".to_string(),
            zone("rulesets/phases/http_request_transform/entrypoint")
        )),
        "{calls:?}"
    );
    assert!(
        calls.contains(&(
            "PUT".to_string(),
            zone("rulesets/phases/http_request_cache_settings/entrypoint")
        )),
        "{calls:?}"
    );
    // The custom domain serves the stable mirror, so the publication must have
    // written it, and must have done so before the purge.
    let disk = Storage::disk(&name).unwrap();
    assert_eq!(
        disk.read("about/index.html")
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"<html>published</html>"
    );
    assert!(disk.read("about/index.json").await.is_ok());
    assert!(mock.driver().serves_mirror());
    assert!(
        !CloudflareDriver::for_origin("zone123", HOST, TOKEN)
            .unwrap()
            .serves_mirror(),
        "the application's origin serves the versioned objects itself"
    );
    let purge = mock
        .requests()
        .into_iter()
        .filter(|r| r.path == zone("purge_cache"))
        .collect::<Vec<_>>();
    assert_eq!(purge.len(), 1, "{calls:?}");
    assert_eq!(
        purge[0].body,
        json!({"files": ["https://www.example.com/about", "https://www.example.com/about/index.json"]})
    );
    assert!(
        calls
            .iter()
            .position(|c| c.1 == zone("purge_cache"))
            .unwrap()
            > calls.iter().rposition(|c| c.0 == "PUT").unwrap(),
        "the purge follows the switch: {calls:?}"
    );
}

#[tokio::test]
async fn cloudflare_rejects_oversized_api_envelope() {
    let mock = Mock::start(Some(vec![Reply {
        status: 200,
        body: json!({"success":true,"errors":[],"padding":"x".repeat(2*1024*1024)}),
    }]))
    .await;
    let error = mock
        .driver()
        .purge(Purge::Prefix("/".into()))
        .await
        .map(|_| ())
        .expect_err("oversized envelope must fail");
    assert!(error.to_string().contains("byte cap"), "{error}");
}
