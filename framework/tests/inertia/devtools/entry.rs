//! PAR-072: one entry per recorded request, with its lineage, request type,
//! request and response, route and render source, and the DevTools headers
//! and first-visit tag on the response.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use suprnova::http::text;
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};
use suprnova::{
    FromRequest, HttpResponse, Inertia, InertiaConfig, InertiaResponse, MiddlewareRegistry,
    MultipartRequest, Redirect, Request, Response, Router,
};

use super::{
    app_env, client, devtools, entry_ids, entry_of, inertia, raw_request, raw_request_on_continue,
    raw_send, read_entry,
};
use crate::protocol_harness::serve;

/// The line `render` builds its response on, beside the response.
fn render(component: &'static str) -> (u32, InertiaResponse) {
    (line!(), InertiaResponse::new(component))
}

/// A named handler, so the route has an action to show.
async fn show_user(req: Request) -> Response {
    let pages = req.header("x-test-pages").map(PathBuf::from);
    let (_, response) = render("Users/Show");
    let response = match pages {
        Some(pages) => {
            response.with_config(InertiaConfig::new().development(true).pages_dir(pages))
        }
        None => response,
    };
    response
        .with("user", json!({"id": 7, "name": "Ada"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}

fn router() -> Router {
    Router::new()
        .get("/users/{id}", show_user)
        .name("users.show")
        .get("/page", |req: Request| async move {
            InertiaResponse::new("Home")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .post("/page", |req: Request| async move {
            InertiaResponse::new("Home")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/text", |_req: Request| async { text("plain text") })
        .get("/json", |_req: Request| async {
            Ok(HttpResponse::json(json!({"ok": true, "items": [1, 2]})))
        })
        .get("/html", |_req: Request| async {
            Ok(HttpResponse::html("<html><body><p>plain</p></body></html>"))
        })
        .get("/png", |_req: Request| async {
            Ok(HttpResponse::bytes_body(
                vec![0x89, b'P', b'N', b'G'],
                "image/png",
            ))
        })
        .get("/big", |_req: Request| async { text("x".repeat(256_001)) })
        .post("/save", |_req: Request| async {
            let response: Response = Redirect::to("/page").into();
            response
        })
        .get("/away", |_req: Request| async {
            Ok(Inertia::location("https://billing.example/portal"))
        })
        .post("/upload", |_req: Request| async { text("uploaded") })
        .post("/album", album)
        .post("/echo", |req: Request| async move {
            let (_, bytes) = req.body_bytes().await?;
            Ok(HttpResponse::bytes_body(bytes, "text/plain"))
        })
        .post("/echo-large", |req: Request| async move {
            // A cap of its own, above the global one, as a `FormRequest`
            // with `max_body_bytes` has.
            let (_, bytes) = req.body_bytes_with_cap(1024 * 1024).await?;
            Ok(HttpResponse::bytes_body(bytes, "text/plain"))
        })
        .get("/upper", |req: Request| {
            first_visit_document(req, "<html><BODY><p>x</p></BODY></html>")
        })
        .get("/spaced", |req: Request| {
            first_visit_document(req, "<html><Body><p>x</p></Body ></html>")
        })
        .get("/bare", |req: Request| {
            first_visit_document(req, "<p>x</p>")
        })
        .get("/sized", |req: Request| async move {
            let page = InertiaResponse::new("Home")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)?;
            let length = page.body().len();
            Ok(page.header("Content-Length", length.to_string()))
        })
        .into()
}

/// An upload the extractor reads; the test decides its authorization.
#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
struct Album {
    #[field("title")]
    title: String,
    #[field("photo")]
    photo: UploadedFile,
}

/// What `Album::authorize` saw, by the request's `X-Run`: whether any of
/// the body had been read before it.
fn read_before_authorize() -> &'static Mutex<HashMap<String, bool>> {
    static SEEN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    SEEN.get_or_init(Mutex::default)
}

impl MultipartRequestHooks for Album {
    /// Notes whether the body was read before it ran, and denies a request
    /// that carries `X-Deny`.
    fn authorize(req: &Request) -> bool {
        if let Some(run) = req.header("X-Run") {
            read_before_authorize()
                .lock()
                .unwrap()
                .insert(run.to_string(), req.cached_body().is_some());
        }
        req.header("X-Deny").is_none()
    }
}

async fn album(req: Request) -> Response {
    let album = Album::from_request(req).await?;
    text(format!("{} {}", album.title, album.photo.size))
}

/// A first visit answered with `document`: the page renders, then the
/// handler answers with the document as an application's root template
/// would lay it out.
async fn first_visit_document(req: Request, document: &'static str) -> Response {
    InertiaResponse::new("Home")
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(HttpResponse::html(document))
}

fn is_ulid(id: &str) -> bool {
    id.len() == 26
        && id
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b.to_ascii_uppercase()))
}

#[tokio::test]
async fn indt_every_recorded_request_gets_its_own_ulid() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let first = client.get("/page").send().await;
    let second = client.get("/page").send().await;
    let first_id = first.header("x-inertia-devtools-id").unwrap().to_string();
    let second_id = second.header("x-inertia-devtools-id").unwrap().to_string();
    assert!(is_ulid(&first_id), "{first_id}");
    assert!(is_ulid(&second_id), "{second_id}");
    assert_ne!(first_id, second_id);
    assert!(
        first_id < second_id,
        "a later entry sorts after an earlier one"
    );
    assert_eq!(
        entry_ids(dir.path()),
        vec![first_id.clone(), second_id.clone()]
    );
    assert_eq!(
        read_entry(dir.path(), &first_id)["__meta"]["id"],
        first_id.as_str()
    );
}

/// The request type an entry of `path` sent with `headers` records.
async fn request_type(dir: &Path, path: &str, headers: &[(&str, &str)]) -> String {
    let client = client(router(), devtools(dir));
    let mut request = client.get(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await;
    entry_of(dir, &response)["__meta"]["requestType"]
        .as_str()
        .unwrap()
        .to_string()
}

/// One request-type case: the path, the request's headers, and the type
/// its entry records.
type Case = (
    &'static str,
    Vec<(&'static str, &'static str)>,
    &'static str,
);

#[tokio::test]
async fn indt_the_request_type_follows_laravels_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let inertia = ("X-Inertia", "true");
    let cases: Vec<Case> = vec![
        (
            "/page",
            vec![("Precognition", "true"), inertia],
            "precognition",
        ),
        ("/page", vec![], "initial"),
        ("/text", vec![], "http"),
        ("/page", vec![inertia], "navigate"),
        (
            "/page",
            vec![
                inertia,
                ("X-Inertia-Partial-Component", "Home"),
                ("X-Inertia-Partial-Data", "greeting"),
                ("X-Inertia-Devtools-Deferred", "true"),
            ],
            "deferred",
        ),
        (
            "/page",
            vec![inertia, ("X-Inertia-Devtools-Poll", "true")],
            "poll",
        ),
        (
            "/page",
            vec![
                inertia,
                ("X-Inertia-Partial-Component", "Home"),
                ("X-Inertia-Partial-Data", "greeting"),
            ],
            "partial",
        ),
        ("/page", vec![inertia, ("Purpose", "prefetch")], "prefetch"),
        (
            "/page",
            vec![inertia, ("Sec-Purpose", "prefetch")],
            "prefetch",
        ),
    ];
    for (path, headers, expected) in cases {
        assert_eq!(
            request_type(dir, path, &headers).await,
            expected,
            "{path} with {headers:?}"
        );
    }
}

#[tokio::test]
async fn indt_an_entry_carries_the_request_response_route_and_render_source() {
    let dir = tempfile::tempdir().unwrap();
    let pages = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pages.path().join("Users")).unwrap();
    std::fs::write(pages.path().join("Users/Show.svelte"), "<script></script>").unwrap();
    let client = client(router(), devtools(dir.path()));

    let response = client
        .get("/users/7?tab=profile")
        .inertia()
        .header("X-Inertia-Devtools-Tab", "tab-1")
        .header("X-Inertia-Devtools-Visit", "visit-9")
        .header("X-Test-Pages", pages.path().display().to_string())
        .send()
        .await;
    response.assert_ok();
    let entry = entry_of(dir.path(), &response);
    let meta = &entry["__meta"];
    assert_eq!(meta["method"], "GET");
    assert!(
        meta["url"]
            .as_str()
            .unwrap()
            .ends_with("/users/7?tab=profile"),
        "{}",
        meta["url"]
    );
    assert_eq!(meta["status"], 200);
    assert_eq!(meta["component"], "Users/Show");
    assert_eq!(meta["requestType"], "navigate");
    assert_eq!(meta["tabUuid"], "tab-1");
    assert_eq!(meta["visitId"], "visit-9");
    assert_eq!(meta["batchId"], Value::Null);
    assert_eq!(meta["redirectLocation"], Value::Null);
    assert!(meta["serverTimingMs"].as_f64().unwrap() >= 0.0);
    assert!(meta["utime"].as_f64().unwrap() > 1_600_000_000.0);
    let timestamp = meta["timestamp"].as_str().unwrap();
    assert_eq!(timestamp.len(), 24, "{timestamp}");
    assert!(
        timestamp.ends_with('Z') && timestamp.as_bytes()[10] == b'T',
        "{timestamp}"
    );

    let http = &entry["http"];
    assert_eq!(http["requestHeaders"]["x-inertia-devtools-tab"], "tab-1");
    assert_eq!(http["requestHeaders"]["x-inertia"], "true");
    assert_eq!(http["responseHeaders"]["x-inertia"], "true");
    assert!(http["responseHeaders"]["x-inertia-devtools-id"].is_string());
    assert_eq!(
        http["requestBody"]["status"], "present",
        "a GET's query is its input"
    );
    assert_eq!(http["requestBody"]["value"], json!({"tab": "profile"}));
    assert_eq!(http["responseBody"]["status"], "present");
    assert_eq!(http["responseBody"]["value"]["component"], "Users/Show");
    assert_eq!(
        http["responseBody"]["value"]["props"]["user"]["name"],
        "Ada"
    );

    assert_eq!(entry["route"]["name"], "users.show");
    assert_eq!(entry["route"]["uri"], "/users/{id}");
    assert_eq!(entry["route"]["method"], "GET");
    assert!(
        entry["route"]["action"]
            .as_str()
            .unwrap()
            .ends_with("show_user"),
        "{}",
        entry["route"]["action"]
    );

    let (line, _) = render("Users/Show");
    assert!(
        entry["renderSource"]["file"]
            .as_str()
            .unwrap()
            .ends_with("entry.rs"),
        "{}",
        entry["renderSource"]
    );
    assert_eq!(entry["renderSource"]["line"], line);
    let component_path = entry["componentPath"].as_str().unwrap();
    assert!(
        component_path.ends_with("Users/Show.svelte"),
        "{component_path}"
    );
    assert!(entry["props"].is_object());
    assert!(entry["propValues"].is_object());
}

#[tokio::test]
async fn indt_a_route_defined_page_names_its_route_as_the_render_source() {
    let dir = tempfile::tempdir().unwrap();
    let router: Router = Router::new()
        .inertia("/about", "About", json!({"team": 4}))
        .into();
    // The line of the `.inertia(` call, which `#[track_caller]` reports
    // however the chain is laid out.
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/inertia/devtools/entry.rs"),
    )
    .unwrap();
    let defined_at = source
        .lines()
        .position(|line| line.contains(".inertia(\"/about\", \"About\""))
        .unwrap()
        + 1;
    let response = client(router, devtools(dir.path()))
        .get("/about")
        .inertia()
        .send()
        .await;
    let entry = entry_of(dir.path(), &response);
    assert_eq!(entry["__meta"]["component"], "About");
    assert!(
        entry["renderSource"]["file"]
            .as_str()
            .unwrap()
            .ends_with("entry.rs")
    );
    assert_eq!(entry["renderSource"]["line"], defined_at);
}

#[tokio::test]
async fn indt_a_non_inertia_write_keeps_no_body_and_an_inertia_write_keeps_its_input() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let plain = client
        .post("/save")
        .json(&json!({"title": "Hello"}))
        .send()
        .await;
    assert_eq!(
        entry_of(dir.path(), &plain)["http"]["requestBody"],
        json!({"status": "omitted", "reason": "non-inertia-request"})
    );

    let json_post = client
        .post("/page")
        .inertia()
        .json(&json!({"title": "Hello", "tags": ["a"]}))
        .send()
        .await;
    assert_eq!(
        entry_of(dir.path(), &json_post)["http"]["requestBody"],
        json!({"status": "present", "value": {"title": "Hello", "tags": ["a"]}})
    );

    let form_post = client
        .post("/page")
        .inertia()
        .form(&[("title", "Hello"), ("user[name]", "Ada")])
        .send()
        .await;
    assert_eq!(
        entry_of(dir.path(), &form_post)["http"]["requestBody"],
        json!({"status": "present", "value": {"title": "Hello", "user": {"name": "Ada"}}})
    );

    let empty = client.get("/page").inertia().send().await;
    assert_eq!(
        entry_of(dir.path(), &empty)["http"]["requestBody"],
        json!({"status": "empty"})
    );
}

#[tokio::test]
async fn indt_a_json_body_records_what_it_parses_to_and_malformed_json_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    for (body, recorded) in [
        (json!([]), json!([])),
        (json!({}), json!({})),
        (Value::Null, Value::Null),
    ] {
        let response = client.post("/page").inertia().json(&body).send().await;
        response.assert_ok();
        assert_eq!(
            entry_of(dir.path(), &response)["http"]["requestBody"],
            json!({"status": "present", "value": recorded}),
            "the JSON body {body}"
        );
    }

    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let malformed = raw_send(
        addr,
        "POST",
        "/upload",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"application/json"),
        ],
        br#"{"a":"#.to_vec(),
    )
    .await;
    assert_eq!(
        read_entry(dir.path(), &malformed.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "present", "value": "{\"a\":"}),
        "JSON that does not parse is kept as its text"
    );
}

/// A multipart body with a title, a photo and a part that is not text.
const ALBUM_BODY: &[u8] = b"--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHoliday\r\n--XYZ\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"beach.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8\xff\xe0JPEG\r\n--XYZ\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\n\xff\xfe\r\n--XYZ--\r\n";

#[tokio::test]
async fn indt_an_upload_is_summarized_and_a_text_body_kept_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let multipart: &[(&str, &[u8])] = &[
        ("X-Inertia", b"true"),
        ("X-Run", b"summarized"),
        ("Content-Type", b"multipart/form-data; boundary=XYZ"),
    ];

    // The extractor parsed the body, after authorization, and handed the
    // recorder what it read.
    let reply = raw_send(addr, "POST", "/album", multipart, ALBUM_BODY.to_vec()).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, "Holiday 8");
    assert_eq!(
        read_before_authorize().lock().unwrap().get("summarized"),
        Some(&false),
        "authorization ran before any of the body was read"
    );
    let entry = read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"]);
    assert_eq!(
        entry["http"]["requestBody"],
        json!({"status": "present", "value": {
            "title": "Holiday",
            "photo": {"name": "beach.jpg", "size": 8, "mimeType": "image/jpeg"},
            "note": "[UNSERIALIZABLE]",
        }})
    );

    // A handler that never extracts its upload leaves it unread.
    let ignored = raw_send(addr, "POST", "/upload", multipart, ALBUM_BODY.to_vec()).await;
    assert_eq!(ignored.status, 200);
    assert_eq!(
        read_entry(dir.path(), &ignored.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "omitted", "reason": "not-read", "size": ALBUM_BODY.len()})
    );

    let text = raw_send(
        addr,
        "POST",
        "/upload",
        &[("X-Inertia", b"true"), ("Content-Type", b"text/plain")],
        b"just words".to_vec(),
    )
    .await;
    assert_eq!(
        read_entry(dir.path(), &text.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "present", "value": "just words"})
    );

    let binary = raw_send(
        addr,
        "POST",
        "/upload",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"application/octet-stream"),
        ],
        vec![0xff, 0xfe, 0x00],
    )
    .await;
    assert_eq!(
        read_entry(dir.path(), &binary.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "omitted", "reason": "binary"})
    );
}

#[tokio::test]
async fn indt_an_upload_lists_every_part_of_a_repeated_name() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let mut body = Vec::new();
    let mut part = |headers: &str, data: &[u8]| {
        body.extend_from_slice(b"--XYZ\r\nContent-Disposition: form-data; ");
        body.extend_from_slice(headers.as_bytes());
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    };
    part("name=\"title\"", b"Holiday");
    part(
        "name=\"photo\"; filename=\"cover.jpg\"\r\nContent-Type: image/jpeg",
        b"cover",
    );
    for (file, data) in [("a.jpg", &b"a"[..]), ("b.png", b"bb"), ("c.gif", b"ccc")] {
        part(
            &format!("name=\"photos[]\"; filename=\"{file}\"\r\nContent-Type: image/x-test"),
            data,
        );
    }
    part("name=\"tags\"", b"sea");
    part("name=\"tags\"", b"sun");
    body.extend_from_slice(b"--XYZ--\r\n");

    let reply = raw_send(
        addr,
        "POST",
        "/album",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"multipart/form-data; boundary=XYZ"),
        ],
        body,
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let file =
        |name: &str, size: usize| json!({"name": name, "size": size, "mimeType": "image/x-test"});
    assert_eq!(
        read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "present", "value": {
            "title": "Holiday",
            "photo": {"name": "cover.jpg", "size": 5, "mimeType": "image/jpeg"},
            "photos[]": [file("a.jpg", 1), file("b.png", 2), file("c.gif", 3)],
            "tags": ["sea", "sun"],
        }})
    );

    // A name that ends in `[]` is a list even with one part.
    let single = b"--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHoliday\r\n--XYZ\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"cover.jpg\"\r\nContent-Type: image/jpeg\r\n\r\ncover\r\n--XYZ\r\nContent-Disposition: form-data; name=\"photos[]\"; filename=\"a.jpg\"\r\nContent-Type: image/x-test\r\n\r\na\r\n--XYZ--\r\n";
    let reply = raw_send(
        addr,
        "POST",
        "/album",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"multipart/form-data; boundary=XYZ"),
        ],
        single.to_vec(),
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(
        read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"])["http"]["requestBody"]["value"]
            ["photos[]"],
        json!([file("a.jpg", 1)])
    );
}

#[tokio::test]
async fn indt_a_denied_upload_is_refused_before_any_byte_of_its_body_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    // `Expect: 100-continue`: the server asks for the body the first time
    // anything polls it, so a reply with no `100 Continue` before it means
    // no byte of the body was read.
    let head = format!(
        "POST /album HTTP/1.1\r\nHost: localhost\r\nX-Inertia: true\r\nX-Deny: yes\r\n\
         X-Run: denied\r\nContent-Type: multipart/form-data; boundary=XYZ\r\n\
         Content-Length: {}\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
        ALBUM_BODY.len()
    );
    let (reply, asked_for_body) = raw_request_on_continue(addr, &head, ALBUM_BODY).await;
    assert_eq!(reply.status, 403);
    assert!(
        !asked_for_body,
        "something read the body before authorization"
    );
    assert_eq!(
        read_before_authorize().lock().unwrap().get("denied"),
        Some(&false),
        "the hook saw a body already read"
    );
    assert_eq!(
        read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "omitted", "reason": "not-read", "size": ALBUM_BODY.len()})
    );
}

#[tokio::test]
async fn indt_an_upload_whose_parse_fails_records_unparsed_and_never_its_text() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    // The last part never ends: the closing boundary is missing.
    let body = b"--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHoliday\r\n--XYZ\r\nContent-Disposition: form-data; name=\"password\"\r\n\r\nhunter2\r\n".to_vec();
    let reply = raw_send(
        addr,
        "POST",
        "/album",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"multipart/form-data; boundary=XYZ"),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(reply.status, 400, "{}", reply.body);
    let id = &reply.headers["x-inertia-devtools-id"];
    assert_eq!(
        read_entry(dir.path(), id)["http"]["requestBody"],
        json!({"status": "omitted", "reason": "unparsed", "size": body.len()})
    );
    let stored = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    assert!(!stored.contains("hunter2"), "the entry kept the raw body");
}

/// The head of an Inertia `POST` to `path` that declares a body of
/// `length` bytes.
fn inertia_post_head(path: &str, length: usize) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Inertia: true\r\n\
         Content-Type: text/plain\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    )
}

#[tokio::test]
async fn indt_a_body_that_fails_to_arrive_leaves_the_answer_to_the_handler() {
    let dir = tempfile::tempdir().unwrap();
    let recording = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;

    // Ten bytes declared and none sent before the client closes: the
    // first poll of the body fails.
    let ignored = raw_request(recording, &inertia_post_head("/upload", 10), b"", true).await;
    assert_eq!(ignored.status, 200, "the handler ignores its body");
    assert_eq!(ignored.body, "uploaded");
    assert_eq!(
        read_entry(dir.path(), &ignored.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "omitted", "reason": "unreadable"})
    );

    // A handler that reads its body meets the failure it would meet with
    // recording off.
    let quiet = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(
            suprnova::DevToolsConfig::new().enabled(false),
        ))),
    )
    .await;
    let read = raw_request(recording, &inertia_post_head("/echo", 10), b"abc", true).await;
    let unrecorded = raw_request(quiet, &inertia_post_head("/echo", 10), b"abc", true).await;
    assert_eq!(read.status, 500);
    assert_eq!(read.status, unrecorded.status);
}

/// The head of a chunked Inertia `POST` to `path`.
fn chunked_inertia_post_head(path: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Inertia: true\r\n\
         Content-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )
}

/// `data` as one chunk of a chunked body, then the last chunk.
fn one_chunk(data: &[u8]) -> Vec<u8> {
    let mut body = format!("{:x}\r\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n0\r\n\r\n");
    body
}

#[tokio::test]
async fn indt_the_development_error_page_keeps_the_devtools_headers() {
    // Debug mode on and a public root, under the environment lock: the
    // development error page replaces a 5xx, and the root adds the base
    // path header.
    let _lock = crate::env_lock::lock_env_async().await;
    let _env = crate::env_snapshot::EnvSnapshot::capture(&["APP_DEBUG", "APP_URL"]);
    crate::env_snapshot::set_env("APP_DEBUG", Some("true"));
    crate::env_snapshot::set_env("APP_URL", Some("http://localhost/billing"));
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;

    // A handler that reads a body that never arrives answers 500.
    let reply = raw_request(addr, &inertia_post_head("/echo", 10), b"abc", true).await;
    assert_eq!(reply.status, 500);
    assert!(
        reply.headers["content-type"].starts_with("text/html"),
        "the development error page: {:?}",
        reply.headers
    );
    let id = reply
        .headers
        .get("x-inertia-devtools-id")
        .unwrap_or_else(|| panic!("no X-Inertia-Devtools-Id on {:?}", reply.headers));
    assert_eq!(read_entry(dir.path(), id)["__meta"]["id"], id.as_str());
    assert_eq!(
        reply.headers.get("x-inertia-devtools-parent-out"),
        Some(id),
        "a request with no parent is its own"
    );
    assert_eq!(
        reply
            .headers
            .get("x-inertia-devtools-base-path")
            .map(String::as_str),
        Some("/billing")
    );
}

#[tokio::test]
async fn indt_a_request_body_longer_than_a_response_body_may_be_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let text = "x".repeat(300_000);
    let response = client
        .post("/page")
        .inertia()
        .json(&json!({"text": text}))
        .send()
        .await;
    response.assert_ok();
    assert_eq!(
        entry_of(dir.path(), &response)["http"]["requestBody"],
        json!({"status": "present", "value": {"text": text}})
    );
}

#[tokio::test]
async fn indt_a_chunked_body_is_recorded_and_still_reaches_the_handler() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let reply = raw_request(
        addr,
        &chunked_inertia_post_head("/echo"),
        b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
        false,
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, "hello world", "the handler read the whole body");
    assert_eq!(
        read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "present", "value": "hello world"})
    );
}

#[tokio::test]
async fn indt_a_body_over_the_request_body_cap_is_too_large_and_the_handler_answers() {
    // The cap is one value for the whole process; this test lowers it.
    if crate::own_process_async::delegate(
        module_path!(),
        "indt_a_body_over_the_request_body_cap_is_too_large_and_the_handler_answers",
    )
    .await
    {
        return;
    }
    suprnova::http::body::set_global_max_request_body_bytes(1024);
    let dir = tempfile::tempdir().unwrap();
    let too_large = json!({"status": "omitted", "reason": "too-large"});
    let body = "y".repeat(2048);

    // A declared length over the cap: the handler refuses the body itself,
    // and one with a larger cap of its own reads it whole.
    let client = client(router(), devtools(dir.path()));
    let refused = client.post("/echo").inertia().json(&body).send().await;
    assert_eq!(refused.status(), 413);
    assert_eq!(
        entry_of(dir.path(), &refused)["http"]["requestBody"],
        too_large
    );
    let taken = client
        .post("/echo-large")
        .inertia()
        .json(&body)
        .send()
        .await;
    taken.assert_ok();
    assert_eq!(taken.body_text().len(), body.len() + 2, "the JSON string");
    assert_eq!(
        entry_of(dir.path(), &taken)["http"]["requestBody"],
        too_large
    );

    // A chunked body: the recorder reads past the cap before it knows, and
    // the handler still gets the whole stream.
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let chunked = one_chunk(body.as_bytes());
    let refused = raw_request(addr, &chunked_inertia_post_head("/echo"), &chunked, false).await;
    assert_eq!(refused.status, 413);
    assert_eq!(
        read_entry(dir.path(), &refused.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        too_large
    );
    let taken = raw_request(
        addr,
        &chunked_inertia_post_head("/echo-large"),
        &chunked,
        false,
    )
    .await;
    assert_eq!(taken.status, 200);
    assert_eq!(taken.body, body, "the handler read the whole stream");
    assert_eq!(
        read_entry(dir.path(), &taken.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        too_large
    );
}

#[tokio::test]
async fn indt_the_response_body_is_the_page_the_text_or_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let body_of = |response: &suprnova::testing::TestResponse| {
        entry_of(dir.path(), response)["http"]["responseBody"].clone()
    };

    let page = client.get("/page").inertia().send().await;
    let page_body = body_of(&page);
    assert_eq!(page_body["status"], "present");
    assert_eq!(
        page_body["value"],
        page.json(),
        "the page object the client got"
    );

    assert_eq!(
        body_of(&client.get("/json").send().await),
        json!({"status": "present", "value": {"ok": true, "items": [1, 2]}})
    );
    assert_eq!(
        body_of(&client.get("/text").send().await),
        json!({"status": "present", "value": "plain text"})
    );
    assert_eq!(
        body_of(&client.get("/png").send().await),
        json!({"status": "omitted", "reason": "non-textual"})
    );
    assert_eq!(
        body_of(&client.get("/big").send().await),
        json!({"status": "omitted", "reason": "too-large"})
    );
}

#[tokio::test]
async fn indt_a_redirect_records_where_it_goes() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let redirect = client.post("/save").inertia().send().await;
    assert_eq!(redirect.status(), 302);
    assert_eq!(
        entry_of(dir.path(), &redirect)["__meta"]["redirectLocation"],
        "/page"
    );

    let away = client.get("/away").inertia().send().await;
    assert_eq!(away.status(), 409);
    assert_eq!(
        entry_of(dir.path(), &away)["__meta"]["redirectLocation"],
        "https://billing.example/portal"
    );
}

#[tokio::test]
async fn indt_every_recorded_response_carries_the_id_and_lineage_headers() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let visit = client
        .get("/page")
        .inertia()
        .header("X-Inertia-Devtools-Parent", "p")
        .send()
        .await;
    let id = visit.header("x-inertia-devtools-id").unwrap();
    assert_eq!(visit.header("x-inertia-devtools-parent-out"), Some("p"));
    assert_eq!(
        visit.header("x-inertia-devtools-base-path"),
        None,
        "at the host root"
    );
    assert_eq!(read_entry(dir.path(), id)["__meta"]["batchId"], "p");

    let first = client
        .get("/page")
        .header("X-Inertia-Devtools-Parent", "p")
        .send()
        .await;
    let first_id = first.header("x-inertia-devtools-id").unwrap();
    assert_eq!(
        first.header("x-inertia-devtools-parent-out"),
        Some(first_id),
        "a request that is not an Inertia visit starts its own batch"
    );
    assert_eq!(
        read_entry(dir.path(), first_id)["__meta"]["batchId"],
        Value::Null
    );

    let prefetch = client
        .get("/page")
        .inertia()
        .header("X-Inertia-Devtools-Parent", "p")
        .header("Purpose", "prefetch")
        .send()
        .await;
    assert_eq!(
        prefetch.header("x-inertia-devtools-parent-out"),
        prefetch.header("x-inertia-devtools-id"),
        "a prefetch is its own parent"
    );

    let plain = client.get("/text").send().await;
    assert!(plain.header("x-inertia-devtools-id").is_some());
    assert!(plain.header("x-inertia-devtools-parent-out").is_some());
}

#[tokio::test]
async fn indt_a_first_visit_document_carries_the_id_tag_and_an_inertia_visit_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let first = client.get("/page").send().await;
    first.assert_ok();
    let id = first.header("x-inertia-devtools-id").unwrap();
    let tag = format!(
        "<script data-inertia-devtools-id type=\"application/json\">\"{id}\"</script></body>"
    );
    assert!(first.body_text().contains(&tag), "{}", first.body_text());

    let visit = client.get("/page").inertia().send().await;
    assert!(!visit.body_text().contains("data-inertia-devtools-id"));
    assert_eq!(
        visit.json()["component"],
        "Home",
        "the JSON page is untouched"
    );

    let plain = client.get("/html").send().await;
    assert!(plain.header("x-inertia-devtools-id").is_some());
    assert_eq!(
        plain.body_text(),
        "<html><body><p>plain</p></body></html>",
        "a page that is not an Inertia page gets no tag"
    );
}

#[tokio::test]
async fn indt_the_id_tag_finds_a_closing_body_tag_in_any_case_or_ends_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let tag = |id: &str| {
        format!("<script data-inertia-devtools-id type=\"application/json\">\"{id}\"</script>")
    };

    let upper = client.get("/upper").send().await;
    upper.assert_ok();
    let id = upper.header("x-inertia-devtools-id").unwrap();
    assert_eq!(
        upper.body_text(),
        format!("<html><BODY><p>x</p>{}</BODY></html>", tag(id))
    );

    let spaced = client.get("/spaced").send().await;
    let id = spaced.header("x-inertia-devtools-id").unwrap();
    assert_eq!(
        spaced.body_text(),
        format!("<html><Body><p>x</p>{}</Body ></html>", tag(id))
    );

    let bare = client.get("/bare").send().await;
    let id = bare.header("x-inertia-devtools-id").unwrap();
    assert_eq!(
        bare.body_text(),
        format!("<p>x</p>{}", tag(id)),
        "a document with no closing body tag gets the tag at its end"
    );
}

#[tokio::test]
async fn indt_a_tagged_first_visit_never_carries_the_length_of_the_untagged_page() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    // The handler sets the length of the page it resolved; the tag makes
    // the document longer, so that length no longer holds.
    let first = client.get("/sized").send().await;
    first.assert_ok();
    let id = first.header("x-inertia-devtools-id").unwrap();
    let body = first.body_text();
    let tagged = format!(
        "<script data-inertia-devtools-id type=\"application/json\">\"{id}\"</script></body>"
    );
    assert!(body.contains(&tagged), "{body}");
    assert!(
        body.trim_end().ends_with("</html>"),
        "the whole document: {body}"
    );
    if let Some(length) = first.header("content-length") {
        assert_eq!(
            length,
            body.len().to_string(),
            "the length of what was sent"
        );
    }
}

#[tokio::test]
async fn indt_under_a_public_root_the_base_path_rides_the_header_and_the_tag() {
    let dir = tempfile::tempdir().unwrap();
    let _env = app_env("local").await;
    crate::env_snapshot::set_env("APP_URL", Some("http://localhost/billing"));
    let client = client(router(), devtools(dir.path()));

    let first = client.get("/page").send().await;
    first.assert_ok();
    assert_eq!(
        first.header("x-inertia-devtools-base-path"),
        Some("/billing")
    );
    assert!(
        first.body_text().contains(
            "<script data-inertia-devtools-id data-inertia-devtools-base-path=\"/billing\" type=\"application/json\">"
        ),
        "{}",
        first.body_text()
    );
}
