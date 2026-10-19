//! Live actions as tools through the browser action endpoint: the caller
//! check every request to a tool action runs, the island the tool-call
//! header binds a request to, and the snapshots the browser endpoint
//! refuses (AGENT-002, AGENT-012, AGENT-013).

use std::net::{IpAddr, Ipv4Addr};

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova::agents::{AgentsConfig, CallLimit};
use suprnova::testing::{TestClient, TestRequest};
use suprnova::{Crypt, CryptPurpose, ToolCaller};

use crate::fixture::{self, TestUser};
use crate::live_fixture::{self, PAGE};

/// An island of a rendered page, as the browser runtime reads its root.
struct PageIsland {
    component: String,
    document_key: String,
    revision: String,
    seed: bool,
    snapshot: Value,
}

impl PageIsland {
    /// The island of `page` keyed `document_key`.
    fn keyed(page: &str, document_key: &str) -> Self {
        for root in page.split("<div").skip(1) {
            let attributes = root.split('>').next().unwrap_or_default();
            if attribute(attributes, "data-suprnova-live-document-key").as_deref()
                != Some(document_key)
            {
                continue;
            }
            let snapshot = attribute(attributes, "data-suprnova-live-snapshot")
                .expect("the island's snapshot");
            let snapshot = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(snapshot)
                .expect("a base64url snapshot");
            return Self {
                component: attribute(attributes, "data-suprnova-live-component")
                    .expect("the island's component"),
                document_key: document_key.to_string(),
                revision: attribute(attributes, "data-suprnova-live-revision")
                    .expect("the island's revision"),
                seed: attribute(attributes, "data-suprnova-live-snapshot-kind").as_deref()
                    == Some("seed"),
                snapshot: serde_json::from_slice(&snapshot).expect("a JSON snapshot"),
            };
        }
        panic!("the page has no island keyed `{document_key}`: {page}");
    }
}

/// The value of the attribute `name` among an element's `attributes`.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let prefix = format!(" {name}=\"");
    let start = attributes.find(&prefix)? + prefix.len();
    let end = start + attributes[start..].find('"')?;
    Some(
        attributes[start..end]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("&amp;", "&"),
    )
}

fn random_identity() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(uuid::Uuid::new_v4().as_bytes())
}

/// A page as a client loaded it: its HTML and the CSRF token its response
/// set.
struct Page {
    html: String,
    xsrf: Option<String>,
}

/// The request a click of `action` on `island` of `page` sends, with the
/// session's CSRF proof, and the tool-call header when `tool` names one.
fn click(
    client: &TestClient,
    page: &Page,
    island: &PageIsland,
    action: &str,
    arguments: Value,
    tool: Option<&str>,
) -> TestRequest {
    let snapshot = if island.seed {
        json!({
            "kind": "seed_promotion",
            "envelope": island.snapshot,
            "browser_nonce": random_identity(),
        })
    } else {
        json!({ "kind": "instance", "envelope": island.snapshot })
    };
    let body = json!({
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot_schema_version": 1,
        "correlation_id": random_identity(),
        "idempotency_key": random_identity(),
        "component": island.component,
        "base_revision": island.revision,
        "snapshot": snapshot,
        "child_parameters": null,
        "model_proposals": {},
        "operations": [{ "kind": "invoke_action", "name": action, "arguments": arguments }],
        "extensions": { "x_suprnova_live_document_key_v1": island.document_key },
    });
    let mut request = client
        .post("/__live/action")
        .json(&body)
        .header(
            "Content-Type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        )
        .header("Sec-Fetch-Site", "same-origin");
    if let Some(tool) = tool {
        request = request.header("Suprnova-Tool-Call", tool);
    }
    if let Some(token) = &page.xsrf {
        request = request.header("X-XSRF-TOKEN", token);
    }
    request
}

/// The page, as `client` loads it.
async fn page(client: &TestClient) -> Page {
    let page = client.get(PAGE).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    Page {
        html: page.body_text(),
        xsrf: page.cookie("XSRF-TOKEN"),
    }
}

fn outcome(body: &str) -> Value {
    serde_json::from_str::<Value>(body).map_or(Value::Null, |answer| answer["outcome"].clone())
}

#[tokio::test]
async fn agent_012_a_router_without_agents_offers_no_tool() {
    let _container = fixture::setup();
    // Without `.agents(config)` nothing would check a tool call's caller or
    // count it, so the page offers WebMCP no tool at all.
    let client = live_fixture::live_client_without_agents();
    let page = page(&client).await;
    assert!(
        page.html.contains("data-suprnova-live-island"),
        "{}",
        page.html
    );
    assert!(
        !page.html.contains("data-suprnova-live-tools"),
        "{}",
        page.html
    );
}

#[tokio::test]
async fn agent_012_a_router_serving_agents_offers_its_tools() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let page = page(&client).await;
    assert!(
        page.html.contains("data-suprnova-live-tools"),
        "{}",
        page.html
    );
    assert!(page.html.contains("counter.peek.main"), "{}", page.html);
}

#[tokio::test]
async fn agent_002_live_tool_action_checks_the_caller_without_the_header() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let page = page(&client).await;
    let counter = PageIsland::keyed(&page.html, "main");
    // `add` is the tool `counter.add`, for users and agents: a guest who
    // leaves the header off is refused as one who sends it.
    let refused = click(
        &client,
        &page,
        &counter,
        "add",
        json!({ "amount": 1 }),
        None,
    )
    .send()
    .await;
    assert_eq!(refused.status(), 401, "{}", refused.body_text());
    // An action that is not a tool is an ordinary click.
    let reset = click(&client, &page, &counter, "reset", json!({}), None)
        .send()
        .await;
    assert_eq!(reset.status(), 200, "{}", reset.body_text());
    assert_eq!(outcome(&reset.body_text()), "accepted");
}

#[tokio::test]
async fn agent_002_live_tool_action_without_the_header_is_not_counted() {
    let _container = fixture::setup();
    let client = live_fixture::live_client_with(
        AgentsConfig::new().caller_limit(CallLimit::per_minute(1)),
        true,
    )
    .acting_as(&TestUser::arc("alice"));
    let page = page(&client).await;
    let counter = PageIsland::keyed(&page.html, "main");
    // A signed-in user's clicks run, and take nothing of the tool budget.
    for _ in 0..2 {
        let clicked = click(
            &client,
            &page,
            &counter,
            "add",
            json!({ "amount": 1 }),
            None,
        )
        .send()
        .await;
        assert_eq!(clicked.status(), 200, "{}", clicked.body_text());
        assert_eq!(outcome(&clicked.body_text()), "accepted");
    }
    // The budget of one tool call is still whole.
    let called = client
        .live_tool("counter.add.main", &page.html, json!({ "amount": 1 }))
        .send()
        .await;
    assert_eq!(called.status(), 200, "{}", called.body_text());
    assert_eq!(outcome(&called.body_text()), "accepted");
    let limited = client
        .live_tool("counter.add.main", &page.html, json!({ "amount": 1 }))
        .send()
        .await;
    assert_eq!(limited.status(), 429, "{}", limited.body_text());
}

#[tokio::test]
async fn agent_012_the_tool_header_cannot_cover_another_action() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let page = page(&client).await;
    let counter = PageIsland::keyed(&page.html, "main");
    let covered = click(
        &client,
        &page,
        &counter,
        "reset",
        json!({}),
        Some("counter.peek.main"),
    )
    .send()
    .await;
    assert_eq!(covered.status(), 404, "{}", covered.body_text());
}

#[tokio::test]
async fn agent_012_the_tool_header_cannot_cover_another_component() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let page = page(&client).await;
    // The quota's own public tool, under the name of the counter's.
    let quota = PageIsland::keyed(&page.html, "side");
    assert_eq!(quota.component, "tests.agent-quota");
    let covered = click(
        &client,
        &page,
        &quota,
        "read",
        json!({}),
        Some("counter.peek.main"),
    )
    .send()
    .await;
    assert_eq!(covered.status(), 404, "{}", covered.body_text());
    // Under its own name it runs.
    let called = click(
        &client,
        &page,
        &quota,
        "read",
        json!({}),
        Some("quota.read.side"),
    )
    .send()
    .await;
    assert_eq!(called.status(), 200, "{}", called.body_text());
    assert_eq!(outcome(&called.body_text()), "accepted");
}

#[tokio::test]
async fn agent_013_a_guest_handle_opens_only_from_its_address_bucket() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let first = client
        .clone()
        .with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 7, 0, 1)));
    let second = client
        .clone()
        .with_peer_address(IpAddr::V4(Ipv4Addr::new(10, 7, 0, 2)));
    let minted = first.mcp().call("quota.read", json!({ "mount": {} })).await;
    assert!(!minted.is_error(), "{:?}", minted.response());
    let handle = minted.handle().expect("a guest handle").to_string();
    // Another guest, from another address, cannot act on it.
    let elsewhere = second
        .mcp()
        .call("quota.read", json!({ "handle": handle }))
        .await;
    assert!(elsewhere.is_error(), "{:?}", elsewhere.response());
    assert!(elsewhere.text().contains("handle"), "{}", elsewhere.text());
    // From its own address it opens.
    let again = first
        .mcp()
        .call("quota.read", json!({ "handle": handle }))
        .await;
    assert!(!again.is_error(), "{:?}", again.response());
    let structured = again.structured().expect("structured content");
    assert_eq!(structured["fields"]["reads"], "2", "{structured}");
}

/// The agent scope a handle of the user `user` is sealed under on the
/// fixture's tenantless routes: spec 05's digest of the caller's kind and
/// user, with no registration, client, tenant or address.
fn user_scope(user: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/agent-scope/v1\0");
    for part in [Some("user"), Some(user), None, None, None, None] {
        match part {
            Some(bytes) => {
                digest.update([1]);
                digest.update((bytes.len() as u64).to_be_bytes());
                digest.update(bytes.as_bytes());
            }
            None => digest.update([0]),
        }
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.finalize())
}

#[tokio::test]
async fn agent_013_an_agent_route_snapshot_is_refused_by_the_browser_endpoint() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let minted = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("alice")))
        .call(
            "counter.add",
            json!({
                "mount": { "start": 1, "label": "Main" },
                "arguments": { "amount": 1 },
            }),
        )
        .await;
    assert!(!minted.is_error(), "{:?}", minted.response());
    let handle = minted.handle().expect("a handle");
    let envelope =
        Crypt::decrypt_string_for(CryptPurpose::LiveAgentHandle, &user_scope("alice"), handle)
            .expect("the handle opens under the caller's agent scope");
    let snapshot: Value = serde_json::from_str(&envelope).expect("a snapshot envelope");
    // The snapshot is genuine and the agent route's own: the browser
    // endpoint still refuses it, signed in as the same user.
    let browser = client.acting_as(&TestUser::arc("alice"));
    let page = page(&browser).await;
    let island = PageIsland {
        component: snapshot["body"]["component"]
            .as_str()
            .map_or_else(|| live_fixture::COUNTER.to_string(), str::to_string),
        document_key: "agent".to_string(),
        revision: snapshot["body"]["revision"]
            .as_str()
            .map_or_else(|| "1".to_string(), str::to_string),
        seed: false,
        snapshot,
    };
    let refused = click(
        &browser,
        &page,
        &island,
        "add",
        json!({ "amount": 1 }),
        None,
    )
    .send()
    .await;
    assert_eq!(refused.status(), 404, "{}", refused.body_text());
}
