use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
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
    async fn start(replies: Vec<Reply>) -> Self {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", socket.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(VecDeque::from(replies)));
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
                            requests.lock().unwrap().push(Request {
                                method: parts.method,
                                path: parts.uri.path().into(),
                                headers: parts.headers,
                                body: serde_json::from_slice(&bytes).unwrap(),
                            });
                            let reply = script.lock().unwrap().pop_front().unwrap_or(Reply {
                                status: 500,
                                body: json!({"success": false, "errors": [{"code": 1000, "message": "unexpected request"}]}),
                            });
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

    fn driver(&self) -> CloudflareDriver {
        driver().with_mock_endpoint(&self.endpoint).unwrap()
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn driver() -> CloudflareDriver {
    CloudflareDriver::new("zone123", "www.example.com", TOKEN, &S3Config::default())
        .unwrap()
        .with_manifest_ttl(Duration::ZERO)
        .unwrap()
}

#[tokio::test]
async fn cloudflare_urls_use_bearer_and_batches_of_one_hundred() {
    let mock = Mock::start(vec![Reply::ok(), Reply::ok(), Reply::ok()]).await;
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
        assert_eq!(request.path, "/client/v4/zones/zone123/purge_cache");
        assert_eq!(request.headers["authorization"], format!("Bearer {TOKEN}"));
        assert_eq!(request.headers["content-type"], "application/json");
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
    let mock = Mock::start(vec![Reply::ok(), Reply::ok()]).await;
    let driver = mock.driver();
    let capabilities = driver.capabilities();
    // The Worker sets no Cache-Tag header, so the publisher is steered to URLs;
    // a caller that sets the header itself can still purge by tag.
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
}

#[tokio::test]
async fn cloudflare_reports_envelope_and_http_errors_without_token() {
    for (status, success) in [(200, false), (403, false), (429, true)] {
        let mock = Mock::start(vec![Reply { status, body: json!({
            "success": success,
            "errors": [{"code": 1001, "message": format!("permission denied for {TOKEN}")}, {"code": 1002, "message": "second error"}]
        }) }]).await;
        let error = mock
            .driver()
            .purge(Purge::Urls(vec!["/about".into()]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("purge cache"), "{error}");
        assert!(error.contains(&status.to_string()), "{error}");
        assert!(
            error.contains("1001") && error.contains("permission denied"),
            "{error}"
        );
        assert!(!error.contains(TOKEN), "{error}");
        assert!(!error.contains("second error"), "{error}");
    }
    let mock = Mock::start(vec![Reply {
        status: 502,
        body: json!({"invalid": TOKEN}),
    }])
    .await;
    let error = mock
        .driver()
        .purge(Purge::Urls(vec!["/".into()]))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("502") && error.contains("envelope"));
    assert!(!error.contains(TOKEN));
}

#[test]
fn cloudflare_validates_headers_identifiers_roots_and_mock_endpoints() {
    let driver = driver();
    for (name, value) in [
        ("set-cookie", "x=1"),
        ("CF-Ray", "123"),
        ("cf-other", "x"),
        ("Via", "proxy"),
        ("bad header", "x"),
        ("x-public", "bad\r\nheader"),
    ] {
        assert!(
            driver
                .validate_headers(&BTreeMap::from([(name.into(), value.into())]))
                .is_err()
        );
    }
    driver
        .validate_headers(&BTreeMap::from([
            ("content-security-policy".into(), "script-src 'self'".into()),
            ("x-public".into(), "retained".into()),
        ]))
        .unwrap();
    for host in [
        "",
        "https://example.com",
        "example.com/path",
        "user@example.com",
        "example.com:443",
        "bad..host",
    ] {
        assert!(CloudflareDriver::new("zone123", host, TOKEN, &S3Config::default()).is_err());
    }
    assert!(CloudflareDriver::new("../zone", "example.com", TOKEN, &S3Config::default()).is_err());
    for root in [
        "pages/../outside",
        "pages//nested",
        "pages?query",
        "pages\\nested",
    ] {
        assert!(
            CloudflareDriver::new(
                "zone123",
                "example.com",
                TOKEN,
                &S3Config {
                    root: Some(root.into()),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    CloudflareDriver::new(
        "zone123",
        "example.com",
        TOKEN,
        &S3Config {
            root: Some("/pages/nested/".into()),
            ..Default::default()
        },
    )
    .unwrap();
    for token in ["", "bad\nsecret", "bad secret"] {
        let error = CloudflareDriver::new("zone123", "example.com", token, &S3Config::default())
            .err()
            .unwrap()
            .to_string();
        if !token.is_empty() {
            assert!(!error.contains(token));
        }
    }
    for endpoint in [
        "https://example.com/",
        "http://example.com/",
        "http://127.0.0.1/path",
        "http://user@127.0.0.1/",
        "http://127.0.0.1/?x=1",
    ] {
        assert!(
            super::cloudflare::driver()
                .with_mock_endpoint(endpoint)
                .is_err()
        );
    }
}

#[tokio::test]
async fn cloudflare_selection_and_rollback_need_no_provider_state() {
    let mock = Mock::start(vec![]).await;
    let driver = mock.driver();
    tokio::time::timeout(Duration::ZERO, driver.switch_version("v1"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(driver.selected_version().await.unwrap(), None);
    driver.rollback_version(Some("v0")).await.unwrap();
    driver.rollback_version(None).await.unwrap();
    driver.purge(Purge::Urls(vec![])).await.unwrap();
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn cloudflare_waits_for_manifest_ttl_before_switch_completion_and_purge() {
    let mock = Mock::start(vec![Reply::ok()]).await;
    let ttl = Duration::from_millis(20);
    let driver = mock.driver().with_manifest_ttl(ttl).unwrap();
    let start = std::time::Instant::now();
    driver.switch_version("v1").await.unwrap();
    assert!(start.elapsed() >= ttl);
    assert!(mock.requests().is_empty());
    let start = std::time::Instant::now();
    driver
        .purge(Purge::Urls(vec!["/about".into()]))
        .await
        .unwrap();
    assert!(start.elapsed() >= ttl);
    assert_eq!(mock.requests().len(), 1);
}

#[tokio::test]
async fn cloudflare_rejects_nonlocal_purges_before_any_request() {
    let mock = Mock::start(vec![]).await;
    for path in [
        "https://elsewhere.test/",
        "//elsewhere.test/",
        "/bad\npath",
        "/bad\\path",
        "/bad#fragment",
        "/wild/*",
    ] {
        assert!(
            mock.driver()
                .purge(Purge::Urls(vec!["/valid".into(), path.into()]))
                .await
                .is_err()
        );
    }
    assert!(
        mock.driver()
            .purge(Purge::Prefix("/path?query".into()))
            .await
            .is_err()
    );
    assert!(mock.requests().is_empty());
}

fn run_worker(script: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/static_publish/cdn/cloudflare_worker.js");
    let module = url::Url::from_file_path(path).unwrap();
    let script = format!(
        "import worker, {{keyFor, responseHeaders}} from {};\n{script}",
        serde_json::to_string(module.as_str()).unwrap()
    );
    let output = std::process::Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .output()
        .expect("Node runs the shipped Worker");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn cloudflare_worker_maps_keys_and_restores_delivery_headers() {
    let result = run_worker(
        r#"
const paths = ['/', '/about', '/about/', '/about/index.json', '/build/app-Ab12Cd34.js', '/_pub/v1/_assets/digest/app.js', '/_pub/manifest.json', '/_pub/status.json'];
const object = {httpEtag: '"r2-etag"', httpMetadata: {
  contentType: 'text/html; charset=utf-8', cacheControl: 'public, s-maxage=60',
  contentEncoding: 'gzip', contentLanguage: 'en', contentDisposition: 'inline',
  cacheExpiry: new Date('2030-01-01T00:00:00Z')
}, customMetadata: {'x-public': 'retained', 'content-security-policy': "script-src 'self'", 'content-type': 'wrong', 'content-length': '999', 'etag': 'wrong', 'content-encoding': 'identity'}};
const headers = Object.fromEntries(responseHeaders(object));
delete object.customMetadata['content-security-policy'];
console.log(JSON.stringify({keys: paths.map(path => keyFor(path, 'v1/revision', '')), prefix: keyFor('/about', 'v1', '/pages/'), headers, fallback: responseHeaders(object).get('content-security-policy'), jsonCsp: responseHeaders({httpEtag:'"x"', httpMetadata:{contentType:'application/json'}}).get('content-security-policy')}));
"#,
    );
    assert_eq!(
        result["keys"],
        json!([
            "_pub/v1/revision/index.html",
            "_pub/v1/revision/about/index.html",
            "_pub/v1/revision/about/index.html",
            "_pub/v1/revision/about/index.json",
            "build/app-Ab12Cd34.js",
            "_pub/v1/_assets/digest/app.js",
            null,
            null
        ])
    );
    assert_eq!(result["prefix"], "pages/_pub/v1/about/index.html");
    assert_eq!(
        result["headers"],
        json!({"content-type": "text/html; charset=utf-8", "cache-control": "public, s-maxage=60", "content-encoding": "gzip", "content-language": "en", "content-disposition": "inline", "expires": "Tue, 01 Jan 2030 00:00:00 GMT", "etag": "\"r2-etag\"", "x-public": "retained", "content-security-policy": "script-src 'self'"})
    );
    assert_eq!(result["fallback"], "default-src 'none'; sandbox");
    assert!(result["jsonCsp"].is_null());
}

#[test]
fn cloudflare_worker_falls_back_caches_manifest_and_handles_head_and_etags() {
    let result = run_worker(
        r#"
const assert = (await import('node:assert/strict')).default;
const reads = [];
let now = 1000;
Date.now = () => now;
let live = 'v1';
let broken = false;
let fallback = 0;
globalThis.fetch = async request => {fallback++; return new Response('application', {status: 202});};
const env = {ROOT_PREFIX: 'pages', MANIFEST_TTL_SECONDS: '5', BUCKET: {async get(key) {
  reads.push(key);
  if (key === 'pages/_pub/manifest.json') {
    if (broken) throw new Error('unavailable');
    return {json: async () => ({live})};
  }
  if (key.endsWith('/about/index.html')) return {body: 'published', httpEtag: '"etag"', httpMetadata: {contentType: 'text/html'}};
  return null;
}}};
const call = (path, options) => worker.fetch(new Request('https://www.example.com' + path, options), env);
assert.equal((await call('/about', {method:'POST'})).status, 202);
assert.equal((await call('/_pub/manifest.json')).status, 202);
assert.equal((await call('/_pub/status.json')).status, 202);
assert.equal(reads.length, 0);
assert.equal(await (await call('/about')).text(), 'published');
const head = await call('/about', {method:'HEAD'});
assert.equal(await head.text(), '');
assert.equal(head.headers.get('etag'), '"etag"');
for (const tag of ['"etag"', '"other", W/"etag"', '*']) {
  const response = await call('/about', {headers:{'If-None-Match':tag}});
  assert.equal(response.status, 304);
  assert.equal(await response.text(), '');
}
assert.equal((await call('/missing')).status, 202);
assert.equal(reads.filter(key => key.endsWith('manifest.json')).length, 1);
live = 'v2';
now = 6000;
await call('/about');
assert.equal(reads.at(-1), 'pages/_pub/v2/about/index.html');
broken = true;
now = 11000;
assert.equal((await call('/about')).status, 202);
for (const value of [null, {}, {live:''}, {live:'../outside'}]) {
  const missing = {BUCKET: {get: async () => value === null ? null : {json: async () => value}}, MANIFEST_TTL_SECONDS: '0'};
  assert.equal((await worker.fetch(new Request('https://www.example.com/about'), missing)).status, 202);
}
console.log(JSON.stringify({fallback, manifestReads: reads.filter(key => key.endsWith('manifest.json')).length}));
"#,
    );
    assert_eq!(result, json!({"fallback": 9, "manifestReads": 3}));
}

#[test]
fn cloudflare_worker_edge_cache_tracks_object_keys_and_never_caches_the_application() {
    let result = run_worker(
        r#"
const assert = (await import('node:assert/strict')).default;
const stored = new Map();
const writes = [];
globalThis.caches = {default: {
  async match(key) {return stored.get(key.url)?.clone();},
  async put(key, response) {writes.push(key.url); stored.set(key.url, response);}
}};
let live = 'v1';
let objectReads = 0;
globalThis.fetch = async () => new Response('application', {status:202});
const env = {MANIFEST_TTL_SECONDS:'0', BUCKET:{async get(key) {
  if (key === '_pub/manifest.json') return {json: async () => ({live, versions:{[live]:{objects:[{url:'/about', path:`_pub/${live}/about.html`, dependency_tags:['dependency-digest']}]}}})};
  objectReads++;
  if (key === `_pub/${live}/about.html`) return {body:live, httpEtag:`"${live}"`, httpMetadata:{contentType:'text/html',cacheControl:'public, s-maxage=60'}};
  return null;
}}};
const call = (path, options) => worker.fetch(new Request('https://www.example.com' + path, options), env);
const first = await call('/about');
assert.equal(await first.text(), 'v1');
assert.equal(first.headers.has('x-suprnova-r2-object-key'), false);
assert.equal(stored.get('https://www.example.com/about').headers.get('cache-tag'), 'dependency-digest');
assert.equal(await (await call('/about?ignored=one')).text(), 'v1');
const head = await call('/about', {method:'HEAD'});
assert.equal(head.status, 200);
assert.equal(await head.text(), '');
assert.equal(head.headers.has('x-suprnova-r2-object-key'), false);
assert.equal((await call('/about', {headers:{'If-None-Match':'"v1"'}})).status, 304);
assert.equal(objectReads, 2);
live = 'v2';
assert.equal(await (await call('/about')).text(), 'v2');
assert.equal(objectReads, 3);
assert.equal((await call('/missing')).status, 202);
assert.equal(stored.has('https://www.example.com/missing'), false);
assert.deepEqual(writes, ['https://www.example.com/about', 'https://www.example.com/about']);
console.log(JSON.stringify({objectReads, writes:writes.length}));
"#,
    );
    assert_eq!(result, json!({"objectReads": 4, "writes": 2}));
}

#[tokio::test]
async fn cloudflare_publication_purges_page_urls_and_worker_serves_the_inventory() {
    let mock = Mock::start(vec![Reply::ok()]).await;
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
    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body,
        json!({"files": ["https://www.example.com/about", "https://www.example.com/about/index.json"]})
    );
    let disk = Storage::disk(&name).unwrap();
    let mut objects = BTreeMap::new();
    for object in &manifest.versions[&manifest.live].objects {
        objects.insert(
            object.path.clone(),
            String::from_utf8(disk.read(&object.path).await.unwrap().to_vec()).unwrap(),
        );
    }
    let script = format!(
        r#"
const manifest = {manifest};
const objects = {objects};
globalThis.fetch = async () => new Response('application', {{status:202}});
const env = {{MANIFEST_TTL_SECONDS:'0', BUCKET: {{async get(key) {{
  if (key === '_pub/manifest.json') return {{json: async () => manifest}};
  if (objects[key] === undefined) return null;
  return {{body: objects[key], httpEtag:'"object"', httpMetadata:{{}}}};
}}}}}};
const html = await worker.fetch(new Request('https://www.example.com/about/'), env);
const json = await worker.fetch(new Request('https://www.example.com/about/index.json'), env);
console.log(JSON.stringify({{html: await html.text(), json: await json.json()}}));
"#,
        manifest = serde_json::to_string(&manifest).unwrap(),
        objects = serde_json::to_string(&objects).unwrap()
    );
    let result = run_worker(&script);
    assert_eq!(result["html"], "<html>published</html>");
    assert_eq!(result["json"]["component"], "About");
}
