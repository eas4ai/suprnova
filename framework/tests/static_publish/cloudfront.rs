use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use suprnova::{
    S3Config,
    static_publish::cdn::{CdnDriver, CloudFrontDriver, Purge},
};

const SECRET: &str = "cloudfront-test-secret";
const FUNCTION: &str = "arn:aws:cloudfront::123456789012:function/suprnova-public-headers";
const BEHAVIOR: &str = r#"<DefaultCacheBehavior><TargetOriginId>public-pages</TargetOriginId><FunctionAssociations><Quantity>1</Quantity><Items><FunctionAssociation><FunctionARN>arn:aws:cloudfront::123456789012:function/suprnova-public-headers</FunctionARN><EventType>viewer-response</EventType></FunctionAssociation></Items></FunctionAssociations></DefaultCacheBehavior>"#;
const CONFIG: &str = r#"<?xml version="1.0"?><DistributionConfig xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/"><CallerReference>keep-this</CallerReference><Origins><Quantity>2</Quantity><Items><Origin><Id>public-pages</Id><DomainName>bucket.s3.amazonaws.com</DomainName><OriginPath></OriginPath><UnknownOriginField value="a&amp;b"/></Origin><Origin><Id>private-origin</Id><OriginPath>/private</OriginPath></Origin></Items></Origins><DefaultCacheBehavior><TargetOriginId>public-pages</TargetOriginId><FunctionAssociations><Quantity>1</Quantity><Items><FunctionAssociation><FunctionARN>arn:aws:cloudfront::123456789012:function/suprnova-public-headers</FunctionARN><EventType>viewer-response</EventType></FunctionAssociation></Items></FunctionAssociations></DefaultCacheBehavior><UnknownFutureField><Nested>keep &amp; preserve</Nested></UnknownFutureField><Enabled>true</Enabled></DistributionConfig>"#;

struct Reply {
    status: u16,
    body: String,
    etag: Option<&'static str>,
}
impl Reply {
    fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            etag: None,
        }
    }
    fn config(body: impl Into<String>) -> Self {
        Self {
            etag: Some("etag-one"),
            ..Self::ok(body)
        }
    }
}
#[derive(Clone)]
struct Request {
    method: String,
    path: String,
    headers: http::HeaderMap,
    body: String,
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
                let script = script.clone();
                let requests = shared.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |incoming: hyper::Request<Incoming>| {
                        let script = script.clone();
                        let requests = requests.clone();
                        async move {
                            let (parts, body) = incoming.into_parts();
                            let bytes = body.collect().await.unwrap().to_bytes();
                            let request = Request {
                                method: parts.method.to_string(),
                                path: parts.uri.path().to_owned(),
                                headers: parts.headers,
                                body: String::from_utf8(bytes.to_vec()).unwrap(),
                            };
                            verify_signature(&request);
                            requests.lock().unwrap().push(request);
                            let reply = script.lock().unwrap().pop_front().unwrap_or(Reply {
                                status: 500,
                                body: "unexpected request".into(),
                                etag: None,
                            });
                            let mut response = http::Response::builder()
                                .status(reply.status)
                                .header("content-type", "application/xml");
                            if let Some(etag) = reply.etag {
                                response = response.header("etag", etag);
                            }
                            Ok::<_, Infallible>(
                                response.body(Full::new(Bytes::from(reply.body))).unwrap(),
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
    fn driver(&self) -> CloudFrontDriver {
        CloudFrontDriver::new("DISTRIBUTION1", "public-pages", &keys())
            .unwrap()
            .with_response_headers_function(FUNCTION)
            .unwrap()
            .with_mock_endpoint(&self.endpoint)
            .unwrap()
            .with_deployment_wait(Duration::from_millis(1), Duration::from_secs(1))
            .unwrap()
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}
fn keys() -> S3Config {
    S3Config {
        access_key_id: Some("AKIDCLOUDFRONTTEST".into()),
        secret_access_key: Some(SECRET.into()),
        ..Default::default()
    }
}
fn mac(key: &[u8], value: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(value.as_bytes());
    mac.finalize().into_bytes().to_vec()
}
fn verify_signature(request: &Request) {
    let header = |name: &str| request.headers[name].to_str().unwrap();
    let fields = header("authorization")
        .strip_prefix("AWS4-HMAC-SHA256 ")
        .unwrap();
    let field = |name: &str| {
        fields
            .split(',')
            .map(str::trim)
            .find_map(|value| value.strip_prefix(&format!("{name}=")))
            .unwrap()
    };
    let credential = field("Credential");
    let scope = credential.strip_prefix("AKIDCLOUDFRONTTEST/").unwrap();
    let parts: Vec<_> = scope.split('/').collect();
    assert_eq!(&parts[1..], &["us-east-1", "cloudfront", "aws4_request"]);
    let signed = field("SignedHeaders");
    for name in ["host", "x-amz-date", "x-amz-content-sha256"] {
        assert!(signed.split(';').any(|value| value == name));
    }
    let payload_hash = hex::encode(Sha256::digest(request.body.as_bytes()));
    assert_eq!(header("x-amz-content-sha256"), payload_hash);
    let canonical_headers: String = signed
        .split(';')
        .map(|name| {
            format!(
                "{name}:{}\n",
                header(name)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        })
        .collect();
    let canonical = format!(
        "{}\n{}\n\n{canonical_headers}\n{signed}\n{payload_hash}",
        request.method, request.path
    );
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        header("x-amz-date"),
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );
    let key = mac(format!("AWS4{SECRET}").as_bytes(), parts[0]);
    let key = mac(&key, "us-east-1");
    let key = mac(&key, "cloudfront");
    let key = mac(&key, "aws4_request");
    assert_eq!(field("Signature"), hex::encode(mac(&key, &to_sign)));
}

#[tokio::test]
async fn pub_002_cloudfront_preserves_config_uses_etag_and_waits_for_deployed() {
    let mock = Mock::start(vec![
        Reply::config(CONFIG),
        Reply::ok("<Distribution><Status>InProgress</Status></Distribution>"),
        Reply::ok("<Distribution><Status>InProgress</Status></Distribution>"),
        Reply::ok("<Distribution><Status>Deployed</Status></Distribution>"),
    ])
    .await;
    mock.driver().switch_version("build-123").await.unwrap();
    let requests = mock.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/2020-05-31/distribution/DISTRIBUTION1/config"
    );
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].headers["if-match"], "etag-one");
    assert_eq!(
        requests[1].body,
        CONFIG.replacen(
            "<OriginPath></OriginPath>",
            "<OriginPath>/_pub/build-123</OriginPath>",
            1
        )
    );
    assert!(
        requests[2..]
            .iter()
            .all(|request| request.path == "/2020-05-31/distribution/DISTRIBUTION1")
    );
}

#[tokio::test]
async fn pub_004_cloudfront_creates_signed_xml_invalidations_for_urls_and_prefixes() {
    let mock = Mock::start(vec![
        Reply::ok("<Invalidation><Status>InProgress</Status></Invalidation>"),
        Reply::ok("<Invalidation/>"),
    ])
    .await;
    let driver = mock.driver();
    driver
        .purge(Purge::Urls(vec![
            "/about/index.html".into(),
            "/about/index.json?x=1&y=2".into(),
        ]))
        .await
        .unwrap();
    driver.purge(Purge::Prefix("/blog/".into())).await.unwrap();
    let requests = mock.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].path,
        "/2020-05-31/distribution/DISTRIBUTION1/invalidation"
    );
    assert!(requests[0].body.contains("<Quantity>2</Quantity>"));
    assert!(requests[0].body.contains("<Path>/about/index.html</Path>"));
    assert!(
        requests[0]
            .body
            .contains("<Path>/about/index.json?x=1&amp;y=2</Path>")
    );
    assert!(requests[1].body.contains("<Path>/blog/*</Path>"));
    assert!(!driver.capabilities().tags);
    assert!(
        driver
            .purge(Purge::Tags(vec!["posts".into()]))
            .await
            .is_err()
    );
    assert!(
        driver
            .purge(Purge::Urls(vec!["https://other.test/".into()]))
            .await
            .is_err()
    );
    assert_eq!(mock.requests().len(), 2);
}

#[tokio::test]
async fn pub_003_cloudfront_http_failure_and_missing_etag_do_not_update() {
    for reply in [
        Reply {
            status: 403,
            body: "secret-provider-detail".into(),
            etag: None,
        },
        Reply::ok(CONFIG),
    ] {
        let mock = Mock::start(vec![reply]).await;
        let error = mock
            .driver()
            .switch_version("v2")
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret-provider-detail"));
        assert_eq!(mock.requests().len(), 1);
    }
    let mock = Mock::start(vec![
        Reply::config(CONFIG),
        Reply {
            status: 412,
            body: "".into(),
            etag: None,
        },
    ])
    .await;
    assert!(
        mock.driver()
            .switch_version("v2")
            .await
            .unwrap_err()
            .to_string()
            .contains("412")
    );
    assert_eq!(mock.requests().len(), 2);
}

#[tokio::test]
async fn pub_003_cloudfront_declines_missing_or_duplicate_origin_and_invalid_xml() {
    for config in [
        CONFIG.replace("public-pages", "different"),
        CONFIG.replace("private-origin", "public-pages"),
        CONFIG.replace("<OriginPath></OriginPath>", ""),
        "<broken".into(),
    ] {
        let mock = Mock::start(vec![Reply::config(config)]).await;
        assert!(mock.driver().switch_version("v2").await.is_err());
        assert_eq!(mock.requests().len(), 1);
    }
}

#[tokio::test]
async fn pub_002_cloudfront_handles_empty_paths_and_namespace_prefixes() {
    let config = CONFIG.replace("<OriginPath></OriginPath>", "<OriginPath/>");
    let mock = Mock::start(vec![
        Reply::config(&config),
        Reply::ok(""),
        Reply::ok("<Distribution><Status>Deployed</Status></Distribution>"),
    ])
    .await;
    mock.driver().switch_version("v2").await.unwrap();
    assert_eq!(
        mock.requests()[1].body,
        config.replace("<OriginPath/>", "<OriginPath>/_pub/v2</OriginPath>")
    );
    let config = r#"<c:DistributionConfig xmlns:c="urn:cloudfront"><c:Origins><c:Items><c:Origin><c:OriginPath/><c:Id>public-pages</c:Id></c:Origin></c:Items></c:Origins></c:DistributionConfig>"#;
    let config = config.replace(
        "</c:DistributionConfig>",
        &format!("{BEHAVIOR}</c:DistributionConfig>"),
    );
    let mock = Mock::start(vec![Reply::config(config), Reply::ok(""), Reply::ok("<c:Distribution xmlns:c='urn:cloudfront'><c:Status>Deployed</c:Status></c:Distribution>")]).await;
    mock.driver().switch_version("v2").await.unwrap();
    assert!(
        mock.requests()[1]
            .body
            .contains("<c:OriginPath>/_pub/v2</c:OriginPath>")
    );
}

#[tokio::test]
async fn pub_003_cloudfront_deployment_timeout_and_unrecognized_status_are_errors() {
    for status in ["InProgress", "Unknown"] {
        let mut replies = vec![Reply::config(CONFIG), Reply::ok("")];
        replies.extend((0..10).map(|_| {
            Reply::ok(format!(
                "<Distribution><Status>{status}</Status></Distribution>"
            ))
        }));
        let mock = Mock::start(replies).await;
        let driver = mock
            .driver()
            .with_deployment_wait(Duration::from_millis(5), Duration::from_millis(2));
        assert!(driver.is_err());
        let driver = mock
            .driver()
            .with_deployment_wait(Duration::from_millis(2), Duration::from_millis(5))
            .unwrap();
        let message = driver.switch_version("v2").await.unwrap_err().to_string();
        assert!(message.contains(if status == "InProgress" {
            "timed out"
        } else {
            "recognized deployment status"
        }));
    }
}

#[tokio::test]
async fn pub_003_cloudfront_restores_original_config_after_initial_manifest_failure() {
    let updated = CONFIG.replacen(
        "<OriginPath></OriginPath>",
        "<OriginPath>/_pub/v2/revision</OriginPath>",
        1,
    );
    let mock = Mock::start(vec![
        Reply::config(CONFIG),
        Reply::ok(""),
        Reply::ok("<Distribution><Status>Deployed</Status></Distribution>"),
        Reply::config(&updated),
        Reply::ok(""),
        Reply::ok("<Distribution><Status>Deployed</Status></Distribution>"),
    ])
    .await;
    let driver = mock.driver();
    driver.switch_version("v2/revision").await.unwrap();
    driver.rollback_version(None).await.unwrap();
    let requests = mock.requests();
    assert_eq!(requests[4].method, "PUT");
    assert_eq!(requests[4].body, CONFIG);
    assert_eq!(requests[4].headers["if-match"], "etag-one");
}

#[test]
fn pub_002_cloudfront_validates_identifiers_credentials_and_mock_endpoint() {
    assert!(CloudFrontDriver::new("../distribution", "pages", &keys()).is_err());
    assert!(CloudFrontDriver::new("DISTRIBUTION1", "", &keys()).is_err());
    assert!(
        CloudFrontDriver::new(
            "DISTRIBUTION1",
            "pages",
            &S3Config {
                secret_access_key: None,
                ..keys()
            }
        )
        .is_err()
    );
    for endpoint in [
        "https://example.com/",
        "http://example.com/",
        "http://127.0.0.1/path",
        "http://user@127.0.0.1/",
        "http://127.0.0.1/?x=1",
    ] {
        assert!(
            CloudFrontDriver::new("DISTRIBUTION1", "pages", &keys())
                .unwrap()
                .with_mock_endpoint(endpoint)
                .is_err()
        );
    }
}

#[tokio::test]
async fn pub_002_cloudfront_refuses_switch_without_header_bridge_on_every_page_behavior() {
    for config in [
        CONFIG.replace(BEHAVIOR, ""),
        CONFIG.replace("viewer-response", "viewer-request"),
        CONFIG.replace("function/suprnova-public-headers", "function/different"),
        CONFIG.replace("</DistributionConfig>", "<CacheBehaviors><Items><CacheBehavior><TargetOriginId>public-pages</TargetOriginId></CacheBehavior></Items></CacheBehaviors></DistributionConfig>"),
    ] {
        let mock = Mock::start(vec![Reply::config(config)]).await;
        assert!(mock.driver().switch_version("v2").await.is_err());
        assert_eq!(mock.requests().len(), 1);
    }
}

#[test]
fn pub_002_cloudfront_header_bridge_restores_csp_and_hides_metadata() {
    let source = suprnova::static_publish::cdn::RESPONSE_HEADERS_FUNCTION;
    let test = r#"
const assert = require('node:assert/strict');
const event = {response: {statusCode: 200, headers: {
  'content-type': {value: 'text/html'},
  'x-amz-meta-content-security-policy': {value: "script-src 'self' 'sha256-trusted'"},
  'x-amz-meta-ignored': {value: 'hidden'}
}}};
const response = handler(event);
assert.equal(response.headers['content-security-policy'].value, "script-src 'self' 'sha256-trusted'");
assert.equal(Object.keys(response.headers).some(key => key.startsWith('x-amz-meta-')), false);
const missing = handler({response: {headers: {'content-type': {value: 'text/html'}}}});
assert.equal(missing.headers['content-security-policy'].value, "default-src 'none'; sandbox");
"#;
    let result = std::process::Command::new("bun")
        .args(["--no-env-file", "-e", &format!("{source}\n{test}")])
        .output()
        .expect("Bun runs the shipped CloudFront function");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
async fn pub_002_fix_a_cloudfront_maps_the_s3_root_and_validates_it() {
    let mock = Mock::start(vec![
        Reply::config(CONFIG),
        Reply::ok(""),
        Reply::ok("<Distribution><Status>Deployed</Status></Distribution>"),
    ])
    .await;
    let driver = CloudFrontDriver::new(
        "DISTRIBUTION1",
        "public-pages",
        &S3Config {
            root: Some("pages".into()),
            ..keys()
        },
    )
    .unwrap()
    .with_response_headers_function(FUNCTION)
    .unwrap()
    .with_mock_endpoint(&mock.endpoint)
    .unwrap();
    driver.switch_version("v2/revision").await.unwrap();
    assert!(
        mock.requests()[1]
            .body
            .contains("<OriginPath>/pages/_pub/v2/revision</OriginPath>")
    );
    for root in [
        "pages/../outside",
        "pages//nested",
        "pages?query",
        "pages\\nested",
    ] {
        assert!(
            CloudFrontDriver::new(
                "DISTRIBUTION1",
                "public-pages",
                &S3Config {
                    root: Some(root.into()),
                    ..keys()
                }
            )
            .is_err(),
            "invalid root {root}"
        );
    }
}
