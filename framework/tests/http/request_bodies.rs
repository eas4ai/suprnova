//! Request bodies the Inertia client sends (PAR-055).
//!
//! The client sends nested data with bracketed and indexed names:
//! `filters[status]=x` and `tags[]=a` in a `GET` query string, `user[name]`
//! in a url-encoded body, and `photos[0]` in a multipart body. Each test
//! drives `handle_request` over an in-memory HTTP/1.1 connection, so the
//! router, the extractor and the error rendering all run as they do for a
//! real client, and asserts the nested data a handler reads.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::MaxSize;
use suprnova::{FormRequest, HttpResponse, MiddlewareRegistry, Request, Router, handle_request};

/// How long a test waits for a response before it calls the server stuck.
const WAIT: Duration = Duration::from_secs(20);

const FORM: &str = "application/x-www-form-urlencoded";

/// What the server answered.
struct Reply {
    status: u16,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!(
                "the body is not JSON ({error}): {}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The fields a validation failure's `errors` names, sorted.
    fn error_fields(&self) -> Vec<String> {
        let body = self.json();
        let errors = body["errors"]
            .as_object()
            .unwrap_or_else(|| panic!("no `errors` in {body}"));
        let mut fields: Vec<String> = errors.keys().cloned().collect();
        fields.sort();
        fields
    }
}

/// Send one request through `handle_request` on `router`.
async fn send(
    router: impl Into<Router>,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Reply {
    let router = Arc::new(router.into());
    let middleware = Arc::new(MiddlewareRegistry::new());
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let router = router.clone();
            let middleware = middleware.clone();
            async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });

    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client_io))
            .await
            .expect("the client handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let mut request = hyper::Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "localhost")
        .header("content-length", body.len());
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    let request = request
        .body(Full::new(Bytes::from(body)))
        .expect("a request");
    let response = tokio::time::timeout(WAIT, sender.send_request(request))
        .await
        .expect("the server answered in time")
        .expect("a response");
    let status = response.status().as_u16();
    let body = tokio::time::timeout(WAIT, response.into_body().collect())
        .await
        .expect("the body arrived in time")
        .expect("the body collects")
        .to_bytes();
    Reply { status, body }
}

/// A route that answers with the query string as `query_into` reads it.
fn query_echo() -> Router {
    Router::new()
        .get("/items", |req: Request| async move {
            let query: Value = req.query_into()?;
            Ok(HttpResponse::json(query))
        })
        .into()
}

/// Read `query` into a `serde_json::Value` through a real request.
async fn query_value(query: &str) -> Value {
    let reply = send(
        query_echo(),
        "GET",
        &format!("/items?{query}"),
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(reply.status, 200, "`{query}`: {}", reply.text());
    reply.json()
}

// ── The query string ──

#[derive(Debug, Deserialize, PartialEq)]
struct Listing {
    filters: Filters,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Filters {
    status: String,
}

#[tokio::test]
async fn inp_a_get_query_decodes_bracketed_names_into_an_object_and_a_list() {
    // The client writes the brackets as they are; a browser address bar
    // may percent-encode them. Both read the same.
    for query in [
        "filters[status]=x&tags[]=a&tags[]=b",
        "filters%5Bstatus%5D=x&tags%5B%5D=a&tags%5B%5D=b",
    ] {
        assert_eq!(
            query_value(query).await,
            json!({ "filters": { "status": "x" }, "tags": ["a", "b"] }),
            "{query}"
        );
    }

    let router: Router = Router::new()
        .get("/items", |req: Request| async move {
            let listing: Listing = req.query_into()?;
            Ok(HttpResponse::json(json!({
                "status": listing.filters.status,
                "tags": listing.tags,
            })))
        })
        .into();
    let reply = send(
        router,
        "GET",
        "/items?filters[status]=x&tags[]=a&tags[]=b",
        None,
        Vec::new(),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json(), json!({ "status": "x", "tags": ["a", "b"] }));
}

#[derive(Debug, Deserialize)]
struct Photos {
    photos: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Ids {
    ids: Vec<u32>,
}

#[tokio::test]
async fn inp_indexed_names_decode_in_index_order() {
    assert_eq!(
        query_value("photos[2]=c&photos[0]=a&photos[1]=b").await,
        json!({ "photos": ["a", "b", "c"] })
    );
    // A gap leaves no hole: the elements keep their order.
    assert_eq!(
        query_value("ids[5]=x&ids[1]=y").await,
        json!({ "ids": ["y", "x"] })
    );

    let photos: Photos = Request::for_test("GET", "/p?photos[1]=b&photos[0]=a")
        .query_into()
        .expect("the photos read");
    assert_eq!(photos.photos, ["a", "b"]);

    // An element that does not parse is named by the index it was sent
    // under, as Laravel names `ids.3`.
    let router: Router = Router::new()
        .get("/ids", |req: Request| async move {
            let ids: Ids = req.query_into()?;
            Ok(HttpResponse::json(json!({ "ids": ids.ids })))
        })
        .into();
    let reply = send(router, "GET", "/ids?ids[3]=x&ids[0]=1", None, Vec::new()).await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["ids.3"]);
}

#[tokio::test]
async fn inp_bracketed_names_read_as_php_parses_them() {
    let deep = |levels: usize| format!("a{}=1", "[b]".repeat(levels));
    for (query, expected) in [
        // Text after a closing bracket that opens no other is dropped.
        ("a[b]c=1".to_string(), json!({ "a": { "b": "1" } })),
        // A bracket that never closes is part of a plain name.
        ("a[x=1".to_string(), json!({ "a[x": "1" })),
        // A name with nothing before its first bracket, or no name at
        // all, is no variable.
        ("[x]=1&=2&y=3".to_string(), json!({ "y": "3" })),
        // `[ ]` appends as `[]` does.
        ("a[%20]=1&a[]=2".to_string(), json!({ "a": ["1", "2"] })),
        // A later name overwrites an earlier one at the same place.
        ("a=1&a[b]=2".to_string(), json!({ "a": { "b": "2" } })),
        ("a[b]=2&a=1".to_string(), json!({ "a": "1" })),
        // `[]` takes the index after the highest one sent so far.
        (
            "a[]=x&a[5]=y&a[]=z".to_string(),
            json!({ "a": ["x", "y", "z"] }),
        ),
        // A key that is no integer makes an object; `[]` still counts.
        (
            "a[k]=1&a[]=2".to_string(),
            json!({ "a": { "k": "1", "0": "2" } }),
        ),
        ("a[01]=x".to_string(), json!({ "a": { "01": "x" } })),
        // An empty value is null at every depth.
        (
            "user[name]=&user[tags][]=".to_string(),
            json!({ "user": { "name": null, "tags": [null] } }),
        ),
        // 64 levels read; a 65th drops the name and what was read under it.
        (
            deep(64),
            serde_json::from_str(&format!(
                "{{\"a\":{}\"1\"{}}}",
                "{\"b\":".repeat(64),
                "}".repeat(64)
            ))
            .expect("the expected value"),
        ),
        (format!("a[x]=1&{}", deep(65)), json!({})),
    ] {
        assert_eq!(query_value(&query).await, expected, "{query}");
    }
}

// ── A url-encoded body ──

#[derive(Debug, Deserialize, Validate)]
struct Profile {
    #[validate(nested)]
    user: User,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, Validate)]
struct User {
    #[validate(length(min = 2))]
    name: String,
    #[validate(email)]
    email: String,
}

impl FormRequest for Profile {}

/// A route that reads a `Profile` form request and echoes it.
fn profile_route() -> Router {
    Router::new()
        .post("/profile", |req: Request| async move {
            let profile = Profile::extract(req).await?;
            Ok(HttpResponse::json(json!({
                "name": profile.user.name,
                "email": profile.user.email,
                "tags": profile.tags,
            })))
        })
        .into()
}

#[tokio::test]
async fn inp_a_form_request_reads_nested_names_from_a_urlencoded_body() {
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(FORM),
        b"user[name]=Ada&user[email]=ada%40example.com&tags[]=a&tags[]=b".to_vec(),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(
        reply.json(),
        json!({ "name": "Ada", "email": "ada@example.com", "tags": ["a", "b"] })
    );

    // The nested rules run, and their failures are named by the path.
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(FORM),
        b"user[name]=A&user[email]=nope&tags[]=a".to_vec(),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["user.email", "user.name"]);

    // A nested field left out is required under its path.
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(FORM),
        b"user[email]=ada%40example.com".to_vec(),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["tags", "user.name"]);
}

#[tokio::test]
async fn inp_request_input_reads_nested_names_from_a_urlencoded_body() {
    let router: Router = Router::new()
        .post("/echo", |req: Request| async move {
            let input: Value = req.input().await?;
            Ok(HttpResponse::json(input))
        })
        .into();
    let reply = send(
        router,
        "POST",
        "/echo",
        Some(FORM),
        b"user[name]=Ada&photos[1]=b&photos[0]=a".to_vec(),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(
        reply.json(),
        json!({ "user": { "name": "Ada" }, "photos": ["a", "b"] })
    );
}

// ── A multipart body ──

const BOUNDARY: &str = "inpboundary";

fn multipart() -> String {
    format!("multipart/form-data; boundary={BOUNDARY}")
}

/// A multipart body of `parts`: a name, a file name for a file part, and
/// the bytes.
fn multipart_body(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
    crate::common::build_multipart_body(BOUNDARY, parts).to_vec()
}

#[tokio::test]
async fn inp_a_form_request_reads_a_multipart_body_with_bracketed_and_indexed_names() {
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(&multipart()),
        multipart_body(&[
            ("user[name]", None, b"Ada"),
            ("tags[1]", None, b"b"),
            ("user[email]", None, b"ada@example.com"),
            ("tags[0]", None, b"a"),
        ]),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(
        reply.json(),
        json!({ "name": "Ada", "email": "ada@example.com", "tags": ["a", "b"] })
    );

    // The rules run on the nested data, named by the path.
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(&multipart()),
        multipart_body(&[
            ("user[name]", None, b"A"),
            ("user[email]", None, b"nope"),
            ("tags[]", None, b"a"),
        ]),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["user.email", "user.name"]);
}

/// A middleware that reads the whole body first, as the rate limiter's
/// body-keyed limits do.
struct Buffering;

#[async_trait::async_trait]
impl suprnova::middleware::Middleware for Buffering {
    async fn handle(
        &self,
        request: Request,
        next: suprnova::middleware::Next,
    ) -> suprnova::Response {
        let request = request
            .buffer_body(1 << 20)
            .await
            .map_err(HttpResponse::from)?;
        next(request).await
    }
}

#[tokio::test]
async fn inp_request_input_reads_a_multipart_body_a_middleware_buffered() {
    // The multipart reader took a streaming body only and answered a
    // buffered one with a 400 naming a framework bug; a form behind a
    // middleware that read the body is an ordinary request (PAR-055).
    let router: Router = Router::new()
        .group("/buffered", |r| {
            r.post("/echo", |req: Request| async move {
                let input: Value = req.input().await?;
                Ok(HttpResponse::json(input))
            })
        })
        .middleware(Buffering)
        .into();
    let reply = send(
        router,
        "POST",
        "/buffered/echo",
        Some(&multipart()),
        multipart_body(&[("user[name]", None, b"Ada"), ("photos[0]", None, b"a")]),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(
        reply.json(),
        json!({ "user": { "name": "Ada" }, "photos": ["a"] })
    );
}

#[tokio::test]
async fn inp_a_multipart_part_that_is_not_text_fails_under_its_path() {
    // A file where text belongs, and text that is not UTF-8.
    let reply = send(
        profile_route(),
        "POST",
        "/profile",
        Some(&multipart()),
        multipart_body(&[
            ("user[name]", Some("name.txt"), b"Ada"),
            ("user[email]", None, b"ada@example.com"),
            ("tags[0]", None, b"a"),
            ("tags[1]", None, &[0xff, 0xfe]),
        ]),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["tags.1", "user.name"]);
    let body = reply.json();
    assert_eq!(
        body["errors"]["user.name"][0],
        "The user.name field must be a string."
    );
}

#[tokio::test]
async fn inp_request_input_reads_a_multipart_body() {
    let router: Router = Router::new()
        .post("/echo", |req: Request| async move {
            let input: Value = req.input().await?;
            Ok(HttpResponse::json(input))
        })
        .into();
    let reply = send(
        router,
        "POST",
        "/echo",
        Some(&multipart()),
        multipart_body(&[
            ("user[name]", None, b"Ada"),
            ("photos[0]", Some("a.png"), b"png bytes"),
            ("tags[]", None, b"a"),
            ("tags[]", None, b""),
        ]),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    // A file holds no text: a `serde_json::Value` reads `null` in its
    // place, as Laravel's `input()` reads a file field.
    assert_eq!(
        reply.json(),
        json!({ "user": { "name": "Ada" }, "photos": [null], "tags": ["a", null] })
    );
}

#[derive(Debug, Deserialize, Validate)]
struct Note {
    text: String,
}

impl FormRequest for Note {
    fn max_body_bytes() -> usize {
        256
    }
}

fn note_route() -> Router {
    Router::new()
        .post("/note", |req: Request| async move {
            let note = Note::extract(req).await?;
            Ok(HttpResponse::json(json!({ "text": note.text })))
        })
        .into()
}

#[tokio::test]
async fn inp_a_multipart_form_request_keeps_its_body_cap() {
    let small = multipart_body(&[("text", None, b"hi")]);
    assert!(small.len() <= 256, "{}", small.len());
    let reply = send(note_route(), "POST", "/note", Some(&multipart()), small).await;
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json(), json!({ "text": "hi" }));

    let reply = send(
        note_route(),
        "POST",
        "/note",
        Some(&multipart()),
        multipart_body(&[("text", None, &[b'x'; 512])]),
    )
    .await;
    assert_eq!(reply.status, 413, "{}", reply.text());
}

// ── Files in a form request ──

#[suprnova::request]
struct Album {
    #[validate(length(min = 1))]
    title: String,
    photos: Vec<UploadedFile>,
    cover: Option<UploadedFile<MaxSize<16>>>,
}

/// A route that reads an `Album` and echoes its files' names and bytes.
fn album_route() -> Router {
    Router::new()
        .post("/album", |req: Request| async move {
            let album = Album::extract(req).await?;
            let mut photos = Vec::new();
            for photo in &album.photos {
                let bytes = photo.bytes().await?;
                photos.push(json!({
                    "name": photo.file_name,
                    "body": String::from_utf8_lossy(&bytes),
                }));
            }
            Ok(HttpResponse::json(json!({
                "title": album.title,
                "photos": photos,
                "cover": album.cover.as_ref().map(|cover| cover.file_name.clone()),
            })))
        })
        .into()
}

#[tokio::test]
async fn inp_a_form_request_takes_files_from_bracketed_and_indexed_names() {
    // Inertia sends a list of files with indexes by default, and with
    // brackets when an application asks for them.
    for (first, second) in [("photos[1]", "photos[0]"), ("photos[]", "photos[]")] {
        let (first_body, second_body): (&[u8], &[u8]) = if first == "photos[1]" {
            (b"second", b"first")
        } else {
            (b"first", b"second")
        };
        let (first_name, second_name) = if first == "photos[1]" {
            ("b.txt", "a.txt")
        } else {
            ("a.txt", "b.txt")
        };
        let reply = send(
            album_route(),
            "POST",
            "/album",
            Some(&multipart()),
            multipart_body(&[
                ("title", None, b"Trip"),
                (first, Some(first_name), first_body),
                (second, Some(second_name), second_body),
                ("cover", Some("c.png"), b"cover"),
            ]),
        )
        .await;
        assert_eq!(reply.status, 200, "{first}: {}", reply.text());
        assert_eq!(
            reply.json(),
            json!({
                "title": "Trip",
                "photos": [
                    { "name": "a.txt", "body": "first" },
                    { "name": "b.txt", "body": "second" },
                ],
                "cover": "c.png",
            }),
            "{first}"
        );
    }

    // Inertia sends a `null` file as an empty part.
    let reply = send(
        album_route(),
        "POST",
        "/album",
        Some(&multipart()),
        multipart_body(&[("title", None, b"Trip"), ("cover", None, b"")]),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["photos"]);
}

#[tokio::test]
async fn inp_a_file_that_fails_is_named_by_its_path() {
    let reply = send(
        album_route(),
        "POST",
        "/album",
        Some(&multipart()),
        multipart_body(&[
            ("title", None, b"Trip"),
            ("photos[0]", Some("a.txt"), b"first"),
            ("photos[1]", None, b"not a file"),
            ("cover", Some("c.png"), b"more than sixteen bytes"),
        ]),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["cover", "photos.1"]);
    let body = reply.json();
    assert_eq!(
        body["errors"]["photos.1"][0],
        "The photos.1 field must be a file."
    );
    let cover = body["errors"]["cover"][0]
        .as_str()
        .expect("a message for the cover");
    assert!(cover.contains("kilobytes"), "{cover}");
}

#[tokio::test]
async fn inp_a_file_field_reads_only_a_multipart_body() {
    // Text where a file belongs, url-encoded.
    let reply = send(
        album_route(),
        "POST",
        "/album",
        Some(FORM),
        b"title=Trip&photos[]=a".to_vec(),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
    assert_eq!(reply.error_fields(), ["photos.0"]);

    // And in JSON, which carries no file.
    let reply = send(
        album_route(),
        "POST",
        "/album",
        Some("application/json"),
        br#"{"title": "Trip", "photos": ["a"]}"#.to_vec(),
    )
    .await;
    assert_eq!(reply.status, 422, "{}", reply.text());
}
