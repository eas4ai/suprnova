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
