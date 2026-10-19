//! The crate's own rules, checked without a browser: the shared browser
//! call fixture, the server tools element, a page tool's descriptor, how a
//! page tool's call reaches its handler, and the names its validation
//! errors use (AGENT-016).

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use serde_json::{Value, json};

use crate::result::{self, Answer};
use crate::server::{self, ServerTools};
use crate::{PageTool, ToolError, ToolResult, WebMcpError, csrf, input_schema, tool_input};

/// The cases every browser implementation of a server tool call shares.
const FIXTURE: &str = include_str!("../../../framework/tests/agents/fixtures/browser-calls.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("the shared fixture is JSON")
}

fn cases<'a>(fixture: &'a Value, list: &str) -> &'a [Value] {
    fixture[list]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("the fixture has a `{list}` list"))
}

/// The value of a result as the page hands it to WebMCP.
fn shown(result: &ToolResult) -> Value {
    serde_json::to_value(result).expect("a tool result serializes")
}

/// Run `future`, which never waits on anything outside itself.
fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future waited on something outside itself"),
    }
}

fn path_case(case: &Value) -> Result<(), String> {
    let payload = json!({
        "v": 1,
        "limits": { "maxResultBytes": 8_388_608, "callTimeoutMs": 30_000 },
        "tools": [{
            "name": "fixture.tool",
            "description": "A tool of the shared fixture.",
            "inputSchema": { "type": "object" },
            "annotations": {
                "readOnlyHint": false,
                "untrustedContentHint": false,
                "consequentialHint": false
            },
            "method": case["method"],
            "path": case["path"],
        }],
    });
    let tools = ServerTools::parse(&payload.to_string()).map_err(|error| error.to_string())?;
    let tool = tools.tool("fixture.tool").ok_or("the tool is missing")?;
    let expected = &case["expected"];
    match server::plan(tool, &case["arguments"]) {
        Ok(plan) => {
            if !expected["error"].is_null() {
                return Err(format!(
                    "planned {} where an error was expected",
                    plan.target
                ));
            }
            if plan.target != expected["url"] {
                return Err(format!("the URL is {}", plan.target));
            }
            let body = match &plan.body {
                Some(text) => serde_json::from_str::<Value>(text).map_err(|e| e.to_string())?,
                None => Value::Null,
            };
            if body != expected["body"] {
                return Err(format!("the body is {body}"));
            }
            Ok(())
        }
        Err(refused) => {
            if shown(&refused) == expected["error"] {
                Ok(())
            } else {
                Err(format!("the result is {}", shown(&refused)))
            }
        }
    }
}

fn csrf_case(case: &Value) -> Result<(), String> {
    let header = csrf::header(case["meta"].as_str(), case["cookie"].as_str().unwrap_or(""));
    let got = match header {
        Some((name, value)) => json!({ name: value }),
        None => json!({}),
    };
    if got == case["expected"] {
        Ok(())
    } else {
        Err(format!("the header is {got}"))
    }
}

fn result_case(case: &Value) -> Result<(), String> {
    let response = &case["response"];
    let max_bytes = case["maxResultBytes"]
        .as_u64()
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or("the case has no maxResultBytes")?;
    let got = match response["failure"].as_str() {
        Some("network") => result::unreachable(),
        Some("timeout") => result::timed_out(
            response["callTimeoutMs"]
                .as_u64()
                .ok_or("the case has no callTimeoutMs")?,
        ),
        Some(other) => return Err(format!("unknown failure {other}")),
        None => {
            let body = response["body"].as_str().ok_or("the case has no body")?;
            let answer = Answer {
                status: response["status"]
                    .as_u64()
                    .and_then(|status| u16::try_from(status).ok())
                    .ok_or("the case has no status")?,
                opaque_redirect: response["type"] == "opaqueredirect",
                content_type: response["contentType"].as_str(),
                body: body.as_bytes(),
            };
            result::answer_result(&answer, max_bytes)
        }
    };
    if shown(&got) == case["expected"] {
        Ok(())
    } else {
        Err(format!("the result is {}", shown(&got)))
    }
}

/// A check of one fixture case, with the reason it failed.
type CaseCheck = fn(&Value) -> Result<(), String>;

#[test]
fn agent_016_path_csrf_and_results_match_the_browser_fixture() {
    let fixture = fixture();
    let mut failures = Vec::new();
    let lists: [(&str, CaseCheck, usize); 3] = [
        ("paths", path_case, 27),
        ("csrf", csrf_case, 7),
        ("results", result_case, 31),
    ];
    for (list, check, count) in lists {
        let cases = cases(&fixture, list);
        assert_eq!(
            cases.len(),
            count,
            "the fixture's `{list}` list changed size"
        );
        for case in cases {
            if let Err(reason) = check(case) {
                failures.push(format!("{list}: {}: {reason}", case["name"]));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The page tools element of the contract (section 0.1), one tool.
fn element() -> Value {
    json!({
        "v": 1,
        "limits": { "maxResultBytes": 8_388_608, "callTimeoutMs": 30_000 },
        "tools": [{
            "name": "posts.create",
            "title": "Create a post",
            "description": "Create a draft post for the signed-in user.",
            "inputSchema": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object"
            },
            "annotations": {
                "readOnlyHint": false,
                "untrustedContentHint": false,
                "consequentialHint": false
            },
            "method": "POST",
            "path": [{ "text": "/app/posts/" }, { "param": "post", "optional": false, "catchAll": false }]
        }]
    })
}

/// A change that breaks the element.
type Breakage = Box<dyn Fn(&mut Value)>;

#[test]
fn agent_016_server_tools_element_is_read_strictly() {
    let tools = ServerTools::parse(&element().to_string()).expect("the contract's element reads");
    assert_eq!(tools.names().collect::<Vec<_>>(), vec!["posts.create"]);

    let mut too_many = element();
    let one = too_many["tools"][0].clone();
    too_many["tools"] = Value::Array(
        (0..65)
            .map(|index| {
                let mut tool = one.clone();
                tool["name"] = json!(format!("posts.tool{index}"));
                tool
            })
            .collect(),
    );
    let mut duplicate = element();
    duplicate["tools"] = json!([one.clone(), one.clone()]);

    let broken: Vec<(&str, Breakage)> = vec![
        ("an unknown key", Box::new(|e| e["extra"] = json!(1))),
        ("another version", Box::new(|e| e["v"] = json!(2))),
        ("no limits", Box::new(|e| e["limits"] = Value::Null)),
        (
            "a zero result bound",
            Box::new(|e| e["limits"]["maxResultBytes"] = json!(0)),
        ),
        (
            "a zero timeout",
            Box::new(|e| e["limits"]["callTimeoutMs"] = json!(0)),
        ),
        (
            "an unknown limit",
            Box::new(|e| e["limits"]["retries"] = json!(1)),
        ),
        (
            "an unknown tool key",
            Box::new(|e| e["tools"][0]["outputSchema"] = json!({})),
        ),
        (
            "a name outside the grammar",
            Box::new(|e| e["tools"][0]["name"] = json!("posts create")),
        ),
        (
            "a blank title",
            Box::new(|e| e["tools"][0]["title"] = json!("  ")),
        ),
        (
            "a blank description",
            Box::new(|e| e["tools"][0]["description"] = json!(" ")),
        ),
        (
            "a schema that is not an object",
            Box::new(|e| e["tools"][0]["inputSchema"] = json!([])),
        ),
        (
            "a missing hint",
            Box::new(|e| {
                e["tools"][0]["annotations"] = json!({
                    "readOnlyHint": false,
                    "untrustedContentHint": false
                });
            }),
        ),
        (
            "an extra hint",
            Box::new(|e| e["tools"][0]["annotations"]["destructiveHint"] = json!(true)),
        ),
        (
            "a hint that is not a boolean",
            Box::new(|e| e["tools"][0]["annotations"]["readOnlyHint"] = json!("yes")),
        ),
        (
            "an unknown method",
            Box::new(|e| e["tools"][0]["method"] = json!("TRACE")),
        ),
        (
            "an empty path",
            Box::new(|e| e["tools"][0]["path"] = json!([])),
        ),
        (
            "a path that starts with a parameter",
            Box::new(|e| {
                e["tools"][0]["path"] =
                    json!([{ "param": "post", "optional": false, "catchAll": false }]);
            }),
        ),
        (
            "a path that does not start at the root",
            Box::new(|e| e["tools"][0]["path"] = json!([{ "text": "posts" }])),
        ),
        (
            "a path that names another host",
            Box::new(|e| e["tools"][0]["path"] = json!([{ "text": "//evil.example/posts" }])),
        ),
        (
            "an empty text piece",
            Box::new(|e| e["tools"][0]["path"][1] = json!({ "text": "" })),
        ),
        (
            "a parameter piece with an unknown key",
            Box::new(|e| e["tools"][0]["path"][1]["pattern"] = json!(".*")),
        ),
        (
            "a parameter without a name",
            Box::new(|e| e["tools"][0]["path"][1]["param"] = json!("")),
        ),
        (
            "a tool list that is not a list",
            Box::new(|e| e["tools"] = json!({})),
        ),
    ];
    for (why, change) in broken {
        let mut payload = element();
        change(&mut payload);
        assert!(
            matches!(
                ServerTools::parse(&payload.to_string()),
                Err(WebMcpError::InvalidPage(_))
            ),
            "{why} must not read"
        );
    }
    for (why, payload) in [("sixty-five tools", too_many), ("a name twice", duplicate)] {
        assert!(
            matches!(
                ServerTools::parse(&payload.to_string()),
                Err(WebMcpError::InvalidPage(_))
            ),
            "{why} must not read"
        );
    }
    assert!(matches!(
        ServerTools::parse("not json"),
        Err(WebMcpError::InvalidPage(_))
    ));
}

#[test]
fn agent_016_an_element_with_no_tools_is_refused() {
    // The server leaves the element out when a page offers no tool, so an
    // empty list is not what it writes.
    let mut payload = element();
    payload["tools"] = json!([]);
    assert!(
        matches!(
            ServerTools::parse(&payload.to_string()),
            Err(WebMcpError::InvalidPage(reason)) if reason.contains("no tools")
        ),
        "an element with no tools must not read"
    );
}

#[test]
fn agent_016_an_element_with_a_null_title_is_refused() {
    // The server leaves `title` out when a tool declares none, so a null is
    // not what it writes.
    let mut payload = element();
    payload["tools"][0]["title"] = Value::Null;
    assert!(
        matches!(
            ServerTools::parse(&payload.to_string()),
            Err(WebMcpError::InvalidPage(_))
        ),
        "a null title must not read"
    );
    let mut untitled = element();
    if let Some(tool) = untitled["tools"][0].as_object_mut() {
        tool.remove("title");
    }
    assert!(
        ServerTools::parse(&untitled.to_string()).is_ok(),
        "a tool without a title reads"
    );
}

/// The input of a typed page tool.
#[tool_input]
#[derive(Debug)]
struct AddTodo {
    /// What to do.
    #[validate(length(min = 1, max = 20))]
    title: String,
    /// How urgent, 1 to 3.
    #[validate(range(min = 1, max = 3))]
    #[serde(default)]
    urgency: Option<u8>,
}

#[test]
fn agent_016_page_tool_descriptor_carries_the_derived_schema_and_all_three_hints() {
    let descriptor = PageTool::<AddTodo>::new("todos.add", "Add an item to the todo list.")
        .title("Add todo")
        .read_only(false)
        .untrusted_content(true)
        .consequential(true)
        .descriptor()
        .expect("a valid tool has a descriptor");
    assert_eq!(
        descriptor,
        json!({
            "name": "todos.add",
            "title": "Add todo",
            "description": "Add an item to the todo list.",
            "inputSchema": input_schema::<AddTodo>(),
            "annotations": {
                "readOnlyHint": false,
                "untrustedContentHint": true,
                "consequentialHint": true
            }
        })
    );
    assert_eq!(
        descriptor["inputSchema"]["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        descriptor["inputSchema"]["properties"]["title"]["maxLength"],
        20
    );

    let plain = PageTool::<AddTodo>::new("todos.add", "Add an item to the todo list.")
        .descriptor()
        .expect("a valid tool has a descriptor");
    assert!(plain.get("title").is_none(), "{plain:#}");
    assert_eq!(
        plain["annotations"],
        json!({
            "readOnlyHint": false,
            "untrustedContentHint": false,
            "consequentialHint": false
        })
    );
}

async fn add(input: AddTodo) -> Result<ToolResult, ToolError> {
    if input.title == "fail" {
        return Err(ToolError::from("the list is\nfull"));
    }
    Ok(ToolResult::json(&json!({ "added": input.title }))?)
}

#[test]
fn agent_016_page_tool_checks_its_input_before_the_handler() {
    let done = ready(crate::tool::run(&add, r#"{"title":"Buy milk"}"#));
    assert_eq!(
        shown(&done),
        json!({
            "content": [{ "type": "text", "text": r#"{"added":"Buy milk"}"# }],
            "structuredContent": { "added": "Buy milk" },
            "isError": false
        })
    );

    let wrong_type = ready(crate::tool::run(&add, r#"{"title":7}"#));
    assert!(wrong_type.is_error);
    assert!(
        matches!(&wrong_type.content[..], [crate::Content::Text { text }] if text.starts_with("Invalid input: invalid type")),
        "{wrong_type:?}"
    );

    let invalid = ready(crate::tool::run(&add, r#"{"title":"","urgency":9}"#));
    assert_eq!(
        shown(&invalid),
        json!({
            "content": [{
                "type": "text",
                "text": "The input is not valid:\n- title: The title field must be between 1 and 20 characters.\n- urgency: The urgency field must be between 1 and 3."
            }],
            "isError": true
        })
    );

    let failed = ready(crate::tool::run(&add, r#"{"title":"fail"}"#));
    assert_eq!(
        shown(&failed),
        json!({
            "content": [{ "type": "text", "text": "the list is full" }],
            "isError": true
        })
    );
}

/// A line of an order, its fields renamed as the client sends them.
#[tool_input]
#[serde(rename_all = "camelCase")]
struct OrderLine {
    /// The price of one, in cents.
    #[validate(range(min = 1))]
    unit_price: u32,
}

/// Where an order goes.
#[tool_input]
#[serde(rename_all = "camelCase")]
struct Destination {
    /// The street.
    #[validate(length(min = 1))]
    street_name: String,
}

/// An order, with a renamed field, a renamed nested list and a renamed
/// nested object.
#[tool_input]
#[serde(rename_all = "camelCase")]
struct PlaceOrder {
    /// Who orders.
    #[validate(length(min = 1))]
    customer_name: String,
    /// What is ordered.
    #[serde(rename = "items")]
    #[validate(nested)]
    line_items: Vec<OrderLine>,
    /// Where it goes.
    #[validate(nested)]
    ship_to: Option<Destination>,
}

async fn place(_order: PlaceOrder) -> Result<ToolResult, ToolError> {
    Ok(ToolResult::text("Placed."))
}

#[test]
fn agent_016_validation_errors_use_the_clients_field_names() {
    // The server's form request reports each field by the key the client
    // sent, list positions as their own segment, so the agent reads one
    // naming whichever side checked the input.
    let invalid = ready(crate::tool::run(
        &place,
        r#"{"customerName":"","items":[{"unitPrice":5},{"unitPrice":0}],"shipTo":{"streetName":""}}"#,
    ));
    assert_eq!(
        shown(&invalid),
        json!({
            "content": [{
                "type": "text",
                "text": "The input is not valid:\n\
                         - customerName: The customer name field must be at least 1 characters.\n\
                         - items.1.unitPrice: The items.1.unit price field must be at least 1.\n\
                         - shipTo.streetName: The ship to.street name field must be at least 1 \
                         characters."
            }],
            "isError": true
        })
    );
    let valid = ready(crate::tool::run(
        &place,
        r#"{"customerName":"Ada","items":[{"unitPrice":5}]}"#,
    ));
    assert!(!valid.is_error, "{valid:?}");
}

/// Signing up, with one rule of each kind the server's catalog words.
#[tool_input]
#[serde(rename_all = "camelCase")]
struct SignUp {
    /// The name to show.
    #[validate(length(min = 3, max = 20))]
    display_name: String,
    /// Age in years.
    #[validate(range(min = 18, max = 120))]
    age_years: u32,
    /// Where to write.
    #[validate(email)]
    contact_email: String,
    /// What friends call you.
    #[validate(length(min = 1, message = "Say what friends call you."))]
    nickname: String,
}

async fn sign_up(_input: SignUp) -> Result<ToolResult, ToolError> {
    Ok(ToolResult::text("Signed up."))
}

#[test]
fn agent_016_validation_messages_take_the_servers_english_text() {
    // The texts the server's English catalog gives for the same rules,
    // with the client's field name humanized as the server humanizes it;
    // the framework's `wasm_messages` test pins the server to the same
    // texts. An author's own message still wins.
    let invalid = ready(crate::tool::run(
        &sign_up,
        r#"{"displayName":"Al","ageYears":12,"contactEmail":"not-an-address","nickname":""}"#,
    ));
    assert_eq!(
        shown(&invalid),
        json!({
            "content": [{
                "type": "text",
                "text": "The input is not valid:\n\
                         - ageYears: The age years field must be between 18 and 120.\n\
                         - contactEmail: The contact email field must be a valid email address.\n\
                         - displayName: The display name field must be between 3 and 20 \
                         characters.\n\
                         - nickname: Say what friends call you."
            }],
            "isError": true
        })
    );
}
