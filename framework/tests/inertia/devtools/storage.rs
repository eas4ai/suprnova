//! PAR-075: entries are stored one file each with an index, pruned after
//! their time to live, limited per tab, and redacted before they are
//! written.

use chrono::{Duration, TimeZone, Utc};
use serde_json::{Value, json};
use suprnova::http::text;
use suprnova::testing::TestClock;
use suprnova::{
    HttpResponse, Inertia, InertiaResponse, MiddlewareRegistry, Prop, Redirect, Request, Response,
    Router,
};

use super::{client, devtools, entry_ids, entry_of, inertia, raw_send, read_entry, read_index};
use crate::protocol_harness::serve;

fn router() -> Router {
    Router::new()
        .get("/page", |req: Request| async move {
            InertiaResponse::new("Home")
                .with("token", "prop-token-value")
                .with("profile", json!({"name": "Ada", "api_key": "prop-api-key"}))
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .post("/login", |req: Request| async move {
            InertiaResponse::new("Login")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/flat", |req: Request| async move {
            InertiaResponse::new("Flat")
                .prop(
                    "auth.password",
                    Prop::eager(json!("prop-password-value")).always(),
                )
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .post("/forgot", |_req: Request| async {
            let response: Response = Redirect::to("/reset?token=abc").into();
            response
        })
        .get("/billing", |_req: Request| async {
            Ok(Inertia::location("/reset?token=abc"))
        })
        .get("/text", |_req: Request| async { text("ok") })
        .into()
}

/// A `multipart/form-data` body with the boundary `XYZ` holding the text
/// `fields`.
fn multipart(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut body = String::new();
    for (name, value) in fields {
        body.push_str(&format!(
            "--XYZ\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    }
    body.push_str("--XYZ--\r\n");
    body.into_bytes()
}

#[tokio::test]
async fn indt_each_entry_is_a_json_file_the_index_lists_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let first = client.get("/text").send().await;
    let second = client.get("/page").inertia().send().await;
    let first_id = first.header("x-inertia-devtools-id").unwrap();
    let second_id = second.header("x-inertia-devtools-id").unwrap();

    assert_eq!(entry_ids(dir.path()).len(), 2);
    assert_eq!(
        read_entry(dir.path(), first_id)["__meta"]["requestType"],
        "http"
    );
    let index = read_index(dir.path());
    let listed: Vec<&str> = index
        .as_array()
        .unwrap()
        .iter()
        .map(|meta| meta["id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, vec![second_id, first_id], "newest first");
    assert_eq!(index[0]["component"], "Home");
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        "*\n"
    );
}

#[tokio::test]
async fn indt_entries_past_their_ttl_are_pruned_once_the_interval_has_passed() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let start = Utc.with_ymd_and_hms(2026, 10, 8, 9, 0, 0).unwrap();
    let clock = TestClock::travel_to(start);

    let old = client.get("/text").send().await;
    let old_id = old.header("x-inertia-devtools-id").unwrap().to_string();

    clock.set(start + Duration::hours(24));
    let day = client.get("/text").send().await;
    let day_id = day.header("x-inertia-devtools-id").unwrap().to_string();
    assert!(
        entry_ids(dir.path()).contains(&old_id),
        "exactly 24 hours old is not past the TTL"
    );

    clock.set(start + Duration::hours(24) + Duration::seconds(100));
    client.get("/text").send().await;
    assert!(
        entry_ids(dir.path()).contains(&old_id),
        "the interval since the last prune has not passed"
    );

    clock.set(start + Duration::hours(25));
    let last = client.get("/text").send().await;
    let ids = entry_ids(dir.path());
    assert!(!ids.contains(&old_id), "25 hours old is pruned");
    assert!(ids.contains(&day_id), "1 hour old is kept");
    assert!(ids.contains(&last.header("x-inertia-devtools-id").unwrap().to_string()));
    let indexed: Vec<String> = read_index(dir.path())
        .as_array()
        .unwrap()
        .iter()
        .map(|meta| meta["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!indexed.contains(&old_id), "the index forgets it too");
}

#[tokio::test]
async fn indt_a_tab_keeps_its_newest_100_entries() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let mut ids = Vec::new();
    for _ in 0..101 {
        let response = client
            .get("/text")
            .header("X-Inertia-Devtools-Tab", "tab-1")
            .send()
            .await;
        ids.push(
            response
                .header("x-inertia-devtools-id")
                .unwrap()
                .to_string(),
        );
    }
    let other = client
        .get("/text")
        .header("X-Inertia-Devtools-Tab", "tab-2")
        .send()
        .await;
    let kept = entry_ids(dir.path());
    assert!(!kept.contains(&ids[0]), "the oldest of the tab is gone");
    assert_eq!(kept.len(), 101, "100 of tab-1 and one of tab-2");
    assert!(kept.contains(&ids[100]));
    assert!(kept.contains(&other.header("x-inertia-devtools-id").unwrap().to_string()));
}

#[tokio::test]
async fn indt_sensitive_keys_headers_and_query_values_are_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let login = client
        .post("/login?token=abc&page=2")
        .inertia()
        .header("Cookie", "suprnova_session=cookie-value")
        .header("Authorization", "Bearer bearer-value")
        .json(
            &json!({"email": "ada@example.com", "Password": "hunter2", "nested": {"secret": "s3"}}),
        )
        .send()
        .await;
    let id = login.header("x-inertia-devtools-id").unwrap();
    let raw = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    for secret in ["hunter2", "s3", "cookie-value", "bearer-value", "token=abc"] {
        assert!(!raw.contains(secret), "{secret} reached the store: {raw}");
    }
    let entry: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        entry["http"]["requestBody"]["value"]["Password"],
        "[REDACTED]"
    );
    assert_eq!(
        entry["http"]["requestBody"]["value"]["email"],
        "ada@example.com"
    );
    assert_eq!(
        entry["http"]["requestBody"]["value"]["nested"]["secret"],
        "[REDACTED]"
    );
    assert_eq!(entry["http"]["requestHeaders"]["cookie"], "[REDACTED]");
    assert_eq!(
        entry["http"]["requestHeaders"]["authorization"],
        "[REDACTED]"
    );
    let url = entry["__meta"]["url"].as_str().unwrap();
    assert!(url.ends_with("/login?token=%5BREDACTED%5D&page=2"), "{url}");
    let index = std::fs::read_to_string(dir.path().join("_meta.json")).unwrap();
    assert!(!index.contains("token=abc"), "{index}");

    let page = client.get("/page").inertia().send().await;
    let entry = entry_of(dir.path(), &page);
    assert_eq!(entry["propValues"]["token"], "[REDACTED]");
    assert_eq!(entry["propValues"]["profile"]["api_key"], "[REDACTED]");
    assert_eq!(entry["propValues"]["profile"]["name"], "Ada");
    assert_eq!(
        entry["http"]["responseBody"]["value"]["props"]["token"],
        "[REDACTED]"
    );
    assert_eq!(
        page.json()["props"]["token"],
        "prop-token-value",
        "the client still gets it"
    );
}

#[tokio::test]
async fn indt_a_flattened_form_field_naming_a_redaction_key_is_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let reply = raw_send(
        addr,
        "POST",
        "/login",
        &[
            ("X-Inertia", b"true"),
            ("Content-Type", b"multipart/form-data; boundary=XYZ"),
        ],
        multipart(&[
            ("user[password]", "hunter2"),
            ("filter[secret]", "s3cret"),
            ("data[0][token]", "t0ken"),
            ("user[name]", "Ada"),
            ("passwords", "a longer word"),
        ]),
    )
    .await;
    assert_eq!(reply.status, 200);
    let id = &reply.headers["x-inertia-devtools-id"];
    let raw = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    for secret in ["hunter2", "s3cret", "t0ken"] {
        assert!(!raw.contains(secret), "{secret} reached the store: {raw}");
    }
    let entry: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        entry["http"]["requestBody"]["value"],
        json!({
            "user[password]": "[REDACTED]",
            "filter[secret]": "[REDACTED]",
            "data[0][token]": "[REDACTED]",
            "user[name]": "Ada",
            "passwords": "a longer word",
        })
    );
}

#[tokio::test]
async fn indt_a_dotted_prop_path_naming_a_redaction_key_is_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let page = client(router(), devtools(dir.path()))
        .get("/flat")
        .inertia()
        .send()
        .await;
    let id = page.header("x-inertia-devtools-id").unwrap();
    let raw = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    assert!(
        !raw.contains("prop-password-value"),
        "the prop value reached the store: {raw}"
    );
    let entry: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(entry["propValues"]["auth.password"], "[REDACTED]");
    assert_eq!(
        page.json()["props"]["auth"]["password"],
        "prop-password-value",
        "the client still gets it"
    );
}

#[tokio::test]
async fn indt_url_query_values_in_headers_and_bodies_are_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let redirect = client
        .post("/forgot")
        .inertia()
        .header("Referer", "http://localhost/forgot?token=abc&page=2")
        .json(&json!({"redirect_to": "/reset?token=abc&page=2", "note": "see the docs?"}))
        .send()
        .await;
    assert_eq!(redirect.status(), 302);
    assert_eq!(
        redirect.header("location"),
        Some("/reset?token=abc"),
        "the client still gets it"
    );
    let id = redirect.header("x-inertia-devtools-id").unwrap();
    let raw = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    assert!(
        !raw.contains("token=abc"),
        "a token reached the store: {raw}"
    );
    let entry: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        entry["http"]["responseHeaders"]["location"],
        "/reset?token=%5BREDACTED%5D"
    );
    assert_eq!(
        entry["http"]["requestHeaders"]["referer"],
        "http://localhost/forgot?token=%5BREDACTED%5D&page=2"
    );
    assert_eq!(
        entry["http"]["requestHeaders"]["accept"], "text/html, application/xhtml+xml",
        "a header that is not a URL is untouched"
    );
    assert_eq!(
        entry["http"]["requestBody"]["value"]["redirect_to"],
        "/reset?token=%5BREDACTED%5D&page=2"
    );
    assert_eq!(
        entry["http"]["requestBody"]["value"]["note"],
        "see the docs?"
    );
    assert_eq!(
        entry["__meta"]["redirectLocation"],
        "/reset?token=%5BREDACTED%5D"
    );

    let away = client.get("/billing").inertia().send().await;
    assert_eq!(away.status(), 409);
    let id = away.header("x-inertia-devtools-id").unwrap();
    let raw = std::fs::read_to_string(dir.path().join(format!("{id}.json"))).unwrap();
    assert!(
        !raw.contains("token=abc"),
        "a token reached the store: {raw}"
    );
    let entry: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        entry["http"]["responseHeaders"]["x-inertia-location"],
        "/reset?token=%5BREDACTED%5D"
    );
    let index = std::fs::read_to_string(dir.path().join("_meta.json")).unwrap();
    assert!(!index.contains("token=abc"), "{index}");
}

#[tokio::test]
async fn indt_a_value_that_is_not_text_becomes_a_marker_and_the_entry_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&inertia(devtools(dir.path())))),
    )
    .await;
    let reply = raw_send(addr, "GET", "/text", &[("X-Note", b"caf\xe9")], Vec::new()).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, "ok", "the response is the handler's");
    let entry = read_entry(dir.path(), &reply.headers["x-inertia-devtools-id"]);
    assert_eq!(
        entry["http"]["requestHeaders"]["x-note"],
        "[UNSERIALIZABLE]"
    );
    assert_eq!(entry["__meta"]["status"], 200);
}
