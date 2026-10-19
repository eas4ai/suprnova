//! A form request on a `GET` or `HEAD` request reads the query string, as
//! Laravel validates `$request->all()`, which is the query on a `GET`
//! (AGENT-014). A request that carries a body reads it as before, so the
//! MCP dispatcher's JSON body and every `POST` are unchanged.

use serde_json::json;
use suprnova::testing::TestClient;
use suprnova::{FormRequest, HttpResponse, MiddlewareRegistry, Request, Router};

/// The filters a search page validates.
#[suprnova::request]
struct Search {
    /// What to look for.
    #[validate(length(min = 2))]
    q: String,
    /// The page, from 1.
    #[validate(range(min = 1))]
    page: Option<u32>,
    /// Whether to include drafts, sent as `include_drafts`.
    #[serde(rename = "include_drafts")]
    drafts: Option<bool>,
}

/// `GET`, `HEAD` and `POST` routes that read a [`Search`] and echo it.
fn routes() -> Router {
    async fn echo(request: Request) -> suprnova::Response {
        let search = Search::extract(request).await?;
        Ok(HttpResponse::json(json!({
            "q": search.q,
            "page": search.page,
            "drafts": search.drafts,
        })))
    }
    Router::new()
        .get("/search", echo)
        .post("/search", echo)
        .into()
}

fn client() -> TestClient {
    TestClient::new(routes(), MiddlewareRegistry::new())
}

#[tokio::test]
async fn agent_014_a_get_form_request_reads_its_query() {
    let reply = client()
        .get("/search?q=caf%C3%A9&page=2&include_drafts=true")
        .send()
        .await;
    reply.assert_ok();
    assert_eq!(
        reply.json(),
        json!({ "q": "café", "page": 2, "drafts": true })
    );
}

#[tokio::test]
async fn agent_014_a_get_form_request_answers_422_naming_the_field() {
    let reply = client()
        .get("/search?q=x&page=0&include_drafts=maybe")
        .header("Accept", "application/json")
        .send()
        .await;
    assert_eq!(reply.status(), 422, "{}", reply.body_text());
    let body = reply.json();
    let errors = body["errors"]
        .as_object()
        .unwrap_or_else(|| panic!("no errors in {body}"));
    // A field that does not parse is named by the name the query used.
    assert!(errors.contains_key("include_drafts"), "{body}");
    let missing = client()
        .get("/search?page=1")
        .header("Accept", "application/json")
        .send()
        .await;
    assert_eq!(missing.status(), 422, "{}", missing.body_text());
    assert!(
        missing.json()["errors"]["q"].is_array(),
        "{}",
        missing.body_text()
    );
    let short = client()
        .get("/search?q=x")
        .header("Accept", "application/json")
        .send()
        .await;
    assert_eq!(short.status(), 422, "{}", short.body_text());
    assert!(
        short.json()["errors"]["q"].is_array(),
        "{}",
        short.body_text()
    );
}

#[tokio::test]
async fn agent_014_a_head_form_request_reads_its_query() {
    let reply = client()
        .send(hyper::Method::HEAD, "/search?q=ok")
        .send()
        .await;
    reply.assert_ok();
    let refused = client()
        .send(hyper::Method::HEAD, "/search?q=x")
        .send()
        .await;
    assert_eq!(refused.status(), 422);
}

#[tokio::test]
async fn agent_014_a_get_form_request_with_a_json_body_reads_the_body() {
    let reply = client()
        .get("/search?q=from-the-query")
        .json(&json!({ "q": "from-the-body" }))
        .send()
        .await;
    reply.assert_ok();
    assert_eq!(reply.json()["q"], "from-the-body");
}

#[tokio::test]
async fn agent_014_a_post_form_request_is_unchanged() {
    let reply = client()
        .post("/search?q=from-the-query")
        .json(&json!({ "q": "from-the-body" }))
        .send()
        .await;
    reply.assert_ok();
    assert_eq!(reply.json()["q"], "from-the-body");
    // A POST without a body it can read is still refused, never read from
    // the query.
    let unread = client().post("/search?q=from-the-query").send().await;
    assert_eq!(unread.status(), 415, "{}", unread.body_text());
}
