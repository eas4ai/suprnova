use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use suprnova::search::{
    EmbedInput, Embedder,
    embedder::http::{HttpEmbedder, HttpEmbedderConfig},
};

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}
impl Reply {
    fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![],
            body: body.into(),
        }
    }
    fn vectors(vectors: Vec<Vec<f32>>) -> Self {
        Self::status(200, &json!({"data": vectors.into_iter().enumerate().rev().map(|(index, embedding)| json!({"index":index,"embedding":embedding,"extra":"ignored"})).collect::<Vec<_>>(), "usage":{"ignored":true}, "provider":"extra"}).to_string())
    }
}
#[derive(Debug)]
struct Seen {
    path: String,
    auth: Option<String>,
    body: Value,
}
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(replies: Vec<Reply>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(vec![]));
    let captured = seen.clone();
    let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let seen = captured.clone();
            let replies = replies.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let seen = seen.clone();
                    let replies = replies.clone();
                    async move {
                        let path = request.uri().path().to_string();
                        let auth = request
                            .headers()
                            .get("authorization")
                            .map(|v| v.to_str().unwrap().to_string());
                        // A model-list GET carries no body.
                        let body = serde_json::from_slice(
                            &request.into_body().collect().await.unwrap().to_bytes(),
                        )
                        .unwrap_or(Value::Null);
                        seen.lock().unwrap().push(Seen { path, auth, body });
                        let reply = replies
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or_else(|| Reply::status(500, "no reply"));
                        let mut builder = Response::builder().status(reply.status);
                        for (name, value) in reply.headers {
                            builder = builder.header(name, value);
                        }
                        Ok::<_, Infallible>(
                            builder.body(Full::new(Bytes::from(reply.body))).unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Server { url, seen, task }
}
fn config(url: &str) -> HttpEmbedderConfig {
    HttpEmbedderConfig {
        base_url: url.into(),
        model: "test-custom".into(),
        api_key: Some("test-secret".into()),
        dimensions: Some(2),
        batch: 2,
        max_tokens: Some(8192),
        ..Default::default()
    }
}
fn doc(text: &str) -> EmbedInput {
    EmbedInput {
        text: text.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn shape_batches_dimensions_and_response_order() {
    let server = server(vec![
        Reply::vectors(vec![vec![1., 0.], vec![0., 1.]]),
        Reply::vectors(vec![vec![1., 1.]]),
    ])
    .await;
    let embedder = HttpEmbedder::new(config(&server.url)).await.unwrap();
    let out = embedder
        .embed_documents(&[doc("one"), doc("two"), doc("three")])
        .await
        .unwrap();
    assert_eq!(out[0].dense, vec![1., 0.]);
    assert_eq!(out[1].dense, vec![0., 1.]);
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].path, "/v1/embeddings");
    assert_eq!(seen[0].auth.as_deref(), Some("Bearer test-secret"));
    assert_eq!(
        seen[0].body,
        json!({"model":"test-custom","input":["one","two"],"dimensions":2,"encoding_format":"float"})
    );
    assert_eq!(seen[1].body["input"], json!(["three"]));
}
#[tokio::test]
async fn prefix_and_template_apply_once() {
    let server = server(vec![
        Reply::vectors(vec![vec![1., 0.]]),
        Reply::vectors(vec![vec![1., 0.]]),
    ])
    .await;
    let mut options = config(&server.url);
    options.query_prefix = Some("query: ".into());
    options.document_template = Some("title: {title} | {text}".into());
    let embedder = HttpEmbedder::new(options).await.unwrap();
    embedder.embed_query("rust").await.unwrap();
    embedder
        .embed_documents(&[doc("{title} rust")])
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen[0].body["input"], json!(["query: rust"]));
    assert_eq!(seen[1].body["input"], json!(["title: none | {title} rust"]));
}
#[tokio::test]
async fn redirects_never_forward_the_key() {
    let destination = server(vec![Reply::vectors(vec![vec![1., 0.]])]).await;
    let mut reply = Reply::status(302, "redirect refused");
    reply
        .headers
        .push(("location", format!("{}/embeddings", destination.url)));
    let source = server(vec![reply]).await;
    let embedder = HttpEmbedder::new(config(&source.url)).await.unwrap();
    let error = embedder.embed_query("rust").await.unwrap_err().to_string();
    assert!(error.contains("302"));
    assert!(error.contains("redirect refused"));
    assert_eq!(source.seen.lock().unwrap().len(), 1);
    assert!(destination.seen.lock().unwrap().is_empty());
}
#[tokio::test]
async fn response_size_is_bounded_before_json() {
    let server = server(vec![Reply::status(200, &"x".repeat(4096))]).await;
    let mut options = config(&server.url);
    options.max_response_bytes = 100;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    let error = embedder.embed_query("rust").await.unwrap_err().to_string();
    assert!(error.contains("SEARCH_EMBED_MAX_RESPONSE_BYTES"));
}
#[tokio::test]
async fn transient_statuses_retry_and_honor_retry_after() {
    let mut limited = Reply::status(429, "slow down");
    limited.headers.push(("retry-after", "1".into()));
    let server = server(vec![
        limited,
        Reply::status(503, "unavailable"),
        Reply::vectors(vec![vec![1., 0.]]),
    ])
    .await;
    let embedder = HttpEmbedder::new(config(&server.url)).await.unwrap();
    let start = std::time::Instant::now();
    embedder.embed_query("rust").await.unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(server.seen.lock().unwrap().len(), 3);
}
#[tokio::test]
async fn permanent_status_is_not_retried_and_body_is_redacted_and_bounded() {
    let body = format!("test-secret {}", "x".repeat(300));
    let server = server(vec![Reply::status(400, &body)]).await;
    let embedder = HttpEmbedder::new(config(&server.url)).await.unwrap();
    let error = embedder.embed_query("rust").await.unwrap_err().to_string();
    assert!(error.contains("400"));
    assert!(!error.contains("test-secret"));
    assert!(error.len() < 400);
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn non_loopback_plaintext_and_url_credentials_are_refused() {
    for url in [
        "http://example.com/v1",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=secret",
    ] {
        let error = HttpEmbedder::new(config(url))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("SEARCH_EMBED_BASE_URL"));
        assert!(!error.contains("secret"));
    }
}
#[tokio::test]
async fn invalid_dimensions_and_indices_are_refused() {
    for body in [
        json!({"data":[{"index":0,"embedding":[1.]}]}),
        json!({"data":[{"index":1,"embedding":[1.,0.]}]}),
    ] {
        let server = server(vec![Reply::status(200, &body.to_string())]).await;
        let embedder = HttpEmbedder::new(config(&server.url)).await.unwrap();
        let error = embedder.embed_query("rust").await.unwrap_err().to_string();
        assert!(error.contains("expected 2") || error.contains("index"));
    }
}
#[tokio::test]
async fn unknown_dimensions_are_probed_once_and_empty_inputs_skip_requests() {
    let server = server(vec![
        Reply::vectors(vec![vec![1., 0., 0.]]),
        Reply::vectors(vec![vec![0., 1., 0.]]),
    ])
    .await;
    let mut options = config(&server.url);
    options.dimensions = None;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert_eq!(embedder.profile().dimensions, 3);
    assert!(embedder.embed_documents(&[]).await.unwrap().is_empty());
    embedder.embed_query("rust").await.unwrap();
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn retries_are_bounded() {
    let server = server(vec![Reply::status(503, "unavailable"); 4]).await;
    let mut options = config(&server.url);
    options.retries = 1;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert!(
        embedder
            .embed_query("rust")
            .await
            .unwrap_err()
            .to_string()
            .contains("503")
    );
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn optional_wire_fields_are_omitted_and_streamed_bodies_are_capped() {
    let server = server(vec![Reply::vectors(vec![vec![1., 0.]])]).await;
    let mut options = config(&server.url);
    options.dimensions = None;
    options.api_key = None;
    let profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    let embedder = HttpEmbedder::with_profile(options, Some(profile))
        .await
        .unwrap();
    embedder.embed_query("rust").await.unwrap();
    {
        let seen = server.seen.lock().unwrap();
        assert!(seen[0].auth.is_none());
        assert!(seen[0].body.get("dimensions").is_none());
    }
    let mut reply = Reply::status(200, &"x".repeat(4096));
    reply.headers.push(("transfer-encoding", "chunked".into()));
    let chunked = crate::embedder_http::server(vec![reply]).await;
    let mut options = config(&chunked.url);
    options.max_response_bytes = 100;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert!(
        embedder
            .embed_query("rust")
            .await
            .unwrap_err()
            .to_string()
            .contains("SEARCH_EMBED_MAX_RESPONSE_BYTES")
    );
}

#[tokio::test]
async fn loopback_hosts_and_positive_configuration_limits_are_checked_at_boot() {
    for url in [
        "http://localhost:1234/v1",
        "http://127.0.0.1:1234/v1",
        "http://[::1]:1234/v1",
    ] {
        assert!(HttpEmbedder::new(config(url)).await.is_ok());
    }
    let mut options = config("http://localhost:1234/v1");
    options.batch = 0;
    assert!(
        HttpEmbedder::new(options)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("SEARCH_EMBED_BATCH")
    );
    let mut options = config("http://localhost:1234/v1");
    options.timeout_secs = 0;
    assert!(
        HttpEmbedder::new(options)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("SEARCH_EMBED_TIMEOUT_SECS")
    );
    let mut options = config("http://localhost:1234/v1");
    options.api_key = Some("invalid\r\ncredential".into());
    let error = HttpEmbedder::new(options).await.err().unwrap().to_string();
    assert!(error.contains("SEARCH_EMBED_API_KEY"));
    assert!(!error.contains("credential"));
}

#[tokio::test]
async fn reduced_provider_dimensions_pass_the_core_validation_boundary() {
    let server = server(vec![Reply::vectors(vec![vec![1., 1.]])]).await;
    let mut profile = suprnova::search::FakeEmbedder::new(4)
        .unwrap()
        .profile()
        .clone();
    profile.truncation_dims = vec![2];
    profile.normalize = false;
    let embedder = HttpEmbedder::with_profile(config(&server.url), Some(profile))
        .await
        .unwrap();
    let vector = embedder.embed_query("rust").await.unwrap();
    assert_eq!(embedder.profile().dimensions, 2);
    assert!(
        (vector
            .dense
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            - 1.0)
            .abs()
            < 1e-6
    );
    suprnova::search::validate_embedding(vector.dense, embedder.profile(), None).unwrap();
}

#[tokio::test]
async fn text_provider_refuses_a_profile_claiming_media_capabilities() {
    let mut profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    profile.inputs.image = true;
    let error = HttpEmbedder::with_profile(config("http://localhost:1234/v1"), Some(profile))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("SEARCH_EMBED_MODEL"));
    assert!(error.contains("image"));
}

#[tokio::test]
async fn prepared_input_is_cut_to_the_profile_context() {
    let server = server(vec![
        Reply::vectors(vec![vec![1., 0.]]),
        Reply::vectors(vec![vec![1., 0.]]),
    ])
    .await;
    let mut profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    profile.max_tokens = 16;
    profile.query_prefix = "query: ".into();
    let mut options = config(&server.url);
    options.max_tokens = None;
    let embedder = HttpEmbedder::with_profile(options, Some(profile))
        .await
        .unwrap();
    embedder.embed_query(&"é".repeat(40)).await.unwrap();
    embedder
        .embed_documents(&[doc(&"rust ".repeat(40))])
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    // Sixteen tokens less the four reserved for special tokens: twelve bytes, cut on a
    // character boundary.
    assert_eq!(seen[0].body["input"], json!(["query: éé"]));
    let document = seen[1].body["input"][0].as_str().unwrap();
    assert!(!document.is_empty() && document.len() <= 12, "{document}");
}

#[tokio::test]
async fn a_named_tokenizer_cuts_input_at_the_exact_token_count() {
    let server = server(vec![
        Reply::vectors(vec![vec![1., 0.]]),
        Reply::vectors(vec![vec![1., 0.]]),
    ])
    .await;
    let mut profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    profile.max_tokens = 16;
    profile.query_prefix = "query: ".into();
    let mut options = config(&server.url);
    options.max_tokens = None;
    // A word-level tokenizer with no special tokens: one token per whitespace word.
    options.tokenizer = Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/search/fixtures/tokenizer.json"),
    );
    let embedder = HttpEmbedder::with_profile(options, Some(profile))
        .await
        .unwrap();
    embedder.embed_query(&"low ".repeat(40)).await.unwrap();
    embedder
        .embed_documents(&[doc(&"high ".repeat(40))])
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    let query = seen[0].body["input"][0].as_str().unwrap();
    assert!(query.starts_with("query: low"), "{query}");
    assert_eq!(query.split_whitespace().count(), 16, "{query}");
    let document = seen[1].body["input"][0].as_str().unwrap();
    assert_eq!(document.split_whitespace().count(), 16, "{document}");
    assert!(!document.ends_with(' '), "{document}");
}

#[tokio::test]
async fn an_unreadable_tokenizer_fails_at_boot_naming_its_setting() {
    let mut options = config("http://localhost:1234/v1");
    options.tokenizer = Some("/nonexistent/tokenizer.json".into());
    let error = HttpEmbedder::with_profile(
        options,
        Some(
            suprnova::search::FakeEmbedder::new(2)
                .unwrap()
                .profile()
                .clone(),
        ),
    )
    .await
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("SEARCH_EMBED_TOKENIZER"), "{error}");
}

#[tokio::test]
async fn an_unknown_models_context_comes_from_the_service_model_list() {
    let server = server(vec![Reply::status(
        200,
        &json!({"data": [{"id": "other", "context_length": 99}, {"id": "test-custom", "context_length": 16}]})
            .to_string(),
    )])
    .await;
    let mut options = config(&server.url);
    options.max_tokens = None;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert_eq!(embedder.profile().max_tokens, 16);
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen[0].path, "/v1/embeddings/models");
    assert_eq!(seen[0].auth.as_deref(), Some("Bearer test-secret"));
}

#[tokio::test]
async fn a_server_model_list_supplies_its_own_context_field() {
    let server = server(vec![
        Reply::status(404, "not found"),
        Reply::status(
            200,
            &json!({"data": [{"id": "test-custom", "max_model_len": 512}]}).to_string(),
        ),
    ])
    .await;
    let mut options = config(&server.url);
    options.max_tokens = None;
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert_eq!(embedder.profile().max_tokens, 512);
    let paths = server
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|seen| seen.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/v1/embeddings/models", "/v1/models"]);
}

#[tokio::test]
async fn an_unknown_model_with_no_listed_context_fails_naming_the_setting() {
    let server = server(vec![
        Reply::status(404, "not found"),
        Reply::status(200, &json!({"data": [{"id": "test-custom"}]}).to_string()),
    ])
    .await;
    let mut options = config(&server.url);
    options.max_tokens = None;
    let error = HttpEmbedder::new(options).await.err().unwrap().to_string();
    assert!(error.contains("SEARCH_EMBED_MAX_TOKENS"), "{error}");
}

#[tokio::test]
async fn a_configured_context_wins_without_asking_the_service() {
    let server = server(vec![]).await;
    let mut options = config(&server.url);
    options.max_tokens = Some(64);
    let embedder = HttpEmbedder::new(options).await.unwrap();
    assert_eq!(embedder.profile().max_tokens, 64);
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn published_hosted_models_need_no_model_list() {
    let server = server(vec![]).await;
    for (model, dimensions, max_tokens) in [
        ("text-embedding-3-small", 1536, 8192),
        ("openai/text-embedding-3-large", 3072, 8192),
        ("gemini-embedding-001", 3072, 2048),
    ] {
        let mut options = config(&server.url);
        options.model = model.into();
        options.dimensions = None;
        options.max_tokens = None;
        let embedder = HttpEmbedder::new(options).await.unwrap();
        assert_eq!(embedder.profile().dimensions, dimensions, "{model}");
        assert_eq!(embedder.profile().max_tokens, max_tokens, "{model}");
    }
    assert!(server.seen.lock().unwrap().is_empty());
}
