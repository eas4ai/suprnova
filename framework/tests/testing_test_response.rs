//! `TestResponse` unit tests. Constructed directly from an
//! already-captured `(status, headers, body)` triple, so most of these
//! don't need a loopback HTTP harness at all — see
//! `framework/src/testing/response.rs` for the type itself and
//! `framework/tests/cors_middleware.rs` for it wired to a real
//! `handle_request` round trip.
//!
//! The `assert_session_has` tests double as the honesty check for that
//! one assertion: it needs a real `SessionStore` and a cookie that
//! really decrypts to that store's session id, so they seed an
//! in-memory store and mint the cookie under the same `Crypt` purpose
//! `SessionMiddleware` uses — mirroring the fake-store pattern in
//! `framework/tests/session_lazy_persistence.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::json;

use suprnova::testing::TestResponse;
use suprnova::{Crypt, CryptPurpose, EncryptionKey, FrameworkError, SessionData, SessionStore};

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn chained_assertions_all_pass_on_representative_responses() {
    let body = json!({
        "user": { "id": 7, "name": "Ada" },
        "items": [1, 2, 3],
    });
    let response = TestResponse::new(
        200,
        headers(&[
            ("Content-Type", "application/json"),
            ("Set-Cookie", "suprnova_session=xyz; Path=/; HttpOnly"),
        ]),
        body.to_string(),
    );

    response
        .assert_status(200)
        .assert_ok()
        .assert_header("content-type", "application/json")
        .assert_cookie("suprnova_session")
        .assert_json(json!({ "user": { "id": 7 } }))
        .assert_json_path("user.name", json!("Ada"))
        .assert_json_path("items.2", json!(3))
        .assert_json_count(Some("items"), 3)
        .assert_see("Ada");

    let redirect = TestResponse::new(302, headers(&[("Location", "/login")]), Bytes::new());
    redirect
        .assert_redirect(None)
        .assert_redirect(Some("/login"));
}

#[test]
#[should_panic(expected = "assert_status")]
fn assert_status_panics_on_a_mismatch() {
    TestResponse::new(404, headers(&[]), Bytes::new()).assert_status(200);
}

#[test]
#[should_panic(expected = "assert_redirect")]
fn assert_redirect_panics_without_a_3xx_status() {
    TestResponse::new(200, headers(&[]), Bytes::new()).assert_redirect(None);
}

#[test]
#[should_panic(expected = "assert_redirect")]
fn assert_redirect_panics_on_a_location_mismatch() {
    TestResponse::new(302, headers(&[("Location", "/login")]), Bytes::new())
        .assert_redirect(Some("/elsewhere"));
}

#[test]
#[should_panic(expected = "assert_json")]
fn assert_json_panics_when_a_key_mismatches() {
    TestResponse::new(200, headers(&[]), json!({ "id": 7 }).to_string())
        .assert_json(json!({ "id": 8 }));
}

#[test]
#[should_panic(expected = "assert_json_path")]
fn assert_json_path_panics_on_a_missing_path() {
    TestResponse::new(200, headers(&[]), json!({ "data": {} }).to_string())
        .assert_json_path("data.missing", json!(1));
}

#[test]
#[should_panic(expected = "assert_json_count")]
fn assert_json_count_panics_on_a_length_mismatch() {
    TestResponse::new(200, headers(&[]), json!([1, 2]).to_string()).assert_json_count(None, 3);
}

// The brief's own test file exercises negative cases for `assert_status`,
// `assert_redirect`, `assert_json`, `assert_json_path`, `assert_json_count`,
// and `assert_session_has`, but not `assert_ok`, `assert_see`,
// `assert_header`, or `assert_cookie`. An assertion that can't fail is
// worse than no assertion, so the four below pin those failure modes too.

#[test]
#[should_panic(expected = "assert_ok")]
fn assert_ok_panics_on_a_non_200_status() {
    TestResponse::new(201, headers(&[]), Bytes::new()).assert_ok();
}

#[test]
#[should_panic(expected = "assert_see")]
fn assert_see_panics_when_the_needle_is_absent() {
    TestResponse::new(200, headers(&[]), "hello world").assert_see("goodbye");
}

#[test]
#[should_panic(expected = "assert_header")]
fn assert_header_panics_on_a_value_mismatch() {
    TestResponse::new(
        200,
        headers(&[("Content-Type", "text/plain")]),
        Bytes::new(),
    )
    .assert_header("content-type", "application/json");
}

#[test]
#[should_panic(expected = "assert_cookie")]
fn assert_cookie_panics_when_no_matching_set_cookie_header() {
    TestResponse::new(
        200,
        headers(&[("Set-Cookie", "other_cookie=1; Path=/")]),
        Bytes::new(),
    )
    .assert_cookie("suprnova_session");
}

// ── `assert_session_has` — needs a real session store ───────────────

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

#[derive(Default)]
struct InMemoryStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for InMemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Mint a `Set-Cookie` header carrying `session_id`, encrypted the same
/// way `SessionMiddleware::create_session_cookie` does
/// (`framework/src/session/middleware.rs:340-371`) — id + `.` + a
/// touched-at timestamp, under `CryptPurpose::Cookie`.
fn encrypted_session_cookie_header(session_id: &str) -> (String, String) {
    let payload = format!("{session_id}.1700000000");
    let wire = Crypt::encrypt_string(CryptPurpose::Cookie, &payload)
        .expect("encrypt session cookie payload");
    (
        "set-cookie".to_string(),
        format!("suprnova_session={wire}; Path=/; HttpOnly"),
    )
}

#[tokio::test]
async fn assert_session_has_reads_the_store_through_the_response_cookie() {
    ensure_crypt();
    let store = Arc::new(InMemoryStore::default());
    let session_id = "a".repeat(40);
    let mut seeded = SessionData::new(session_id.clone(), "csrf-token".to_string());
    seeded.put("flash.message", "Saved!");
    store.write(&seeded).await.expect("seed session row");

    let (name, value) = encrypted_session_cookie_header(&session_id);
    let response = TestResponse::new(200, vec![(name, value)], Bytes::new())
        .with_session_store(store, "suprnova_session");

    response
        .assert_session_has("flash.message", json!("Saved!"))
        .await;
}

#[tokio::test]
#[should_panic(expected = "assert_session_has")]
async fn assert_session_has_panics_on_a_value_mismatch() {
    ensure_crypt();
    let store = Arc::new(InMemoryStore::default());
    let session_id = "b".repeat(40);
    let mut seeded = SessionData::new(session_id.clone(), "csrf-token".to_string());
    seeded.put("flash.message", "Saved!");
    store.write(&seeded).await.expect("seed session row");

    let (name, value) = encrypted_session_cookie_header(&session_id);
    let response = TestResponse::new(200, vec![(name, value)], Bytes::new())
        .with_session_store(store, "suprnova_session");

    response
        .assert_session_has("flash.message", json!("WRONG"))
        .await;
}
