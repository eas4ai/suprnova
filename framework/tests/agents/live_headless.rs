//! Live actions as tools: which actions are tools, and the headless calls
//! over the MCP endpoint (AGENT-001, AGENT-002, AGENT-012, AGENT-013,
//! AGENT-018).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use suprnova::agents::AgentsConfig;
use suprnova::live::testing::{AdjustableTestClock, prepare_live_router_with_clock_for_test};
use suprnova::testing::TestClient;
use suprnova::{Response, ToolCaller, handler, json_response, tool};

use crate::fixture::{self, TestUser};
use crate::live_fixture::{self, GUARDED, PAGE, RESOLVED};

fn alice() -> ToolCaller {
    ToolCaller::user(TestUser::arc("alice"))
}

fn bob() -> ToolCaller {
    ToolCaller::user(TestUser::arc("bob"))
}

fn fresh(start: u64, amount: u64) -> Value {
    json!({
        "mount": { "start": start, "label": "Main" },
        "arguments": { "amount": amount },
    })
}

/// A handler tool whose name is the browser name of `counter.peek` on an
/// island keyed `left`. No shared router routes it.
#[tool(
    name = "counter.peek.left",
    description = "Shadow a browser tool name.",
    callers = [user],
)]
#[handler]
pub async fn shadow() -> Response {
    json_response!({ "shadow": true })
}

#[tokio::test]
async fn agent_001_live_unmarked_action_is_not_a_tool() {
    let _container = fixture::setup();
    let registry = live_fixture::live_router_with(AgentsConfig::new())
        .tool_registry()
        .expect("the tools check");
    let add = registry
        .get("counter.add")
        .expect("the headless tool is offered");
    assert_eq!(add.live_action(), Some((live_fixture::COUNTER, "add")));
    // A tool only the browser registers is checked but never offered.
    assert!(registry.get("counter.peek").is_none());
    let peek = registry
        .live_tool("counter.peek")
        .expect("the browser tool is checked");
    assert_eq!(peek.live_action(), Some((live_fixture::COUNTER, "peek")));
    // `reset` is an action without #[tool]: no tool runs it.
    for tool in registry.tools() {
        assert_ne!(tool.live_action().map(|(_, action)| action), Some("reset"));
    }
    assert!(registry.live_tool("counter.reset").is_none());
}

#[tokio::test]
async fn agent_001_live_tool_name_prefix_cannot_shadow_another_tool() {
    let _container = fixture::setup();
    let router: suprnova::Router = suprnova::Router::new().post("/shadow", shadow).into();
    let error = router
        .agents(AgentsConfig::new())
        .tool_registry()
        .expect_err("a route tool cannot take a browser tool's name");
    let message = error.to_string();
    assert!(message.contains("counter.peek.left"), "{message}");
    assert!(message.contains("counter.peek"), "{message}");
}

#[tokio::test]
async fn agent_002_live_headless_tool_refuses_a_caller_kind_it_did_not_accept() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let before = RESOLVED.load(Ordering::SeqCst);
    let guest = client.mcp().call("counter.add", fresh(1, 1)).await;
    assert_eq!(guest.status(), 401, "{:?}", guest.response());
    assert!(guest.structured().is_none());
    assert_eq!(
        RESOLVED.load(Ordering::SeqCst),
        before,
        "nothing was mounted"
    );
}

#[tokio::test]
async fn agent_013_headless_tool_is_listed_and_callable_without_a_browser() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let listed = client.mcp().as_caller(alice()).list().await;
    let names = listed.tool_names();
    assert!(names.contains(&"counter.add".to_string()), "{names:?}");

    let result = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(2, 3))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    let structured = result.structured().expect("structured content");
    assert_eq!(structured["outcome"], "accepted");
    assert_eq!(structured["fields"]["count"], "5");
    assert_eq!(structured["fields"]["label"], "Main");
    // Only the fields the component marks agent-visible.
    assert!(structured["fields"].get("owner").is_none(), "{structured}");
    assert_eq!(
        structured["fields"].as_object().map(serde_json::Map::len),
        Some(2)
    );
    assert!(result.handle().is_some_and(|handle| !handle.is_empty()));
    assert!(structured["expiresAt"].as_u64().is_some_and(|at| at > 0));
    // The sealed handle shows nothing of the state it carries.
    let handle = result.handle().unwrap_or_default();
    assert!(!handle.contains("alice") && !handle.contains("Main"));
    assert_eq!(
        RESOLVED.load(Ordering::SeqCst),
        1,
        "the resolver filled the owner"
    );
}

#[tokio::test]
async fn agent_013_schema_has_mount_handle_and_arguments() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let listed = client.mcp().as_caller(alice()).list().await;
    let tools = listed
        .result()
        .and_then(|result| result["tools"].as_array())
        .cloned()
        .unwrap_or_default();
    let add = tools
        .iter()
        .find(|tool| tool["name"] == "counter.add")
        .expect("counter.add is listed");
    let input = &add["inputSchema"];
    assert_eq!(input["type"], "object");
    assert_eq!(input["additionalProperties"], false);
    assert_eq!(input["required"], json!(["arguments"]));
    let mount = &input["properties"]["mount"];
    assert_eq!(mount["required"], json!(["label", "start"]));
    assert_eq!(mount["additionalProperties"], false);
    assert!(mount["properties"].get("owner").is_none(), "{mount}");
    assert_eq!(mount["properties"]["label"], json!({ "type": "string" }));
    assert!(mount["description"].is_string());
    assert_eq!(input["properties"]["handle"]["type"], "string");
    let arguments = &input["properties"]["arguments"];
    assert_eq!(arguments["required"], json!(["amount"]));
    assert!(arguments.get("$schema").is_none());
    let output = &add["outputSchema"];
    assert_eq!(
        output["required"],
        json!(["outcome", "fields", "handle", "expiresAt"])
    );
    assert!(
        output["properties"]["fields"]["properties"]
            .get("count")
            .is_some()
    );
    assert!(
        output["properties"]["fields"]["properties"]
            .get("owner")
            .is_none()
    );
}

#[tokio::test]
async fn agent_013_an_undeclared_mount_property_is_refused() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let result = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({
                "mount": { "start": 1, "label": "Main", "extra": true },
                "arguments": { "amount": 1 },
            }),
        )
        .await;
    assert!(result.is_error(), "{:?}", result.response());
    assert!(result.text().contains("mount.extra"), "{}", result.text());
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0, "nothing was mounted");
}

#[tokio::test]
async fn agent_013_mount_properties_are_validated_like_arguments() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    // Shapes the codecs refuse, each named.
    let result = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({
                "mount": { "start": "007", "label": "Main" },
                "arguments": { "amount": "-1" },
            }),
        )
        .await;
    assert!(result.is_error(), "{:?}", result.response());
    let text = result.text();
    assert!(text.contains("mount.start"), "{text}");
    assert!(text.contains("arguments.amount"), "{text}");
    // A missing required property is named too.
    let missing = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({ "mount": { "start": 1 }, "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(missing.is_error());
    assert!(missing.text().contains("mount.label"), "{}", missing.text());
    // Mount properties and a handle together are refused.
    let both = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({
                "mount": { "start": 1, "label": "Main" },
                "handle": "x",
                "arguments": { "amount": 1 },
            }),
        )
        .await;
    assert!(both.is_error());
    assert!(both.text().contains("handle"), "{}", both.text());
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0, "nothing was mounted");
}

#[tokio::test]
async fn agent_013_mount_checks_the_gate_before_loading() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let mallory = client
        .mcp()
        .as_caller(ToolCaller::user(TestUser::arc("mallory")))
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(mallory.is_error(), "{:?}", mallory.response());
    assert!(
        mallory.text().contains("unauthorized"),
        "{}",
        mallory.text()
    );
    // The policy reads the values the agent sent.
    let large = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(5_000, 1))
        .await;
    assert!(large.is_error(), "{:?}", large.response());
    assert!(large.text().contains("unauthorized"), "{}", large.text());
    // Nothing was loaded: the resolver never ran.
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn agent_013_mount_without_a_policy_is_refused() {
    let _container = fixture::setup();
    let client = live_fixture::live_client_with(AgentsConfig::new(), false);
    // Even an ability every hook allows is refused when no policy defines
    // it.
    suprnova::Gate::before(|_user: &ToolCaller, _ability: &str| Some(true));
    let result = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(result.is_error(), "{:?}", result.response());
    assert!(result.text().contains("unauthorized"), "{}", result.text());
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn agent_013_resolver_property_cannot_be_set_by_the_agent() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let result = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({
                "mount": { "start": 1, "label": "Main", "owner": "eve" },
                "arguments": { "amount": 1 },
            }),
        )
        .await;
    assert!(result.is_error(), "{:?}", result.response());
    assert!(result.text().contains("mount.owner"), "{}", result.text());
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn agent_013_headless_action_is_authorized() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    // Bob may mount a counter but not add to it: the action's own
    // `live:tests.agent-counter.add` policy refuses him.
    let denied = client
        .mcp()
        .as_caller(bob())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(denied.is_error(), "{:?}", denied.response());
    assert!(denied.text().contains("unauthorized"), "{}", denied.text());
    assert!(denied.handle().is_none());
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 1, "bob's mount ran");
    let allowed = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!allowed.is_error(), "{:?}", allowed.response());
}

#[tokio::test]
#[tracing_test::traced_test]
async fn agent_013_headless_action_runs_the_execution_phases_of_a_click() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let result = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    // An instanced action's phases, in a click's order: the ledger claim,
    // hydration, model binding, authorization, validation, the action
    // between its hooks, the render, the signed successor, the response
    // sealing and the ledger acceptance. The action declares no host
    // transaction, so the transaction port records none, as for a click.
    let expected = [
        "Claim",
        "Hydrate",
        "Bind",
        "Authorize",
        "Validate",
        "BeforeAction",
        "Action",
        "AfterAction",
        "Render",
        "Dehydrate",
        "Sign",
        "OutcomeValidation",
        "ResponseIntentPreparation",
        "ResponseSealing",
        "LedgerAcceptance",
    ];
    logs_assert(|lines: &[&str]| {
        let phases: Vec<String> = lines
            .iter()
            .filter(|line| line.contains("Live execution phase"))
            .filter_map(|line| {
                line.split("phase=")
                    .nth(1)
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_string)
            })
            .collect();
        let mut at = 0;
        for phase in &phases {
            if at < expected.len() && phase == expected[at] {
                at += 1;
            }
        }
        if at == expected.len() {
            Ok(())
        } else {
            Err(format!("phases {phases:?} miss {:?}", &expected[at..]))
        }
    });
}

#[tokio::test]
async fn agent_013_a_second_call_with_the_handle_sees_the_first() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let first = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 2))
        .await;
    assert!(!first.is_error(), "{:?}", first.response());
    let handle = first.handle().expect("a handle").to_string();
    let second = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({ "handle": handle, "arguments": { "amount": 4 } }),
        )
        .await;
    assert!(!second.is_error(), "{:?}", second.response());
    assert_eq!(
        second
            .structured()
            .map(|structured| structured["fields"]["count"].clone()),
        Some(json!("7"))
    );
    assert_ne!(second.handle(), first.handle());
    assert_eq!(
        RESOLVED.load(Ordering::SeqCst),
        1,
        "the handle mounted nothing"
    );
}

#[tokio::test]
async fn agent_013_a_stale_or_tampered_handle_is_refused() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let first = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    let handle = first.handle().expect("a handle").to_string();
    let next = |handle: &str| {
        let client = client.clone();
        let handle = handle.to_string();
        async move {
            client
                .mcp()
                .as_caller(alice())
                .call(
                    "counter.add",
                    json!({ "handle": handle, "arguments": { "amount": 1 } }),
                )
                .await
        }
    };
    let second = next(&handle).await;
    assert!(!second.is_error(), "{:?}", second.response());
    // The first handle's revision was consumed.
    let stale = next(&handle).await;
    assert!(stale.is_error(), "{:?}", stale.response());
    assert!(stale.text().contains("out of date"), "{}", stale.text());
    // A handle changed by one character does not open.
    let mut tampered = second.handle().expect("a handle").to_string();
    let last = tampered.pop().unwrap_or('A');
    tampered.push(if last == 'A' { 'B' } else { 'A' });
    let refused = next(&tampered).await;
    assert!(refused.is_error(), "{:?}", refused.response());
    assert!(refused.text().contains("not valid"), "{}", refused.text());
    let garbage = next("not-a-handle").await;
    assert!(garbage.is_error());
    assert!(garbage.text().contains("not valid"), "{}", garbage.text());
}

#[tokio::test]
async fn agent_013_a_handle_is_refused_for_another_caller() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let first = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    let handle = first.handle().expect("a handle").to_string();
    // Alice's own agent account acts as alice, and the action's policy
    // allows her: only the handle's caller binding refuses it.
    let agent = ToolCaller::agent("reg-alice")
        .claimed(true)
        .bound_to(TestUser::arc("alice"));
    let other = client
        .mcp()
        .as_caller(agent)
        .call(
            "counter.add",
            json!({ "handle": handle, "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(other.is_error(), "{:?}", other.response());
    assert!(other.text().contains("not valid"), "{}", other.text());
    // A client of alice's is another caller too.
    let client_of_alice = alice().client("other-client");
    let client_call = client
        .mcp()
        .as_caller(client_of_alice)
        .call(
            "counter.add",
            json!({ "handle": first.handle().unwrap_or_default(), "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(client_call.is_error(), "{:?}", client_call.response());
}

#[tokio::test]
async fn agent_013_a_handle_expires_as_a_browser_snapshot_does() {
    let _container = fixture::setup();
    live_fixture::register_components();
    live_fixture::define_live_policies();
    let clock = Arc::new(AdjustableTestClock::new(1_800_000_000_000));
    let router = live_fixture::live_router_with(AgentsConfig::new());
    prepare_live_router_with_clock_for_test(&router, clock.clone())
        .expect("prepare the Live runtime");
    let client = TestClient::new(router, live_fixture::browser_middleware());
    let first = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!first.is_error(), "{:?}", first.response());
    let expires_at = first
        .structured()
        .and_then(|structured| structured["expiresAt"].as_u64())
        .expect("an expiry");
    assert!(
        expires_at > clock.now_ms(),
        "expires {expires_at} now {}",
        clock.now_ms()
    );
    // Within its lifetime the handle works.
    let second = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({ "handle": first.handle(), "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(!second.is_error(), "{:?}", second.response());
    assert_eq!(
        second
            .structured()
            .and_then(|structured| structured["expiresAt"].as_u64()),
        Some(expires_at),
        "a successor keeps the mount's expiry, as a browser snapshot does"
    );
    // Past it, refused.
    clock.advance_ms(expires_at - clock.now_ms() + 1);
    let expired = client
        .mcp()
        .as_caller(alice())
        .call(
            "counter.add",
            json!({ "handle": second.handle(), "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(expired.is_error(), "{:?}", expired.response());
    assert!(expired.text().contains("not valid"), "{}", expired.text());
}

#[tokio::test]
async fn agent_013_an_action_without_headless_is_not_offered() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    for caller in [ToolCaller::guest(), alice()] {
        let listed = client.mcp().as_caller(caller.clone()).list().await;
        assert!(
            !listed
                .tool_names()
                .iter()
                .any(|name| name.starts_with("counter.peek")),
            "{:?}",
            listed.tool_names()
        );
        let call = client
            .mcp()
            .as_caller(caller)
            .call("counter.peek", json!({}))
            .await;
        assert!(call.structured().is_none());
        assert!(call.error_code().is_some(), "{:?}", call.response());
    }
}

#[tokio::test]
async fn agent_013_a_wire_request_to_the_agent_route_is_refused() {
    let _container = fixture::setup();
    live_fixture::register_components();
    live_fixture::define_live_policies();
    // No global middleware, so nothing but the route itself answers: a
    // request off the wire, even one naming a tool, never runs.
    let router = live_fixture::live_router_with(AgentsConfig::new());
    suprnova::live::testing::prepare_live_router_for_test(&router)
        .expect("prepare the Live runtime");
    let client = TestClient::new(router, suprnova::MiddlewareRegistry::new());
    let posted = client
        .post("/__live/agent")
        .json(&fresh(1, 1))
        .header("Suprnova-Tool-Call", "counter.add")
        .send()
        .await;
    assert_eq!(posted.status(), 404, "{}", posted.body_text());
    let fetched = client.get("/__live/agent").send().await;
    assert_eq!(fetched.status(), 405);
    assert_eq!(RESOLVED.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn agent_013_headless_runs_the_live_route_guard() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let before = GUARDED.load(Ordering::SeqCst);
    let result = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    assert!(
        GUARDED.load(Ordering::SeqCst) > before,
        "the application's Live guard ran on the call"
    );
}

#[tokio::test]
async fn agent_018_live_tool_through_mcp_and_the_browser_endpoint() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    // Over MCP, as a user and as an agent account bound to one.
    let user = client
        .mcp()
        .as_caller(alice())
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!user.is_error(), "{:?}", user.response());
    assert!(user.handle().is_some());
    let agent = client
        .mcp()
        .as_caller(
            ToolCaller::agent("reg-1")
                .claimed(true)
                .bound_to(TestUser::arc("alice")),
        )
        .call("counter.add", fresh(1, 1))
        .await;
    assert!(!agent.is_error(), "{:?}", agent.response());

    // Through the browser endpoint, as the page's guest: the island's
    // snapshot, the session and the CSRF proof a click carries.
    let page = client.get(PAGE).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let html = page.body_text();
    assert!(html.contains("counter.peek.main"), "{html}");
    let called = client
        .live_tool("counter.peek.main", &html, json!({}))
        .send()
        .await;
    assert_eq!(called.status(), 200, "{}", called.body_text());
    let answer: Value = serde_json::from_str(&called.body_text()).expect("a Live answer");
    assert_eq!(answer["outcome"], "accepted", "{answer}");
    // The header must name the action the request runs: a counted tool
    // name cannot cover another action of the island.
    let mismatched = client
        .live_tool("counter.peek.main", &html, json!({}))
        .header("Suprnova-Tool-Call", "counter.peek.other")
        .send()
        .await;
    assert_ne!(mismatched.status(), 200, "{}", mismatched.body_text());
}
