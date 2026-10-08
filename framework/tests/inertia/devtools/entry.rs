//! PAR-072: one entry per recorded request, with its lineage, request type,
//! request and response, route and render source, and the DevTools headers
//! and first-visit tag on the response.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use suprnova::http::text;
use suprnova::{
    HttpResponse, Inertia, InertiaConfig, InertiaResponse, MiddlewareRegistry, Redirect, Request,
    Response, Router,
};

use super::{app_env, client, devtools, entry_ids, entry_of, inertia, raw_send, read_entry};
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
        Some(pages) => response.with_config(InertiaConfig::new().development(true).pages_dir(pages)),
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
            Ok(HttpResponse::bytes_body(vec![0x89, b'P', b'N', b'G'], "image/png"))
        })
        .get("/big", |_req: Request| async { text("x".repeat(256_001)) })
        .post("/save", |_req: Request| async {
            let response: Response = Redirect::to("/page").into();
            response
        })
        .get("/away", |_req: Request| async { Ok(Inertia::location("https://billing.example/portal")) })
        .post("/upload", |_req: Request| async { text("uploaded") })
        .into()
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
    assert!(first_id < second_id, "a later entry sorts after an earlier one");
    assert_eq!(entry_ids(dir.path()), vec![first_id.clone(), second_id.clone()]);
    assert_eq!(read_entry(dir.path(), &first_id)["__meta"]["id"], first_id.as_str());
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

#[tokio::test]
async fn indt_the_request_type_follows_laravels_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let inertia = ("X-Inertia", "true");
    let cases: Vec<(&str, Vec<(&str, &str)>, &str)> = vec![
        ("/page", vec![("Precognition", "true"), inertia], "precognition"),
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
        ("/page", vec![inertia, ("X-Inertia-Devtools-Poll", "true")], "poll"),
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
        ("/page", vec![inertia, ("Sec-Purpose", "prefetch")], "prefetch"),
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
        meta["url"].as_str().unwrap().ends_with("/users/7?tab=profile"),
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
    assert!(timestamp.ends_with('Z') && timestamp.as_bytes()[10] == b'T', "{timestamp}");

    let http = &entry["http"];
    assert_eq!(http["requestHeaders"]["x-inertia-devtools-tab"], "tab-1");
    assert_eq!(http["requestHeaders"]["x-inertia"], "true");
    assert_eq!(http["responseHeaders"]["x-inertia"], "true");
    assert!(http["responseHeaders"]["x-inertia-devtools-id"].is_string());
    assert_eq!(http["requestBody"]["status"], "present", "a GET's query is its input");
    assert_eq!(http["requestBody"]["value"], json!({"tab": "profile"}));
    assert_eq!(http["responseBody"]["status"], "present");
    assert_eq!(http["responseBody"]["value"]["component"], "Users/Show");
    assert_eq!(http["responseBody"]["value"]["props"]["user"]["name"], "Ada");

    assert_eq!(entry["route"]["name"], "users.show");
    assert_eq!(entry["route"]["uri"], "/users/{id}");
    assert_eq!(entry["route"]["method"], "GET");
    assert!(
        entry["route"]["action"].as_str().unwrap().ends_with("show_user"),
        "{}",
        entry["route"]["action"]
    );

    let (line, _) = render("Users/Show");
    assert!(
        entry["renderSource"]["file"].as_str().unwrap().ends_with("entry.rs"),
        "{}",
        entry["renderSource"]
    );
    assert_eq!(entry["renderSource"]["line"], line);
    let component_path = entry["componentPath"].as_str().unwrap();
    assert!(component_path.ends_with("Users/Show.svelte"), "{component_path}");
    assert!(entry["props"].is_object());
    assert!(entry["propValues"].is_object());
}

#[tokio::test]
async fn indt_a_route_defined_page_names_its_route_as_the_render_source() {
    let dir = tempfile::tempdir().unwrap();
    let defined_at = line!() + 1;
    let router: Router = Router::new().inertia("/about", "About", json!({"team": 4})).into();
    let response = client(router, devtools(dir.path())).get("/about").inertia().send().await;
    let entry = entry_of(dir.path(), &response);
    assert_eq!(entry["__meta"]["component"], "About");
    assert!(entry["renderSource"]["file"].as_str().unwrap().ends_with("entry.rs"));
    assert_eq!(entry["renderSource"]["line"], defined_at);
}

#[tokio::test]
async fn indt_a_non_inertia_write_keeps_no_body_and_an_inertia_write_keeps_its_input() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let plain = client.post("/save").json(&json!({"title": "Hello"})).send().await;
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
async fn indt_an_upload_is_summarized_and_a_text_body_kept_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let body = b"--XYZ\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHoliday\r\n--XYZ\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"beach.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n\xff\xd8\xff\xe0JPEG\r\n--XYZ--\r\n".to_vec();
    let reply = raw_send(
        addr,
        "POST",
        "/upload",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"multipart/form-data; boundary=XYZ"),
        ],
        body,
    )
    .await;
    assert_eq!(reply.status, 200);
    let entry = read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"]);
    assert_eq!(
        entry["http"]["requestBody"],
        json!({"status": "present", "value": {
            "title": "Holiday",
            "photo": {"name": "beach.jpg", "size": 8, "mimeType": "image/jpeg"},
        }})
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
        &[("X-Inertia", b"true"), ("Content-Type", b"application/octet-stream")],
        vec![0xff, 0xfe, 0x00],
    )
    .await;
    assert_eq!(
        read_entry(dir.path(), &binary.headers["x-inertia-devtools-id"])["http"]["requestBody"],
        json!({"status": "omitted", "reason": "binary"})
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
    assert_eq!(page_body["value"], page.json(), "the page object the client got");

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
    assert_eq!(entry_of(dir.path(), &redirect)["__meta"]["redirectLocation"], "/page");

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
    assert_eq!(visit.header("x-inertia-devtools-base-path"), None, "at the host root");
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
    assert_eq!(read_entry(dir.path(), first_id)["__meta"]["batchId"], Value::Null);

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
    let tag = format!("<script data-inertia-devtools-id type=\"application/json\">\"{id}\"</script></body>");
    assert!(first.body_text().contains(&tag), "{}", first.body_text());

    let visit = client.get("/page").inertia().send().await;
    assert!(!visit.body_text().contains("data-inertia-devtools-id"));
    assert_eq!(visit.json()["component"], "Home", "the JSON page is untouched");

    let plain = client.get("/html").send().await;
    assert!(plain.header("x-inertia-devtools-id").is_some());
    assert_eq!(
        plain.body_text(),
        "<html><body><p>plain</p></body></html>",
        "a page that is not an Inertia page gets no tag"
    );
}

#[tokio::test]
async fn indt_under_a_public_root_the_base_path_rides_the_header_and_the_tag() {
    let dir = tempfile::tempdir().unwrap();
    let _env = app_env("local").await;
    crate::env_snapshot::set_env("APP_URL", Some("http://localhost/billing"));
    let client = client(router(), devtools(dir.path()));

    let first = client.get("/page").send().await;
    first.assert_ok();
    assert_eq!(first.header("x-inertia-devtools-base-path"), Some("/billing"));
    assert!(
        first.body_text().contains(
            "<script data-inertia-devtools-id data-inertia-devtools-base-path=\"/billing\" type=\"application/json\">"
        ),
        "{}",
        first.body_text()
    );
}
