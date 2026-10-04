//! Multipart validation through the whole request path (PAR-042, PAR-043).
//!
//! Every test drives `handle_request` over an in-memory HTTP/1.1
//! connection, so the middleware chain, the extractor and the error
//! rendering all run as they do for a real client. The client side of the
//! connection is written by hand, chunk by chunk, which is what lets a test
//! hold the body back, count how much of it the server pulled, or never
//! finish it: a test whose body never ends can only get a response if the
//! server stopped reading on its own.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use suprnova::http::upload::validators::{ImageFile, MaxSize, UploadValidator};
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};
use suprnova::session::SessionData;
use suprnova::testing::TestContainer;
use suprnova::{
    FrameworkError, FromRequest, HttpResponse, InertiaResponse,
    InertiaValidationRedirectMiddleware, Middleware, MiddlewareRegistry, MultipartRequest, Next,
    Request, Response, Router, ValidationErrors, handle_request,
};

// ── The connection harness ──

const BOUNDARY: &str = "par043boundary";

/// How long a test waits for a response before it calls the server stuck.
const WAIT: Duration = Duration::from_secs(20);

/// Capacity of the in-memory pipe between client and server. Small, so the
/// client can only write ahead of the server by this much: a chunk is
/// pulled from the client's source only once the server has taken most of
/// the one before it.
const PIPE_BYTES: usize = 64 * 1024;

/// A router and middleware stack the harness serves.
#[derive(Clone)]
struct App {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
}

impl App {
    fn new(router: impl Into<Router>) -> Self {
        Self::with(router, MiddlewareRegistry::new())
    }

    fn with(router: impl Into<Router>, middleware: MiddlewareRegistry) -> Self {
        Self {
            router: Arc::new(router.into()),
            middleware: Arc::new(middleware),
        }
    }
}

/// The request the client writes.
struct Outgoing {
    method: &'static str,
    path: &'static str,
    headers: Vec<(&'static str, String)>,
    /// Body chunks, written one at a time with chunked transfer encoding.
    chunks: Vec<Vec<u8>>,
    /// Whether the body ends after the chunks. When it does not, the
    /// connection stays open with no further byte, forever.
    finish: bool,
    /// When set, the body waits for this notification after the head.
    gate: Option<Arc<Notify>>,
    /// The body's media type, multipart unless a test says otherwise.
    content_type: Option<&'static str>,
}

impl Outgoing {
    fn post(path: &'static str, body: Vec<u8>) -> Self {
        Self::post_chunks(path, vec![body])
    }

    fn post_chunks(path: &'static str, chunks: Vec<Vec<u8>>) -> Self {
        Self {
            method: "POST",
            path,
            headers: Vec::new(),
            chunks,
            finish: true,
            gate: None,
            content_type: None,
        }
    }

    fn get(path: &'static str) -> Self {
        Self {
            method: "GET",
            path,
            headers: Vec::new(),
            chunks: Vec::new(),
            finish: true,
            gate: None,
            content_type: None,
        }
    }

    /// Send the body as `media_type` rather than multipart.
    fn content_type(mut self, media_type: &'static str) -> Self {
        self.content_type = Some(media_type);
        self
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    /// Never end the body after the chunks.
    fn unfinished(mut self) -> Self {
        self.finish = false;
        self
    }

    /// Send no body byte until `gate` is notified.
    fn gated(mut self, gate: Arc<Notify>) -> Self {
        self.gate = Some(gate);
        self
    }
}

/// What came back, and how many body chunks the client had pulled from its
/// source when the response arrived.
struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
    chunks_sent: usize,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn send(app: &App, out: Outgoing) -> Reply {
    let (client, server) = tokio::io::duplex(PIPE_BYTES);
    let router = app.router.clone();
    let middleware = app.middleware.clone();
    // `TestContainer::spawn` carries a test's container scope (a bound
    // translator) into the task that serves the request.
    let server_task = TestContainer::spawn(async move {
        let svc = service_fn(move |req| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server), svc)
            .await;
    });

    let (mut reader, mut writer) = tokio::io::split(client);
    let sent = Arc::new(AtomicUsize::new(0));
    let writer_task = {
        let sent = sent.clone();
        tokio::spawn(async move {
            let mut head = format!(
                "{} {} HTTP/1.1\r\nHost: localhost\r\n",
                out.method, out.path
            );
            if out.method == "POST" {
                let content_type = out.content_type.map_or_else(
                    || format!("multipart/form-data; boundary={BOUNDARY}"),
                    str::to_string,
                );
                head.push_str(&format!(
                    "Content-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n"
                ));
            }
            for (name, value) in &out.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            if writer.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            if let Some(gate) = &out.gate {
                gate.notified().await;
            }
            if out.method == "POST" {
                for chunk in &out.chunks {
                    sent.fetch_add(1, Ordering::SeqCst);
                    let mut framed = format!("{:x}\r\n", chunk.len()).into_bytes();
                    framed.extend_from_slice(chunk);
                    framed.extend_from_slice(b"\r\n");
                    if writer.write_all(&framed).await.is_err() {
                        return;
                    }
                }
                if out.finish && writer.write_all(b"0\r\n\r\n").await.is_err() {
                    return;
                }
            }
            // Hold the connection open: an unfinished body never ends.
            std::future::pending::<()>().await;
        })
    };

    let (status, headers, body) = tokio::time::timeout(WAIT, read_response(&mut reader))
        .await
        .expect("no response arrived: the server is still waiting for the body");
    let chunks_sent = sent.load(Ordering::SeqCst);
    writer_task.abort();
    server_task.abort();
    Reply {
        status,
        headers,
        body,
        chunks_sent,
    }
}

async fn read_response(
    reader: &mut (impl AsyncRead + Unpin),
) -> (u16, HashMap<String, String>, Vec<u8>) {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(at) = find(&buf, b"\r\n\r\n") {
            break at;
        }
        fill(reader, &mut buf).await;
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let mut rest = buf.split_off(head_end + 4);
    let body = if let Some(length) = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        while rest.len() < length {
            fill(reader, &mut rest).await;
        }
        rest.truncate(length);
        rest
    } else if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.contains("chunked"))
    {
        decode_chunked(reader, rest).await
    } else {
        Vec::new()
    };
    (status, headers, body)
}

async fn decode_chunked(reader: &mut (impl AsyncRead + Unpin), mut raw: Vec<u8>) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let line_end = loop {
            if let Some(at) = find(&raw, b"\r\n") {
                break at;
            }
            fill(reader, &mut raw).await;
        };
        let size_line = String::from_utf8_lossy(&raw[..line_end]).into_owned();
        let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
            .expect("a chunk size");
        raw.drain(..line_end + 2);
        if size == 0 {
            return body;
        }
        while raw.len() < size + 2 {
            fill(reader, &mut raw).await;
        }
        body.extend_from_slice(&raw[..size]);
        raw.drain(..size + 2);
    }
}

async fn fill(reader: &mut (impl AsyncRead + Unpin), buf: &mut Vec<u8>) {
    let mut tmp = [0u8; 16 * 1024];
    let n = reader.read(&mut tmp).await.expect("read the response");
    assert!(
        n > 0,
        "the connection closed before the response was complete"
    );
    buf.extend_from_slice(&tmp[..n]);
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ── Multipart bodies ──

/// The head of one part: boundary line and headers.
fn part_head(name: &str, file: Option<(&str, &str)>) -> Vec<u8> {
    let mut head = format!("--{BOUNDARY}\r\n").into_bytes();
    match file {
        Some((file_name, content_type)) => head.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        ),
        None => head.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        ),
    }
    head
}

fn text_part(name: &str, value: &str) -> Vec<u8> {
    text_part_bytes(name, value.as_bytes())
}

/// A text part carrying `value` as sent, which need not be UTF-8.
fn text_part_bytes(name: &str, value: &[u8]) -> Vec<u8> {
    let mut part = part_head(name, None);
    part.extend_from_slice(value);
    part.extend_from_slice(b"\r\n");
    part
}

fn file_part(name: &str, file_name: &str, content_type: &str, bytes: &[u8]) -> Vec<u8> {
    let mut part = part_head(name, Some((file_name, content_type)));
    part.extend_from_slice(bytes);
    part.extend_from_slice(b"\r\n");
    part
}

fn closing() -> Vec<u8> {
    format!("--{BOUNDARY}--\r\n").into_bytes()
}

fn form(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = parts.concat();
    body.extend_from_slice(&closing());
    body
}

/// A minimal PNG: signature and IHDR chunk, enough for magic-byte sniffing.
fn png() -> Vec<u8> {
    let mut bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&[0; 13]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
}

fn pdf() -> Vec<u8> {
    b"%PDF-1.4 a document, not a picture".to_vec()
}

fn errors(reply: &Reply) -> serde_json::Map<String, Value> {
    reply
        .json()
        .get("errors")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "no `errors` object in a {} body: {}",
                reply.status,
                reply.text()
            )
        })
}

fn first_message(reply: &Reply, key: &str) -> String {
    errors(reply)
        .get(key)
        .and_then(|messages| messages.get(0))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no message under `{key}`: {}", reply.text()))
        .to_string()
}

/// The upload spill files on disk whose content starts with `marker`. The
/// marker is unique to one test, so files other tests spill at the same
/// time never match.
fn spill_files_starting_with(marker: &[u8]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("suprnova-upload-"))
        })
        .filter(|path| {
            let mut head = vec![0u8; marker.len()];
            std::fs::File::open(path)
                .and_then(|mut file| file.read_exact(&mut head))
                .is_ok()
                && head == marker
        })
        .collect()
}

fn marker() -> Vec<u8> {
    format!("PAR043-{}", uuid::Uuid::new_v4().simple()).into_bytes()
}

/// `len` bytes that start with `marker`.
fn marked(marker: &[u8], len: usize) -> Vec<u8> {
    let mut bytes = marker.to_vec();
    bytes.resize(len, b'.');
    bytes
}

// ── PAR-043: a field's failure is a validation error under its input name ──

#[derive(MultipartRequest)]
struct ImageGallery {
    #[field("title")]
    title: String,
    #[field("files[]")]
    files: Vec<UploadedFile<ImageFile>>,
}

async fn gallery(req: Request) -> Response {
    let form = ImageGallery::from_request(req).await?;
    Ok(HttpResponse::json(
        json!({ "title": form.title, "files": form.files.len() }),
    ))
}

fn gallery_body(second: &[u8]) -> Vec<u8> {
    form(&[
        text_part("title", "Holiday"),
        file_part("files[]", "a.png", "image/png", &png()),
        file_part("files[]", "b.png", "image/png", second),
    ])
}

#[tokio::test]
async fn a_pdf_as_the_second_image_answers_422_under_files_1() {
    let app = App::new(Router::new().post("/gallery", gallery));

    let reply = send(&app, Outgoing::post("/gallery", gallery_body(&pdf()))).await;

    assert_eq!(reply.status, 422, "{}", reply.text());
    let errors = errors(&reply);
    assert!(
        errors.contains_key("files.1"),
        "the second file is `files.1`: {}",
        reply.text()
    );
    assert_eq!(
        errors.len(),
        1,
        "only the second file failed: {}",
        reply.text()
    );
}

#[tokio::test]
async fn two_images_pass_the_gallery() {
    let app = App::new(Router::new().post("/gallery", gallery));

    let reply = send(&app, Outgoing::post("/gallery", gallery_body(&png()))).await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["files"], 2);
}

#[derive(MultipartRequest)]
struct AvatarForm {
    #[field("avatar")]
    avatar: UploadedFile<ImageFile>,
    #[field("caption")]
    caption: Option<String>,
}

async fn avatar(req: Request) -> Response {
    let form = AvatarForm::from_request(req).await?;
    Ok(HttpResponse::json(
        json!({ "size": form.avatar.size, "caption": form.caption }),
    ))
}

#[tokio::test]
async fn a_missing_required_file_answers_422_under_its_name() {
    let app = App::new(Router::new().post("/avatar", avatar));

    // Left out of the form entirely.
    let omitted = send(
        &app,
        Outgoing::post("/avatar", form(&[text_part("caption", "me")])),
    )
    .await;
    assert_eq!(omitted.status, 422, "{}", omitted.text());
    assert!(
        errors(&omitted).contains_key("avatar"),
        "{}",
        omitted.text()
    );

    // What a browser sends for a file input left empty: a file part with
    // no file name and no bytes. It is missing, not an empty image.
    let empty_input = send(
        &app,
        Outgoing::post(
            "/avatar",
            form(&[
                file_part("avatar", "", "application/octet-stream", b""),
                text_part("caption", "me"),
            ]),
        ),
    )
    .await;
    assert_eq!(empty_input.status, 422, "{}", empty_input.text());
    assert_eq!(
        first_message(&empty_input, "avatar"),
        first_message(&omitted, "avatar"),
        "an empty file input reads as a missing file"
    );

    // What Inertia sends for a file left `null`: an empty text part.
    let null_file = send(
        &app,
        Outgoing::post(
            "/avatar",
            form(&[text_part("avatar", ""), text_part("caption", "me")]),
        ),
    )
    .await;
    assert_eq!(null_file.status, 422, "{}", null_file.text());
    assert_eq!(
        first_message(&null_file, "avatar"),
        first_message(&omitted, "avatar"),
        "a null file reads as a missing file"
    );
}

#[derive(MultipartRequest)]
struct OptionalAvatar {
    #[field("avatar")]
    avatar: Option<UploadedFile<ImageFile>>,
}

async fn optional_avatar(req: Request) -> Response {
    let form = OptionalAvatar::from_request(req).await?;
    Ok(HttpResponse::json(
        json!({ "present": form.avatar.is_some() }),
    ))
}

#[tokio::test]
async fn an_optional_image_left_empty_is_absent_not_invalid() {
    let app = App::new(Router::new().post("/optional", optional_avatar));

    for body in [
        form(&[file_part("avatar", "", "application/octet-stream", b"")]),
        form(&[text_part("avatar", "")]),
        form(&[]),
    ] {
        let reply = send(&app, Outgoing::post("/optional", body)).await;
        assert_eq!(reply.status, 200, "{}", reply.text());
        assert_eq!(reply.json()["present"], false);
    }
}

#[tokio::test]
async fn a_part_of_the_wrong_kind_answers_422_under_its_name() {
    let app = App::new(
        Router::new()
            .post("/avatar", avatar)
            .post("/gallery", gallery),
    );

    // Text where a file belongs.
    let text_for_file = send(
        &app,
        Outgoing::post("/avatar", form(&[text_part("avatar", "not a file")])),
    )
    .await;
    assert_eq!(text_for_file.status, 422, "{}", text_for_file.text());
    assert!(
        errors(&text_for_file).contains_key("avatar"),
        "{}",
        text_for_file.text()
    );

    // A file where text belongs.
    let file_for_text = send(
        &app,
        Outgoing::post(
            "/gallery",
            form(&[
                file_part("title", "t.txt", "text/plain", b"Holiday"),
                file_part("files[]", "a.png", "image/png", &png()),
            ]),
        ),
    )
    .await;
    assert_eq!(file_for_text.status, 422, "{}", file_for_text.text());
    assert!(
        errors(&file_for_text).contains_key("title"),
        "{}",
        file_for_text.text()
    );
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // the assertions are on the failures, never the values
struct Typed {
    #[field("count")]
    count: u32,
    #[field("ratio")]
    ratio: f64,
    #[field("active")]
    active: bool,
    #[field("address")]
    address: std::net::IpAddr,
    #[field("tags[]")]
    tags: Vec<i64>,
}

async fn typed(req: Request) -> Response {
    Typed::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn a_text_part_that_does_not_parse_answers_422_with_the_type_key() {
    let app = App::new(Router::new().post("/typed", typed));
    let body = form(&[
        text_part("count", "three"),
        text_part("ratio", "half"),
        text_part("active", "maybe"),
        text_part("address", "nowhere"),
        text_part("tags[]", "1"),
        text_part("tags[]", "two"),
    ]);

    let reply = TestContainer::scope(async {
        let _catalog = bind_catalog("");
        send(&app, Outgoing::post("/typed", body)).await
    })
    .await;

    assert_eq!(
        reply.status,
        422,
        "a parse failure is invalid input: {}",
        reply.text()
    );
    assert_eq!(
        first_message(&reply, "count"),
        "The count field must be an integer."
    );
    assert_eq!(
        first_message(&reply, "ratio"),
        "The ratio field must be a number."
    );
    assert_eq!(
        first_message(&reply, "active"),
        "The active field must be true or false."
    );
    assert_eq!(
        first_message(&reply, "address"),
        "The address field format is invalid."
    );
    assert_eq!(
        first_message(&reply, "tags.1"),
        "The tags.1 field must be an integer."
    );
    assert!(
        !errors(&reply).contains_key("tags.0"),
        "the first tag parsed: {}",
        reply.text()
    );
}

/// The catalog key of every message, read off the extractor's error
/// rather than rendered text.
#[tokio::test]
async fn each_field_failure_carries_its_catalog_key() {
    let body = form(&[
        text_part("count", "three"),
        text_part("ratio", "half"),
        text_part("active", "maybe"),
        file_part("address", "a.txt", "text/plain", b"127.0.0.1"),
        text_part("tags[]", "1"),
        text_part("tags[]", "two"),
    ]);
    let errors = typed_errors(body).await;
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(key(&errors, "ratio"), "validation-numeric");
    assert_eq!(key(&errors, "active"), "validation-boolean");
    assert_eq!(
        key(&errors, "address"),
        "validation-string",
        "a file where text belongs"
    );
    assert_eq!(key(&errors, "tags.1"), "validation-integer");
    assert_eq!(errors.errors.len(), 5, "{errors}");

    let errors = typed_errors(form(&[text_part("address", "nowhere")])).await;
    assert_eq!(key(&errors, "address"), "validation-format");
    assert_eq!(key(&errors, "count"), "validation-required");
}

async fn typed_errors(body: Vec<u8>) -> ValidationErrors {
    let req = crate::common::request_from_multipart(BOUNDARY, body.into()).await;
    match Typed::from_request(req).await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {:?}", other.err()),
    }
}

fn key(errors: &ValidationErrors, field: &str) -> String {
    errors
        .errors
        .get(field)
        .and_then(|messages| messages.first())
        .map(|message| message.key.to_string())
        .unwrap_or_else(|| panic!("no error under `{field}`: {errors}"))
}

/// Bind a translator over the framework's English catalog plus `overrides`,
/// an application's `lang/en/validation.ftl`, for the current
/// `TestContainer::scope`.
fn bind_catalog(overrides: &str) -> tempfile::TempDir {
    use suprnova::{FluentTranslator, Locale, LocalizationConfig, Translator};

    let dir = tempfile::tempdir().expect("a lang directory");
    std::fs::create_dir_all(dir.path().join("en")).expect("lang/en");
    std::fs::write(dir.path().join("en").join("validation.ftl"), overrides)
        .expect("lang/en/validation.ftl");
    let config = LocalizationConfig {
        default_locale: Locale::parse("en").expect("en"),
        fallback_locale: Locale::parse("en").expect("en"),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
        parents: Default::default(),
    };
    let translator = FluentTranslator::from_dir(dir.path(), &config).expect("the catalog loads");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));
    dir
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // the assertions are on the failures, never the values
struct Attachments {
    #[field("cover")]
    cover: UploadedFile<ImageFile>,
    #[field("scan")]
    scan: UploadedFile<MaxSize<2048>>,
}

async fn attachments(req: Request) -> Response {
    Attachments::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn messages_come_from_the_catalog_by_key_and_an_application_overrides_them() {
    let app = App::new(
        Router::new()
            .post("/gallery", gallery)
            .post("/attachments", attachments),
    );

    let (english, sizes) = TestContainer::scope(async {
        let _catalog = bind_catalog("");
        let english = send(&app, Outgoing::post("/gallery", gallery_body(&pdf()))).await;
        let sizes = send(
            &app,
            Outgoing::post(
                "/attachments",
                form(&[
                    file_part("cover", "c.png", "image/png", &png()),
                    file_part("scan", "s.bin", "application/octet-stream", &[7u8; 4096]),
                ]),
            ),
        )
        .await;
        (english, sizes)
    })
    .await;
    // Its own container scope, so the application catalog never reaches
    // the English requests above.
    let overridden = TestContainer::scope(async {
        let _catalog =
            bind_catalog("validation-image = Upload a picture for { $field }, please.\n");
        send(&app, Outgoing::post("/gallery", gallery_body(&pdf()))).await
    })
    .await;

    assert_eq!(
        first_message(&english, "files.1"),
        "The files.1 field must be an image."
    );
    assert_eq!(
        first_message(&overridden, "files.1"),
        "Upload a picture for files.1, please.",
        "the application's catalog entry wins"
    );
    assert_eq!(
        first_message(&sizes, "scan"),
        "The scan field must not be greater than 2 kilobytes.",
        "the limit reads in kilobytes, as Laravel words it"
    );
}

// ── PAR-043: a failure found while the body streams stops the read ──

/// 3 MiB, a whole number of `STREAM_CHUNK`s.
const VIDEO_MAX: usize = 3 * 1024 * 1024;
const STREAM_CHUNK: usize = 256 * 1024;

static LARGE_UPLOAD_HOOK_RAN: AtomicBool = AtomicBool::new(false);
static LARGE_UPLOAD_HANDLER_RAN: AtomicBool = AtomicBool::new(false);

#[derive(MultipartRequest)]
#[multipart(custom_hooks, max_body_bytes = 64 * 1024 * 1024)]
#[allow(dead_code)] // the handler never sees a value: extraction fails
struct LargeUploads {
    #[field("document")]
    document: UploadedFile,
    #[field("video")]
    video: UploadedFile<MaxSize<VIDEO_MAX>>,
}

impl MultipartRequestHooks for LargeUploads {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        LARGE_UPLOAD_HOOK_RAN.store(true, Ordering::SeqCst);
        Ok(())
    }
}

async fn large_uploads(req: Request) -> Response {
    LargeUploads::from_request(req).await?;
    LARGE_UPLOAD_HANDLER_RAN.store(true, Ordering::SeqCst);
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn an_oversized_file_stops_the_read_at_the_crossing_chunk_and_leaves_no_temp_file() {
    let app = App::new(Router::new().post("/large", large_uploads));
    let document_marker = marker();
    let video_marker = marker();

    // A finished 2.5 MiB document, which spills to disk, then the video's
    // part head; split into `STREAM_CHUNK` pieces.
    let mut prefix = file_part(
        "document",
        "doc.bin",
        "application/octet-stream",
        &marked(&document_marker, 5 * 512 * 1024),
    );
    prefix.extend_from_slice(&part_head(
        "video",
        Some(("v.bin", "application/octet-stream")),
    ));
    let mut chunks: Vec<Vec<u8>> = prefix.chunks(STREAM_CHUNK).map(<[u8]>::to_vec).collect();
    let prefix_chunks = chunks.len();
    // Then 16 MiB of video that never ends.
    let video_chunks = 64;
    let video = marked(&video_marker, video_chunks * STREAM_CHUNK);
    chunks.extend(video.chunks(STREAM_CHUNK).map(<[u8]>::to_vec));
    // The video crosses its limit inside its 13th chunk.
    let crossing = prefix_chunks + VIDEO_MAX / STREAM_CHUNK + 1;

    let reply = send(&app, Outgoing::post_chunks("/large", chunks).unfinished()).await;

    assert_eq!(reply.status, 422, "{}", reply.text());
    assert!(errors(&reply).contains_key("video"), "{}", reply.text());
    // What the pipe and hyper's read buffer hold ahead of the server is
    // the only slack: well under 2 MiB, so 6 chunks.
    assert!(
        reply.chunks_sent <= crossing + 6,
        "the read stops after the crossing chunk ({crossing}); the client had sent {}",
        reply.chunks_sent
    );
    assert!(
        !LARGE_UPLOAD_HOOK_RAN.load(Ordering::SeqCst),
        "the hook ran"
    );
    assert!(
        !LARGE_UPLOAD_HANDLER_RAN.load(Ordering::SeqCst),
        "the handler ran"
    );
    assert!(
        spill_files_starting_with(&document_marker).is_empty(),
        "the finished document's temp file was left behind"
    );
    assert!(
        spill_files_starting_with(&video_marker).is_empty(),
        "the oversized video's temp file was left behind"
    );
}

#[derive(MultipartRequest)]
// Its own cap: `uploads` lowers the process-global one while it runs.
#[multipart(max_body_bytes = 64 * 1024 * 1024)]
struct Spilled {
    #[field("document")]
    document: UploadedFile,
    #[field("marker")]
    marker: String,
}

async fn spilled(req: Request) -> Response {
    let form = Spilled::from_request(req).await?;
    let on_disk = spill_files_starting_with(form.marker.as_bytes()).len();
    Ok(HttpResponse::json(
        json!({ "on_disk": on_disk, "size": form.document.size }),
    ))
}

/// The positive control for the temp-file checks above: while an extracted
/// form holds a spilled file, the scan finds it.
#[tokio::test]
async fn a_spilled_file_is_on_disk_while_the_handler_holds_it() {
    let app = App::new(Router::new().post("/spilled", spilled));
    let marker = marker();
    let body = form(&[
        file_part(
            "document",
            "doc.bin",
            "application/octet-stream",
            &marked(&marker, 5 * 512 * 1024),
        ),
        text_part("marker", std::str::from_utf8(&marker).expect("ascii")),
    ]);

    let reply = send(&app, Outgoing::post("/spilled", body)).await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["on_disk"], 1, "{}", reply.text());
}

// ── PAR-043: a validator's operational error keeps its status ──

#[derive(Default)]
struct ScannerDown;

impl UploadValidator for ScannerDown {
    fn validate_final(
        &self,
        _sniff: &[u8],
        _size: u64,
        _content_type: Option<&str>,
    ) -> Result<(), FrameworkError> {
        Err(FrameworkError::Domain {
            message: "the virus scanner is unavailable".into(),
            status_code: 503,
        })
    }
}

#[derive(Default)]
struct DiskFull;

impl UploadValidator for DiskFull {
    fn validate_chunk(&self, _sniff: &[u8], _size: u64) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the quarantine disk is full"))
    }
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // the assertions are on the failures, never the values
struct Scanned {
    #[field("final")]
    at_final: Option<UploadedFile<ScannerDown>>,
    #[field("chunk")]
    at_chunk: Option<UploadedFile<DiskFull>>,
}

async fn scanned(req: Request) -> Response {
    Scanned::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn a_validators_operational_error_keeps_its_own_status() {
    let app = App::new(Router::new().post("/scanned", scanned));

    let at_final = send(
        &app,
        Outgoing::post(
            "/scanned",
            form(&[file_part(
                "final",
                "f.bin",
                "application/octet-stream",
                b"data",
            )]),
        ),
    )
    .await;
    assert_eq!(at_final.status, 503, "{}", at_final.text());
    assert!(
        at_final.json().get("errors").is_none(),
        "{}",
        at_final.text()
    );

    let at_chunk = send(
        &app,
        Outgoing::post(
            "/scanned",
            form(&[file_part(
                "chunk",
                "c.bin",
                "application/octet-stream",
                b"data",
            )]),
        ),
    )
    .await;
    assert_eq!(at_chunk.status, 500, "{}", at_chunk.text());
    assert!(
        at_chunk.json().get("errors").is_none(),
        "{}",
        at_chunk.text()
    );
}

// ── PAR-043: request-wide limits answer 413 without reading further ──

#[derive(MultipartRequest)]
#[multipart(max_body_bytes = 1024 * 1024)]
#[allow(dead_code)] // extraction always fails
struct CappedBody {
    #[field("file")]
    file: UploadedFile,
}

async fn capped_body(req: Request) -> Response {
    CappedBody::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn a_body_over_its_cap_answers_413_without_reading_further() {
    let app = App::new(Router::new().post("/capped", capped_body));
    let mut chunks = vec![part_head(
        "file",
        Some(("f.bin", "application/octet-stream")),
    )];
    chunks.extend((0..32).map(|_| vec![b'x'; STREAM_CHUNK]));

    let reply = send(&app, Outgoing::post_chunks("/capped", chunks).unfinished()).await;

    assert_eq!(reply.status, 413, "{}", reply.text());
    assert!(reply.json().get("errors").is_none(), "{}", reply.text());
    // The cap is crossed in the fifth content chunk.
    assert!(
        reply.chunks_sent <= 1 + 5 + 6,
        "the client had sent {} chunks",
        reply.chunks_sent
    );
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // extraction always fails
struct ManyParts {
    #[field("n")]
    n: Vec<String>,
}

async fn many_parts(req: Request) -> Response {
    ManyParts::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn too_many_parts_answer_413_without_reading_further() {
    let app = App::new(Router::new().post("/parts", many_parts));
    let ceiling = suprnova::http::upload::global_max_multipart_parts();
    // Every part up to the ceiling, then the head of one more, whose
    // content never comes.
    let mut body: Vec<u8> = (0..ceiling).flat_map(|_| text_part("n", "x")).collect();
    body.extend_from_slice(&part_head("n", None));

    let reply = send(&app, Outgoing::post("/parts", body).unfinished()).await;

    assert_eq!(reply.status, 413, "{}", reply.text());
    assert!(reply.json().get("errors").is_none(), "{}", reply.text());
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // extraction always fails
struct TwoFiles {
    #[field("files[]", max_count = 2)]
    files: Vec<UploadedFile>,
}

async fn two_files(req: Request) -> Response {
    TwoFiles::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn too_many_files_for_max_count_answer_413_without_reading_further() {
    let app = App::new(Router::new().post("/two", two_files));
    let mut body = [
        file_part("files[]", "a.bin", "application/octet-stream", b"a"),
        file_part("files[]", "b.bin", "application/octet-stream", b"b"),
    ]
    .concat();
    // The third file's head, whose content never comes.
    body.extend_from_slice(&part_head(
        "files[]",
        Some(("c.bin", "application/octet-stream")),
    ));

    let reply = send(&app, Outgoing::post("/two", body).unfinished()).await;

    assert_eq!(reply.status, 413, "{}", reply.text());
    assert!(reply.json().get("errors").is_none(), "{}", reply.text());
}

#[derive(MultipartRequest)]
#[multipart(max_body_bytes = 4096)]
#[allow(dead_code)] // extraction always fails
struct Tight {
    #[field("file")]
    file: UploadedFile<MaxSize<4096>>,
}

async fn tight(req: Request) -> Response {
    Tight::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn a_chunk_that_crosses_the_body_cap_and_a_file_limit_answers_413() {
    let app = App::new(Router::new().post("/tight", tight));
    let body = form(&[file_part(
        "file",
        "f.bin",
        "application/octet-stream",
        &[1u8; 64 * 1024],
    )]);

    let reply = send(&app, Outgoing::post("/tight", body)).await;

    assert_eq!(
        reply.status,
        413,
        "the request-wide limit wins: {}",
        reply.text()
    );
    assert!(reply.json().get("errors").is_none(), "{}", reply.text());
}

// ── PAR-042: hook errors, their names, and an empty set ──

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
struct Covers {
    #[field("mode")]
    mode: String,
    #[field("covers[]")]
    cover: Vec<UploadedFile>,
}

impl MultipartRequestHooks for Covers {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        match self.mode.as_str() {
            // The Rust field name and an index: reported under the input
            // name, as extraction errors are.
            "rust-name" => errs.add("cover.1", "This cover is taken."),
            // Already the input name: kept.
            "input-name" => errs.add("covers.1", "This cover is taken."),
            // An empty set is success.
            _ => return Err(errs),
        }
        Err(errs)
    }
}

async fn covers(req: Request) -> Response {
    let form = Covers::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "covers": form.cover.len() })))
}

fn covers_body(mode: &str) -> Vec<u8> {
    form(&[
        text_part("mode", mode),
        file_part("covers[]", "a.bin", "application/octet-stream", b"a"),
        file_part("covers[]", "b.bin", "application/octet-stream", b"b"),
    ])
}

#[tokio::test]
async fn hook_errors_go_under_the_input_names() {
    let app = App::new(Router::new().post("/covers", covers));

    for mode in ["rust-name", "input-name"] {
        let reply = send(&app, Outgoing::post("/covers", covers_body(mode))).await;
        assert_eq!(reply.status, 422, "{mode}: {}", reply.text());
        assert_eq!(
            first_message(&reply, "covers.1"),
            "This cover is taken.",
            "{mode}"
        );
        assert_eq!(errors(&reply).len(), 1, "{mode}: {}", reply.text());
    }
}

#[tokio::test]
async fn an_empty_error_set_from_a_hook_is_success() {
    let app = App::new(Router::new().post("/covers", covers));

    let reply = send(&app, Outgoing::post("/covers", covers_body("none"))).await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["covers"], 2);
}

// ── Inertia: the error comes back as `props.errors` on the form page ──

/// Test stand-in for `SessionMiddleware`: scopes a session slot the test
/// owns into the whole chain, so the flash survives between requests.
struct SeededSessionScope(Arc<Mutex<Option<SessionData>>>);

#[async_trait::async_trait]
impl Middleware for SeededSessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

fn inertia_app(slot: &Arc<Mutex<Option<SessionData>>>) -> App {
    App::with(
        Router::new()
            .post("/gallery", gallery)
            .get("/gallery", |req: Request| async move {
                InertiaResponse::new("Gallery/Create")
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            }),
        MiddlewareRegistry::new()
            .append(SeededSessionScope(slot.clone()))
            .append(InertiaValidationRedirectMiddleware::new()),
    )
}

/// Post `body` as an Inertia form, follow the redirect back, and return the
/// page's `props.errors`.
async fn inertia_round_trip(
    app: &App,
    slot: &Arc<Mutex<Option<SessionData>>>,
    path: &'static str,
    body: Vec<u8>,
) -> Value {
    let posted = send(
        app,
        Outgoing::post(path, body)
            .header("X-Inertia", "true")
            .header("Referer", format!("http://localhost{path}")),
    )
    .await;
    assert_eq!(
        posted.status,
        303,
        "an Inertia form's validation failure redirects back: {}",
        posted.text()
    );
    assert_eq!(
        posted.headers.get("location").map(String::as_str),
        Some(path)
    );

    // `SessionMiddleware` ages the flash at the start of the next request;
    // the test scope does not.
    slot.lock()
        .expect("session slot")
        .as_mut()
        .expect("session")
        .age_flash_data();

    let page = send(app, Outgoing::get(path).header("X-Inertia", "true")).await;
    assert_eq!(page.status, 200, "{}", page.text());
    page.json()["props"]["errors"].clone()
}

#[tokio::test]
async fn an_inertia_form_gets_the_file_error_back_in_props_errors() {
    let slot = suprnova::session::new_session_slot_for_test();
    let app = inertia_app(&slot);

    let errors = inertia_round_trip(&app, &slot, "/gallery", gallery_body(&pdf())).await;

    assert!(
        errors["files.1"].is_string(),
        "props.errors carries `files.1`: {errors}"
    );
}

// ── PAR-042: the stage order ──

/// Each run's stages in the order they ran, keyed by the run id its
/// request carries, so concurrent tests never share a record.
fn stage_log() -> &'static Mutex<HashMap<String, Vec<&'static str>>> {
    static LOG: std::sync::OnceLock<Mutex<HashMap<String, Vec<&'static str>>>> =
        std::sync::OnceLock::new();
    LOG.get_or_init(Default::default)
}

fn record(run: &str, stage: &'static str) {
    stage_log()
        .lock()
        .expect("stage log")
        .entry(run.to_string())
        .or_default()
        .push(stage);
}

fn stages(run: &str) -> Vec<&'static str> {
    stage_log()
        .lock()
        .expect("stage log")
        .get(run)
        .cloned()
        .unwrap_or_default()
}

/// The gate each run's client holds its body behind until `authorize`
/// opens it.
fn gates() -> &'static Mutex<HashMap<String, Arc<Notify>>> {
    static GATES: std::sync::OnceLock<Mutex<HashMap<String, Arc<Notify>>>> =
        std::sync::OnceLock::new();
    GATES.get_or_init(Default::default)
}

/// Records the extraction stage: its `validate_final` runs while the
/// extractor builds the fields. The file's content names the run and
/// whether to refuse the file.
#[derive(Default)]
struct StageProbe;

impl UploadValidator for StageProbe {
    fn validate_final(
        &self,
        sniff: &[u8],
        _size: u64,
        _content_type: Option<&str>,
    ) -> Result<(), FrameworkError> {
        let content = String::from_utf8_lossy(sniff);
        let (run, mode) = content.split_once(';').unwrap_or((&content, ""));
        record(run, "extraction");
        if mode == "invalid" {
            return Err(FrameworkError::invalid_upload(
                suprnova::ValidationMessage::keyed("validation-scan-unreadable")
                    .fallback("The scan is unreadable."),
            ));
        }
        Ok(())
    }
}

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
struct Staged {
    #[field("run")]
    run: String,
    #[field("fail")]
    fail: String,
    #[field("scans[]")]
    scan: Vec<UploadedFile<StageProbe>>,
}

#[async_trait::async_trait]
impl MultipartRequestHooks for Staged {
    fn authorize(req: &Request) -> bool {
        let run = req.header("X-Run").unwrap_or_default().to_string();
        record(&run, "authorize");
        if let Some(gate) = gates().lock().expect("gates").get(&run) {
            gate.notify_one();
        }
        req.header("X-Deny").is_none()
    }

    fn after_validation(&self) -> Result<(), ValidationErrors> {
        record(&self.run, "after_validation");
        let mut errs = ValidationErrors::new();
        if self.fail == "sync" {
            errs.add("scan.0", "Rejected by the sync hook.");
        }
        // Empty unless this run fails here: success.
        Err(errs)
    }

    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        // A real await point, as a database check has.
        tokio::task::yield_now().await;
        record(&self.run, "after_validation_async");
        let mut errs = ValidationErrors::new();
        if self.fail == "async" {
            errs.add("scan.0", "Rejected by the async hook.");
        }
        Err(errs)
    }
}

async fn staged(req: Request) -> Response {
    let form = Staged::from_request(req).await?;
    record(&form.run, "handler");
    Ok(HttpResponse::json(json!({ "scans": form.scan.len() })))
}

fn staged_body(run: &str, fail: &str, scan_mode: &str) -> Vec<u8> {
    form(&[
        text_part("run", run),
        text_part("fail", fail),
        file_part(
            "scans[]",
            "scan.bin",
            "application/octet-stream",
            format!("{run};{scan_mode}").as_bytes(),
        ),
    ])
}

/// Send one staged request whose body the client holds back until
/// `authorize` runs. Were the body read first, the read would wait forever
/// and the request would time out.
async fn staged_run(
    app: &App,
    fail: &str,
    scan_mode: &str,
    deny: bool,
) -> (Reply, Vec<&'static str>) {
    let run = uuid::Uuid::new_v4().simple().to_string();
    let gate = Arc::new(Notify::new());
    gates()
        .lock()
        .expect("gates")
        .insert(run.clone(), gate.clone());
    let mut out = Outgoing::post("/staged", staged_body(&run, fail, scan_mode))
        .header("X-Run", run.clone())
        .gated(gate);
    if deny {
        out = out.header("X-Deny", "1");
    }
    let reply = send(app, out).await;
    gates().lock().expect("gates").remove(&run);
    (reply, stages(&run))
}

#[tokio::test]
async fn the_stages_run_in_order_each_only_after_the_one_before_succeeded() {
    let app = App::new(Router::new().post("/staged", staged));

    let (reply, ran) = staged_run(&app, "none", "ok", false).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(
        ran,
        [
            "authorize",
            "extraction",
            "after_validation",
            "after_validation_async",
            "handler"
        ],
        "authorize first, the async hook after the sync one, the handler last"
    );

    let (reply, ran) = staged_run(&app, "none", "ok", true).await;
    assert_eq!(reply.status, 403, "{}", reply.text());
    assert_eq!(ran, ["authorize"]);

    let (reply, ran) = staged_run(&app, "none", "invalid", false).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(first_message(&reply, "scans.0"), "The scan is unreadable.");
    assert_eq!(ran, ["authorize", "extraction"]);

    let (reply, ran) = staged_run(&app, "sync", "ok", false).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(
        first_message(&reply, "scans.0"),
        "Rejected by the sync hook."
    );
    assert_eq!(ran, ["authorize", "extraction", "after_validation"]);

    let (reply, ran) = staged_run(&app, "async", "ok", false).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(
        first_message(&reply, "scans.0"),
        "Rejected by the async hook."
    );
    assert_eq!(
        ran,
        [
            "authorize",
            "extraction",
            "after_validation",
            "after_validation_async"
        ],
        "an async hook failure never reaches the handler"
    );
}

#[tokio::test]
async fn an_inertia_form_gets_the_async_hook_error_back_in_props_errors() {
    let slot = suprnova::session::new_session_slot_for_test();
    let app = App::with(
        Router::new()
            .post("/staged", staged)
            .get("/staged", |req: Request| async move {
                InertiaResponse::new("Scans/Create")
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            }),
        MiddlewareRegistry::new()
            .append(SeededSessionScope(slot.clone()))
            .append(InertiaValidationRedirectMiddleware::new()),
    );
    let run = uuid::Uuid::new_v4().simple().to_string();

    let errors = inertia_round_trip(&app, &slot, "/staged", staged_body(&run, "async", "ok")).await;

    assert_eq!(errors["scans.0"], "Rejected by the async hook.", "{errors}");
    assert!(
        !stages(&run).contains(&"handler"),
        "the handler never ran: {:?}",
        stages(&run)
    );
}

// ── PAR-043: the values an Inertia or HTML form sends ──

#[derive(MultipartRequest)]
struct Consent {
    #[field("terms")]
    terms: bool,
    #[field("newsletter")]
    newsletter: Option<bool>,
    #[field("flags[]")]
    flags: Vec<bool>,
}

async fn consent(req: Request) -> Response {
    let form = Consent::from_request(req).await?;
    Ok(HttpResponse::json(json!({
        "terms": form.terms,
        "newsletter": form.newsletter,
        "flags": form.flags,
    })))
}

#[tokio::test]
async fn a_bool_field_reads_what_inertia_and_html_forms_send() {
    let app = App::new(Router::new().post("/consent", consent));

    // Inertia sends `1`/`0`; a checked HTML checkbox sends `on`.
    for (sent, read) in [
        ("1", true),
        ("0", false),
        ("true", true),
        ("false", false),
        ("TRUE", true),
        ("False", false),
        ("on", true),
        ("Off", false),
    ] {
        let reply = send(
            &app,
            Outgoing::post("/consent", form(&[text_part("terms", sent)])),
        )
        .await;
        assert_eq!(reply.status, 200, "`{sent}`: {}", reply.text());
        assert_eq!(reply.json()["terms"], read, "`{sent}`");
        // An unchecked checkbox sends nothing: the optional field is absent.
        assert_eq!(reply.json()["newsletter"], Value::Null, "`{sent}`");
    }

    let reply = send(
        &app,
        Outgoing::post(
            "/consent",
            form(&[
                text_part("terms", "1"),
                text_part("newsletter", "on"),
                text_part("flags[]", "1"),
                text_part("flags[]", "0"),
                text_part("flags[]", "ON"),
                text_part("flags[]", "off"),
            ]),
        ),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["newsletter"], true);
    assert_eq!(reply.json()["flags"], json!([true, false, true, false]));

    for sent in ["yes", "2", "onn", " 1"] {
        let reply = send(
            &app,
            Outgoing::post(
                "/consent",
                form(&[text_part("terms", sent), text_part("flags[]", "maybe")]),
            ),
        )
        .await;
        assert_eq!(reply.status, 422, "`{sent}`: {}", reply.text());
        assert_eq!(
            first_message(&reply, "terms"),
            "The terms field must be true or false.",
            "`{sent}`"
        );
        assert_eq!(
            first_message(&reply, "flags.0"),
            "The flags.0 field must be true or false."
        );
    }
}

#[derive(MultipartRequest)]
struct Nullable {
    #[field("count")]
    count: Option<u32>,
    #[field("active")]
    active: Option<bool>,
    #[field("ratio")]
    ratio: Option<f64>,
    #[field("address")]
    address: Option<std::net::IpAddr>,
    #[field("note")]
    note: Option<String>,
    #[field("title")]
    title: String,
    #[field("ids[]")]
    ids: Vec<Option<u32>>,
    #[field("tags[]")]
    tags: Vec<Option<String>>,
}

async fn nullable(req: Request) -> Response {
    let form = Nullable::from_request(req).await?;
    Ok(HttpResponse::json(json!({
        "count": form.count,
        "active": form.active,
        "ratio": form.ratio,
        "address": form.address.map(|a| a.to_string()),
        "note": form.note,
        "title": form.title,
        "ids": form.ids,
        "tags": form.tags,
    })))
}

#[tokio::test]
async fn an_empty_part_is_null_for_an_optional_typed_field() {
    let app = App::new(Router::new().post("/nullable", nullable));

    // What Inertia sends for `null` values: empty text parts.
    let reply = send(
        &app,
        Outgoing::post(
            "/nullable",
            form(&[
                text_part("count", ""),
                text_part("active", ""),
                text_part("ratio", ""),
                text_part("address", ""),
                text_part("note", ""),
                text_part("title", "Holiday"),
                text_part("ids[]", "3"),
                text_part("ids[]", ""),
                text_part("ids[]", "4"),
                text_part("tags[]", "beach"),
                text_part("tags[]", ""),
                text_part("tags[]", "sun"),
            ]),
        ),
    )
    .await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    let body = reply.json();
    assert_eq!(body["count"], Value::Null);
    assert_eq!(body["active"], Value::Null);
    assert_eq!(body["ratio"], Value::Null);
    assert_eq!(body["address"], Value::Null);
    // A `String` is no exception: Laravel's `ConvertEmptyStringsToNull`
    // makes the empty text `null` whatever the field's rules.
    assert_eq!(body["note"], Value::Null);
    assert_eq!(body["title"], "Holiday");
    // A list whose elements may be null keeps a null element in its place,
    // of numbers and of text alike.
    assert_eq!(body["ids"], json!([3, null, 4]));
    assert_eq!(body["tags"], json!(["beach", null, "sun"]));
}

#[derive(MultipartRequest)]
struct RequiredText {
    #[field("title")]
    title: String,
}

async fn required_text(req: Request) -> Response {
    let form = RequiredText::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "title": form.title })))
}

#[tokio::test]
async fn an_empty_part_for_a_required_string_field_is_missing() {
    let app = App::new(Router::new().post("/required-text", required_text));

    // An empty first part, and an empty last part after a value: the last
    // part decides, and empty text is `null`, so `required` fails.
    for parts in [
        vec![text_part("title", "")],
        vec![text_part("title", "Holiday"), text_part("title", "")],
    ] {
        let reply = send(&app, Outgoing::post("/required-text", form(&parts))).await;
        assert_eq!(reply.status, 422, "{}", reply.text());
        assert_eq!(
            first_message(&reply, "title"),
            "The title field is required."
        );
    }
}

#[tokio::test]
async fn a_later_part_carries_the_value_after_an_empty_first_part() {
    let app = App::new(Router::new().post("/nullable", nullable));

    let reply = send(
        &app,
        Outgoing::post(
            "/nullable",
            form(&[
                text_part("title", ""),
                text_part("title", "Holiday"),
                text_part("note", ""),
                text_part("note", "kept"),
                text_part("count", "1"),
                text_part("count", "2"),
            ]),
        ),
    )
    .await;

    assert_eq!(reply.status, 200, "{}", reply.text());
    let body = reply.json();
    // PHP keeps the last value of a name that is not a list.
    assert_eq!(body["title"], "Holiday");
    assert_eq!(body["note"], "kept");
    assert_eq!(body["count"], 2);
}

#[derive(MultipartRequest)]
#[allow(dead_code)] // the assertions are on the failures, never the values
struct RequiredTyped {
    #[field("count")]
    count: u32,
    #[field("active")]
    active: bool,
    #[field("ratio")]
    ratio: f64,
}

async fn required_typed(req: Request) -> Response {
    RequiredTyped::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "ok": true })))
}

#[tokio::test]
async fn an_empty_part_for_a_required_typed_field_is_missing() {
    let app = App::new(Router::new().post("/required", required_typed));

    let reply = send(
        &app,
        Outgoing::post(
            "/required",
            form(&[
                text_part("count", ""),
                text_part("active", ""),
                text_part("ratio", ""),
            ]),
        ),
    )
    .await;

    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(
        first_message(&reply, "count"),
        "The count field is required."
    );
    assert_eq!(
        first_message(&reply, "active"),
        "The active field is required."
    );
    assert_eq!(
        first_message(&reply, "ratio"),
        "The ratio field is required."
    );
}

// ── PAR-043: a text part whose bytes are not UTF-8 ──

/// One field for each key a part that is not UTF-8 can report.
#[derive(MultipartRequest)]
struct Encoded {
    #[field("title")]
    title: String,
    #[field("count")]
    count: u32,
    #[field("tags[]")]
    tags: Vec<String>,
    #[field("ratio")]
    ratio: Option<f64>,
    #[field("active")]
    active: Option<bool>,
    #[field("address")]
    address: Option<std::net::IpAddr>,
    #[field("attachment")]
    attachment: Option<UploadedFile>,
}

async fn encoded(req: Request) -> Response {
    let form = Encoded::from_request(req).await?;
    Ok(HttpResponse::json(json!({
        "title": form.title,
        "count": form.count,
        "tags": form.tags,
        "ratio": form.ratio,
        "active": form.active,
        "address": form.address.map(|a| a.to_string()),
        "attachment": form.attachment.map(|a| a.size),
    })))
}

/// `café` in Latin-1, as a page served in a legacy encoding sends it.
const LATIN1_CAFE: &[u8] = b"caf\xe9";

fn encoded_body() -> Vec<u8> {
    form(&[
        text_part_bytes("title", LATIN1_CAFE),
        text_part_bytes("count", b"\xff7"),
        text_part("tags[]", "fine"),
        text_part_bytes("tags[]", LATIN1_CAFE),
    ])
}

#[tokio::test]
async fn a_text_part_that_is_not_utf8_answers_422_under_its_name() {
    let app = App::new(Router::new().post("/encoded", encoded));

    let reply = TestContainer::scope(async {
        let _catalog = bind_catalog("");
        send(&app, Outgoing::post("/encoded", encoded_body())).await
    })
    .await;

    assert_eq!(
        reply.status,
        422,
        "a part that is not UTF-8 is invalid input: {}",
        reply.text()
    );
    assert_eq!(
        first_message(&reply, "title"),
        "The title field must be a string."
    );
    assert_eq!(
        first_message(&reply, "count"),
        "The count field must be an integer."
    );
    assert_eq!(
        first_message(&reply, "tags.1"),
        "The tags.1 field must be a string."
    );
    assert_eq!(
        errors(&reply).len(),
        3,
        "only the three parts that are not UTF-8 failed: {}",
        reply.text()
    );
}

#[tokio::test]
async fn a_part_that_is_not_utf8_reports_the_key_of_its_fields_type() {
    let body = form(&[
        text_part_bytes("title", LATIN1_CAFE),
        text_part_bytes("count", b"\xff7"),
        text_part("tags[]", "fine"),
        text_part_bytes("tags[]", LATIN1_CAFE),
        text_part_bytes("ratio", b"0.5\xff"),
        text_part_bytes("active", b"\xc0"),
        text_part_bytes("address", b"127.0.0.\xb1"),
        text_part_bytes("attachment", LATIN1_CAFE),
    ]);
    let req = crate::common::request_from_multipart(BOUNDARY, body.into()).await;
    let errors = match Encoded::from_request(req).await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {:?}", other.err()),
    };

    assert_eq!(key(&errors, "title"), "validation-string");
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(key(&errors, "tags.1"), "validation-string");
    assert_eq!(key(&errors, "ratio"), "validation-numeric");
    assert_eq!(key(&errors, "active"), "validation-boolean");
    assert_eq!(key(&errors, "address"), "validation-format");
    assert_eq!(
        key(&errors, "attachment"),
        "validation-file",
        "text where a file belongs"
    );
    assert_eq!(errors.errors.len(), 7, "{errors}");
}

#[tokio::test]
async fn an_inertia_form_gets_a_part_that_is_not_utf8_back_in_props_errors() {
    let slot = suprnova::session::new_session_slot_for_test();
    let app = App::with(
        Router::new()
            .post("/encoded", encoded)
            .get("/encoded", |req: Request| async move {
                InertiaResponse::new("Profile/Edit")
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            }),
        MiddlewareRegistry::new()
            .append(SeededSessionScope(slot.clone()))
            .append(InertiaValidationRedirectMiddleware::new()),
    );

    let errors = inertia_round_trip(&app, &slot, "/encoded", encoded_body()).await;

    for field in ["title", "count", "tags.1"] {
        assert!(
            errors[field].is_string(),
            "props.errors carries `{field}`: {errors}"
        );
    }
}

// ── PAR-043: the streaming check reads only the parts a field takes ──

/// Sized well under every spill threshold a concurrent test may set, so a
/// text part longer than the limit stays a text part.
const SCAN_MAX: usize = 64;

#[derive(MultipartRequest)]
struct Scans {
    #[field("scan")]
    scan: UploadedFile<MaxSize<SCAN_MAX>>,
    #[field("extra")]
    extra: Option<UploadedFile<MaxSize<SCAN_MAX>>>,
    #[field("pages[]")]
    pages: Vec<UploadedFile<MaxSize<SCAN_MAX>>>,
}

async fn scans(req: Request) -> Response {
    let form = Scans::from_request(req).await?;
    Ok(HttpResponse::json(json!({
        "scan": form.scan.size,
        "extra": form.extra.map(|file| file.size),
        "pages": form.pages.iter().map(|file| file.size).collect::<Vec<_>>(),
    })))
}

async fn scans_errors(body: Vec<u8>) -> ValidationErrors {
    let req = crate::common::request_from_multipart(BOUNDARY, body.into()).await;
    match Scans::from_request(req).await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {:?}", other.err()),
    }
}

/// A file part of `len` bytes.
fn sized_file(name: &str, len: usize) -> Vec<u8> {
    file_part(
        name,
        "scan.bin",
        "application/octet-stream",
        &vec![7u8; len],
    )
}

#[tokio::test]
async fn a_text_part_longer_than_a_sized_file_is_not_a_file() {
    let long = "x".repeat(SCAN_MAX * 2);
    let errors = scans_errors(form(&[
        text_part("scan", &long),
        text_part("extra", &long),
        text_part("pages[]", &long),
    ]))
    .await;

    assert_eq!(key(&errors, "scan"), "validation-file");
    assert_eq!(key(&errors, "extra"), "validation-file");
    assert_eq!(key(&errors, "pages.0"), "validation-file");
    assert_eq!(errors.errors.len(), 3, "{errors}");
}

#[tokio::test]
async fn a_later_part_for_a_field_that_holds_one_file_is_ignored_unvalidated() {
    let app = App::new(Router::new().post("/scans", scans));

    // The field takes the first file; the oversized second one is ignored.
    let reply = send(
        &app,
        Outgoing::post(
            "/scans",
            form(&[
                sized_file("scan", SCAN_MAX / 2),
                sized_file("scan", SCAN_MAX * 2),
                sized_file("extra", SCAN_MAX / 2),
                sized_file("extra", SCAN_MAX * 2),
            ]),
        ),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["scan"], SCAN_MAX / 2);
    assert_eq!(reply.json()["extra"], SCAN_MAX / 2);

    // An empty file that has a name is still the file the field takes.
    let reply = send(
        &app,
        Outgoing::post(
            "/scans",
            form(&[
                file_part("scan", "empty.bin", "application/octet-stream", b""),
                sized_file("scan", SCAN_MAX * 2),
            ]),
        ),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["scan"], 0);

    // Text where the file belongs is the part the field takes: the error is
    // that it is not a file, never the size of the ignored file after it.
    let errors = scans_errors(form(&[
        text_part("scan", "not a file"),
        sized_file("scan", SCAN_MAX * 2),
    ]))
    .await;
    assert_eq!(key(&errors, "scan"), "validation-file");
    assert_eq!(errors.errors.len(), 1, "{errors}");
    assert_eq!(errors.errors["scan"].len(), 1, "{errors}");
}

/// The control for the test above: a part the field does take is checked
/// while it streams, whatever came before it.
#[tokio::test]
async fn a_part_the_field_takes_after_a_left_out_one_is_still_checked() {
    for left_out in [
        // Inertia's `null` file.
        text_part("scan", ""),
        // A file input left empty.
        file_part("scan", "", "application/octet-stream", b""),
    ] {
        let errors = scans_errors(form(&[left_out, sized_file("scan", SCAN_MAX * 2)])).await;
        assert_eq!(key(&errors, "scan"), "validation-max-file");
    }

    // Every part of a list is taken, so every part is checked.
    let errors = scans_errors(form(&[
        sized_file("scan", SCAN_MAX / 2),
        sized_file("pages[]", SCAN_MAX / 2),
        sized_file("pages[]", SCAN_MAX * 2),
    ]))
    .await;
    assert_eq!(key(&errors, "pages.1"), "validation-max-file");
}

// ── PAR-043: a text part over the in-memory limit is a request-wide 413 ──

static NOTES_HANDLER_RAN: AtomicBool = AtomicBool::new(false);

#[derive(MultipartRequest)]
// Its own cap, far above the text limit, so only the text limit can answer.
#[multipart(max_body_bytes = 64 * 1024 * 1024)]
struct Notes {
    #[field("count")]
    count: u32,
    #[field("bio")]
    bio: String,
}

async fn notes(req: Request) -> Response {
    let form = Notes::from_request(req).await?;
    NOTES_HANDLER_RAN.store(true, Ordering::SeqCst);
    Ok(HttpResponse::json(
        json!({ "count": form.count, "bio": form.bio.len() }),
    ))
}

#[tokio::test]
async fn a_text_part_over_the_in_memory_limit_answers_413_and_stops_the_read() {
    let app = App::new(Router::new().post("/notes", notes));

    // A field failure first, then a text part that never ends.
    let mut prefix = text_part("count", "three");
    prefix.extend_from_slice(&part_head("bio", None));
    let mut chunks = vec![prefix];
    let prefix_chunks = chunks.len();
    chunks.extend((0..64).map(|_| vec![b'x'; STREAM_CHUNK]));
    // The default in-memory limit is crossed inside the 9th text chunk.
    let crossing =
        prefix_chunks + suprnova::http::upload::DEFAULT_UPLOAD_SPILL_THRESHOLD / STREAM_CHUNK + 1;

    let reply = send(&app, Outgoing::post_chunks("/notes", chunks).unfinished()).await;

    assert_eq!(
        reply.status,
        413,
        "the text limit bounds the whole request: {}",
        reply.text()
    );
    assert!(reply.text().contains("in-memory limit"), "{}", reply.text());
    assert!(
        reply.json().get("errors").is_none(),
        "a request-wide limit wins over the field failure before it: {}",
        reply.text()
    );
    assert!(
        reply.chunks_sent <= crossing + 6,
        "the read stops after the crossing chunk ({crossing}); the client had sent {}",
        reply.chunks_sent
    );
    assert!(!NOTES_HANDLER_RAN.load(Ordering::SeqCst), "the handler ran");
}

// ── PAR-043: the last part decides a text field that holds one value ──

#[tokio::test]
async fn the_last_part_decides_a_text_field_that_holds_one_value() {
    // A required field: only the last part is parsed, so two failing
    // parts report one error.
    let errors = typed_errors(form(&[
        text_part("count", "abc"),
        text_part("count", "xyz"),
    ]))
    .await;
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(errors.errors["count"].len(), 1, "{errors}");

    // An earlier part that fails is replaced by a later one that parses,
    // as PHP keeps the last value of the name.
    let errors = typed_errors(form(&[text_part("count", "abc"), text_part("count", "5")])).await;
    assert!(!errors.errors.contains_key("count"), "{errors}");

    // And a later part that fails replaces one that parsed.
    let errors = typed_errors(form(&[text_part("count", "5"), text_part("count", "abc")])).await;
    assert_eq!(key(&errors, "count"), "validation-integer");

    // An optional field, the same.
    let req = crate::common::request_from_multipart(
        BOUNDARY,
        form(&[
            text_part("count", "abc"),
            text_part("count", "xyz"),
            text_part("title", "t"),
        ])
        .into(),
    )
    .await;
    let errors = match Nullable::from_request(req).await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {:?}", other.err()),
    };
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(errors.errors["count"].len(), 1, "{errors}");
    assert_eq!(errors.errors.len(), 1, "{errors}");
}

// ── Empty text and repeated names: a url-encoded form reads the same ──
//
// `FormRequest` reads a url-encoded body by Laravel's rules too, so a form
// posted either way gives a handler the same values.

/// The text fields of `Nullable`, read from a url-encoded body.
#[derive(Debug, serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
struct UrlencodedText {
    title: String,
    note: Option<String>,
    count: Option<u32>,
}

async fn urlencoded(body: &str) -> Result<UrlencodedText, FrameworkError> {
    let req =
        crate::common::request_with_body("/", "application/x-www-form-urlencoded", body.as_bytes())
            .await;
    UrlencodedText::from_request(req).await
}

#[tokio::test]
async fn an_empty_urlencoded_value_for_a_required_string_field_is_missing() {
    // An empty first value, an empty last value after a value, and no
    // value at all: each is a validation failure under the field's name.
    for body in ["title=&note=kept", "title=Holiday&title=", "note=kept"] {
        let errors = match urlencoded(body).await {
            Err(FrameworkError::Validation(errors)) => errors,
            other => panic!("`{body}`: expected validation errors, got {other:?}"),
        };
        assert_eq!(key(&errors, "title"), "validation-required", "`{body}`");
        assert_eq!(
            errors.errors["title"][0].fallback, "The title field is required.",
            "`{body}`"
        );
        assert_eq!(errors.errors.len(), 1, "`{body}`: {errors}");
    }
}

#[tokio::test]
async fn an_empty_urlencoded_value_is_null_for_an_optional_field() {
    let form = match urlencoded("title=Holiday&note=&count=").await {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(form.title, "Holiday");
    assert_eq!(form.note, None);
    assert_eq!(form.count, None);
}

#[tokio::test]
async fn a_repeated_urlencoded_name_keeps_its_last_value() {
    let form = match urlencoded("title=&title=Holiday&note=&note=kept&count=1&count=2").await {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(form.title, "Holiday");
    assert_eq!(form.note.as_deref(), Some("kept"));
    assert_eq!(form.count, Some(2));
}

/// Every kind of field a url-encoded form request can fail on.
#[derive(Debug, serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
struct UrlencodedTyped {
    title: String,
    count: u32,
    ratio: f64,
    active: bool,
    note: Option<String>,
    #[serde(default)]
    remember: bool,
}

async fn urlencoded_typed(body: &str) -> Result<UrlencodedTyped, FrameworkError> {
    let req =
        crate::common::request_with_body("/", "application/x-www-form-urlencoded", body.as_bytes())
            .await;
    UrlencodedTyped::from_request(req).await
}

#[tokio::test]
async fn a_urlencoded_form_reports_every_field_that_does_not_parse_under_its_name() {
    let errors = match urlencoded_typed("count=abc&ratio=half&active=maybe&note=x").await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    };
    assert_eq!(key(&errors, "title"), "validation-required");
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(key(&errors, "ratio"), "validation-numeric");
    assert_eq!(key(&errors, "active"), "validation-boolean");
    // An optional field and a `#[serde(default)]` one are not required.
    assert_eq!(errors.errors.len(), 4, "{errors}");

    // Every required field missing at once is reported at once.
    let errors = match urlencoded_typed("note=x").await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    };
    for field in ["title", "count", "ratio", "active"] {
        assert_eq!(key(&errors, field), "validation-required", "{field}");
    }
    assert_eq!(errors.errors.len(), 4, "{errors}");

    // A form that parses still extracts.
    let form = match urlencoded_typed("title=t&count=3&ratio=0.5&active=true").await {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    assert_eq!((form.count, form.active, form.remember), (3, true, false));
    assert_eq!((form.ratio, form.note), (0.5, None));
}

#[derive(Debug, serde::Deserialize, validator::Validate)]
struct JsonAddress {
    street: String,
}

/// Every kind of field a JSON form request can fail on, nested ones too.
#[derive(Debug, serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
struct JsonTyped {
    title: String,
    count: u32,
    ratio: f64,
    active: bool,
    tags: Vec<u32>,
    note: Option<String>,
    address: JsonAddress,
}

async fn json_typed(body: &str) -> Result<JsonTyped, FrameworkError> {
    let req = crate::common::request_with_body("/", "application/json", body.as_bytes()).await;
    JsonTyped::from_request(req).await
}

#[tokio::test]
async fn a_json_form_reports_every_field_that_does_not_parse_under_its_name() {
    let errors = match json_typed(
        r#"{"title": null, "count": "abc", "ratio": "x", "active": 1,
            "tags": [1, "two"], "note": 5, "address": {}}"#,
    )
    .await
    {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    };
    // `null` is a missing value, as Laravel's `required` reads it.
    assert_eq!(key(&errors, "title"), "validation-required");
    assert_eq!(key(&errors, "count"), "validation-integer");
    assert_eq!(key(&errors, "ratio"), "validation-numeric");
    assert_eq!(key(&errors, "active"), "validation-boolean");
    assert_eq!(key(&errors, "tags.1"), "validation-integer");
    assert_eq!(key(&errors, "note"), "validation-string");
    assert_eq!(key(&errors, "address.street"), "validation-required");
    assert_eq!(errors.errors.len(), 7, "{errors}");

    // Fields left out entirely, every one of them at once.
    let errors = match json_typed(r#"{"tags": []}"#).await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    };
    for field in ["title", "count", "ratio", "active", "address"] {
        assert_eq!(key(&errors, field), "validation-required", "{field}");
    }
    assert_eq!(errors.errors.len(), 5, "{errors}");

    // A body that is not JSON at all is no field's failure.
    match json_typed(r#"{"title": "#).await {
        Err(FrameworkError::Domain { status_code, .. }) => assert_eq!(status_code, 422),
        other => panic!("expected a parse error, got {other:?}"),
    }

    // A body that parses still extracts.
    let form = match json_typed(
        r#"{"title": "t", "count": 3, "ratio": 1, "active": true, "tags": [4],
            "address": {"street": "Main"}}"#,
    )
    .await
    {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    assert_eq!((form.count, form.tags, form.note), (3, vec![4], None));
    assert_eq!((form.ratio, form.active), (1.0, true));
    assert_eq!(form.address.street, "Main");
}

async fn typed_form_handler(req: Request) -> Response {
    let form = UrlencodedTyped::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "title": form.title })))
}

async fn typed_json_handler(req: Request) -> Response {
    let form = JsonTyped::from_request(req).await?;
    Ok(HttpResponse::json(json!({ "title": form.title })))
}

#[tokio::test]
async fn an_inertia_form_request_gets_its_field_errors_back_in_props_errors() {
    let slot = Arc::new(Mutex::new(Some(SessionData::new(
        "sess-form-request".into(),
        "csrf".into(),
    ))));
    let page = |req: Request| async move {
        InertiaResponse::new("Profile/Edit")
            .resolve(&req)
            .await
            .map_err(HttpResponse::from)
    };
    let app = App::with(
        Router::new()
            .post("/profile", typed_form_handler)
            .get("/profile", page)
            .post("/settings", typed_json_handler)
            .get("/settings", page),
        MiddlewareRegistry::new()
            .append(SeededSessionScope(slot.clone()))
            .append(InertiaValidationRedirectMiddleware::new()),
    );

    for (path, content_type, body) in [
        (
            "/profile",
            "application/x-www-form-urlencoded",
            "title=&count=abc&ratio=1&active=true",
        ),
        (
            "/settings",
            "application/json",
            r#"{"count": "abc", "ratio": 1, "active": true, "tags": [], "address": {"street": "x"}}"#,
        ),
    ] {
        let posted = send(
            &app,
            Outgoing::post(path, body.as_bytes().to_vec())
                .content_type(content_type)
                .header("X-Inertia", "true")
                .header("Referer", format!("http://localhost{path}")),
        )
        .await;
        assert_eq!(posted.status, 303, "{content_type}: {}", posted.text());
        assert_eq!(
            posted.headers.get("location").map(String::as_str),
            Some(path)
        );

        slot.lock()
            .expect("session slot")
            .as_mut()
            .expect("session")
            .age_flash_data();
        let shown = send(&app, Outgoing::get(path).header("X-Inertia", "true")).await;
        assert_eq!(shown.status, 200, "{}", shown.text());
        let errors = shown.json()["props"]["errors"].clone();
        assert_eq!(
            errors["title"], "The title field is required.",
            "{content_type}: {errors}"
        );
        assert_eq!(
            errors["count"], "The count field must be an integer.",
            "{content_type}: {errors}"
        );
    }
}

#[tokio::test]
async fn a_precognitive_form_request_reports_the_fields_it_was_asked_about() {
    let app = App::new(Router::new().post("/profile", typed_form_handler));
    let ask = |only: &'static str| {
        Outgoing::post("/profile", b"count=abc&ratio=1&active=true".to_vec())
            .content_type("application/x-www-form-urlencoded")
            .header("Precognition", "true")
            .header("Precognition-Validate-Only", only)
    };

    // Asked about `count`, which does not parse: only its error comes back.
    let reply = send(&app, ask("count")).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(
        first_message(&reply, "count"),
        "The count field must be an integer."
    );
    assert!(!errors(&reply).contains_key("title"), "{}", reply.text());

    // Asked about `ratio`, which parses: the form still cannot be checked
    // while other fields do not parse, so it is not reported valid, and
    // the fields in the way are named.
    let reply = send(&app, ask("ratio")).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(
        first_message(&reply, "title"),
        "The title field is required."
    );
}

// ── An empty value is a present key holding null, as Laravel keeps it ──

#[tokio::test]
async fn an_empty_urlencoded_value_is_a_present_key_with_null() {
    let req = crate::common::request_with_body(
        "/",
        "application/x-www-form-urlencoded",
        b"name=Ada&bio=&tags[]=a&tags[]=&tags[]=b",
    )
    .await;
    let form: Value = match req.form().await {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    // A cleared field is told apart from one never sent.
    assert_eq!(
        form,
        json!({ "name": "Ada", "bio": null, "tags": ["a", null, "b"] })
    );
}

/// A list whose elements may be null, and one whose elements may not.
#[derive(Debug, serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
struct UrlencodedLists {
    #[serde(default)]
    maybe: Vec<Option<u32>>,
    #[serde(default)]
    ids: Vec<u32>,
}

#[tokio::test]
async fn a_null_list_element_is_none_or_a_missing_element() {
    let read = |body: &'static str| async move {
        let req = crate::common::request_with_body(
            "/",
            "application/x-www-form-urlencoded",
            body.as_bytes(),
        )
        .await;
        UrlencodedLists::from_request(req).await
    };
    let form = match read("maybe[]=3&maybe[]=&maybe[]=4").await {
        Ok(form) => form,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(form.maybe, vec![Some(3), None, Some(4)]);
    assert!(form.ids.is_empty());

    // An element that cannot be null is required, under its own index.
    let errors = match read("ids[]=3&ids[]=&ids[]=4").await {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected validation errors, got {other:?}"),
    };
    assert_eq!(key(&errors, "ids.1"), "validation-required");
    assert_eq!(errors.errors.len(), 1, "{errors}");
}

#[tokio::test]
async fn a_urlencoded_bool_reads_what_forms_send() {
    for (sent, read) in [
        ("1", true),
        ("0", false),
        ("true", true),
        ("FALSE", false),
        ("on", true),
        ("Off", false),
    ] {
        let body = format!("title=t&count=1&ratio=1&active={sent}");
        match urlencoded_typed(&body).await {
            Ok(form) => assert_eq!(form.active, read, "`{sent}`"),
            Err(error) => panic!("`{sent}`: {error}"),
        }
    }
    for sent in ["yes", "2", " 1"] {
        let body = format!("title=t&count=1&ratio=1&active={sent}");
        let errors = match urlencoded_typed(&body).await {
            Err(FrameworkError::Validation(errors)) => errors,
            other => panic!("`{sent}`: expected validation errors, got {other:?}"),
        };
        assert_eq!(key(&errors, "active"), "validation-boolean", "`{sent}`");
    }
}

#[tokio::test]
async fn an_empty_part_of_a_list_that_cannot_hold_null_is_a_missing_element() {
    let errors = typed_errors(form(&[
        text_part("count", "1"),
        text_part("ratio", "1"),
        text_part("active", "1"),
        text_part("address", "127.0.0.1"),
        text_part("tags[]", "1"),
        text_part("tags[]", ""),
        text_part("tags[]", "3"),
    ]))
    .await;
    assert_eq!(key(&errors, "tags.1"), "validation-required");
    assert_eq!(errors.errors.len(), 1, "{errors}");
}
