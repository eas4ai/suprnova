//! AGENT-005, AGENT-008, AGENT-009 and AGENT-017: what an application's own
//! consent pages are built on. [`AgentApprovals`] and [`AgentClaims`] read
//! and record the user's decisions, driven here through routes shaped like
//! a kit's handlers, and [`ConsentPageHeaders`] makes every answer of a
//! consent page unframeable, uncached and referrer-free under each
//! security headers preset.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use suprnova::agents::{
    AgentApprovals, AgentAuth, AgentClaims, AgentsConfig, ApprovalDecision, ApprovalLookup,
    ClaimConfirmation, ClaimLookup, ConsentPageHeaders, OAuthServer,
};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    ContentSecurityPolicy, FrameworkError, HttpResponse, Inertia, Middleware, MiddlewareRegistry,
    Next, Request, Response, Router, SecurityHeaders, SecurityHeadersOverride, ToolCaller, handler,
    tool,
};

use crate::agent_auth::{
    AgentWorld, PROVIDER_NAME, RESOURCE, agent_database, agent_world_routed, anonymous,
    id_jag_claims, present, service_auth, with_id_jags,
};
use crate::oauth::{
    World, account_email, authorize_path, container, database, exchange, location, mark_unverified,
    new_account, pkce, query_param, register, registry, router, sign_in, world_routed,
};

/// A loopback client's redirect URI: another origin than the
/// application's, as every client's is.
const LOOPBACK_REDIRECT: &str = "http://127.0.0.1:7777/callback";
/// Where the authorization endpoint sends the user: the application's own
/// approval page.
const APPROVAL_PAGE: &str = "/consent/approve";

// ------------------------------------------------- the kit's handlers ----

#[derive(Deserialize)]
struct DecisionForm {
    authorization: String,
    decision: String,
}

#[derive(Deserialize)]
struct CodeForm {
    claim_attempt_token: String,
    user_code: String,
}

fn approval_json(lookup: ApprovalLookup) -> Value {
    match lookup {
        ApprovalLookup::Pending(approval) => json!({ "lookup": "pending", "approval": approval }),
        ApprovalLookup::Ended => json!({ "lookup": "ended" }),
        ApprovalLookup::SignIn => json!({ "lookup": "sign_in" }),
    }
}

fn decision_json(decision: ApprovalDecision) -> Value {
    match decision {
        ApprovalDecision::Redirect(url) => json!({ "decision": "redirect", "url": url }),
        ApprovalDecision::Ended => json!({ "decision": "ended" }),
        ApprovalDecision::SignIn => json!({ "decision": "sign_in" }),
    }
}

fn claim_json(lookup: ClaimLookup) -> Value {
    match lookup {
        ClaimLookup::Pending(claim) => json!({ "lookup": "pending", "claim": claim }),
        ClaimLookup::Refused(refusal) => {
            json!({ "lookup": "refused", "refusal": format!("{refusal:?}") })
        }
    }
}

fn confirmation_json(confirmation: ClaimConfirmation) -> Value {
    match confirmation {
        ClaimConfirmation::Claimed { registration_id } => {
            json!({ "confirmation": "claimed", "registration_id": registration_id })
        }
        ClaimConfirmation::WrongCode { attempts_remaining } => {
            json!({ "confirmation": "wrong_code", "attempts_remaining": attempts_remaining })
        }
        ClaimConfirmation::Refused(refusal) => {
            json!({ "confirmation": "refused", "refusal": format!("{refusal:?}") })
        }
    }
}

fn failed(error: FrameworkError) -> HttpResponse {
    HttpResponse::from(error)
}

/// The routes an application's own approval and claim pages would be,
/// answering JSON so a test reads exactly what the facade gave. Each
/// `POST` takes its handle before it reads the form, as a kit's handler
/// that reads a form request after it does.
fn consent_routes(router: Router) -> Router {
    router
        .get("/test/consent/approval", |request: Request| async move {
            let id = request.query_param("authorization").unwrap_or_default();
            let approvals = AgentApprovals::of(&request).map_err(failed)?;
            let lookup = approvals.pending(&id).await.map_err(failed)?;
            Ok(HttpResponse::json(approval_json(lookup)))
        })
        .post("/test/consent/decide", |request: Request| async move {
            let approvals = AgentApprovals::of(&request).map_err(failed)?;
            let form: DecisionForm = request.form().await.map_err(failed)?;
            let decision = approvals
                .decide(&form.authorization, form.decision == "approve")
                .await
                .map_err(failed)?;
            Ok(HttpResponse::json(decision_json(decision)))
        })
        // As a kit routes its consent pages.
        .middleware(ConsentPageHeaders)
        .post("/test/consent/approve-tool", approve_as_tool)
        .get("/test/consent/claim", |request: Request| async move {
            let token = request
                .query_param("claim_attempt_token")
                .unwrap_or_default();
            let claims = AgentClaims::of(&request).map_err(failed)?;
            let lookup = claims.pending(&token).await.map_err(failed)?;
            Ok(HttpResponse::json(claim_json(lookup)))
        })
        .post("/test/consent/confirm", |request: Request| async move {
            let claims = AgentClaims::of(&request).map_err(failed)?;
            let form: CodeForm = request.form().await.map_err(failed)?;
            let confirmation = claims
                .confirm(&form.claim_attempt_token, &form.user_code)
                .await
                .map_err(failed)?;
            Ok(HttpResponse::json(confirmation_json(confirmation)))
        })
        .get("/test/consent/logout", |_request: Request| async move {
            // The session forgets its user, as a sign-out does.
            suprnova::session::clear_auth_user();
            Ok(HttpResponse::text("signed out"))
        })
        .into()
}

/// The pending authorization the tool below approves. The tool takes no
/// input, since a tool's input is a form request and a decision handler
/// reads the request itself, so the test names its target here.
static TOOL_TARGET: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// An application that declares a decision route as a tool, which no
/// application should: an agent's call of it must never approve.
#[tool(
    name = "approve_consent",
    description = "Approve a pending authorization.",
    callers = [user],
)]
#[handler]
pub async fn approve_as_tool(request: Request) -> Response {
    let approvals = AgentApprovals::of(&request)?;
    let id = TOOL_TARGET
        .lock()
        .map(|target| target.clone())
        .unwrap_or_default();
    let decision = approvals.decide(&id, true).await?;
    Ok(HttpResponse::json(decision_json(decision)))
}

// ------------------------------------------------------------ steps ----

/// A world whose OAuth server sends users to the application's own
/// approval page, with the kit's handlers routed.
async fn approval_world() -> World {
    world_routed(
        |server| server.custom_approval_page(APPROVAL_PAGE),
        |agents| agents,
        consent_routes,
    )
    .await
}

/// A world with agent auth on, as `configure` adjusts it, and the kit's
/// handlers routed.
async fn claim_world(configure: impl FnOnce(AgentAuth) -> AgentAuth) -> AgentWorld {
    agent_world_routed(
        container(),
        agent_database().await,
        configure,
        consent_routes,
    )
}

/// The session's CSRF token.
async fn csrf(client: &TestClient) -> String {
    client.get("/test/csrf").send().await.body_text()
}

/// Send the browser to the authorization endpoint as the signed-in user;
/// the pending authorization's id the application's page is named with.
async fn authorize(client: &TestClient, client_id: &str, challenge: &str) -> String {
    let authorized = client
        .get(authorize_path(
            client_id,
            LOOPBACK_REDIRECT,
            challenge,
            "posts:read posts:write",
        ))
        .send()
        .await;
    assert_eq!(authorized.status(), 302, "{}", authorized.body_text());
    let to = location(&authorized);
    assert!(
        to.starts_with(&format!("http://localhost{APPROVAL_PAGE}?authorization=")),
        "{to}"
    );
    query_param(&to, "authorization").expect("the id")
}

async fn lookup_approval(client: &TestClient, id: &str) -> Value {
    let response = client
        .get(format!("/test/consent/approval?authorization={id}"))
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

async fn decide(client: &TestClient, id: &str, decision: &str) -> Value {
    let token = csrf(client).await;
    let response = client
        .post("/test/consent/decide")
        .form(&[
            ("_token", token.as_str()),
            ("authorization", id),
            ("decision", decision),
        ])
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

async fn lookup_claim(client: &TestClient, token: &str) -> Value {
    let response = client
        .get(format!(
            "/test/consent/claim?claim_attempt_token={}",
            url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
        ))
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

async fn confirm_claim(client: &TestClient, token: &str, code: &str) -> Value {
    let csrf = csrf(client).await;
    let response = client
        .post("/test/consent/confirm")
        .form(&[
            ("_token", csrf.as_str()),
            ("claim_attempt_token", token),
            ("user_code", code),
        ])
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

async fn sign_out(client: &TestClient) {
    client.get("/test/consent/logout").send().await.assert_ok();
}

/// The attempt token and the code of a `service_auth` registration's
/// claim.
fn attempt_of(registration: &Value) -> (String, String) {
    let claim = &registration["claim"];
    let token = query_param(
        claim["verification_uri"]
            .as_str()
            .expect("a verification URI"),
        "claim_attempt_token",
    )
    .expect("the attempt token");
    (
        token,
        claim["user_code"].as_str().expect("a code").to_owned(),
    )
}

/// A six-digit code other than `code`.
fn wrong(code: &str) -> &'static str {
    if code == "000000" { "111111" } else { "000000" }
}

// ---------------------------------------------------------- approvals ----

#[tokio::test]
async fn agent_017_approvals_read_and_decide_for_the_signed_in_user() {
    let world = approval_world().await;
    let client = &world.client;
    let user = new_account("facade-approve");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (verifier, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;

    let pending = lookup_approval(client, &id).await;
    assert_eq!(pending["lookup"], "pending", "{pending}");
    let approval = &pending["approval"];
    assert_eq!(approval["id"], id.as_str());
    assert_eq!(approval["client_name"], "Example Tool");
    assert_eq!(approval["client_host"], "127.0.0.1");
    assert_eq!(approval["redirect_uri"], LOOPBACK_REDIRECT);
    assert_eq!(
        approval["scopes"],
        json!([
            { "name": "posts:read", "description": "Read your posts" },
            { "name": "posts:write", "description": "Create and edit your posts" },
        ])
    );
    let expires: chrono::DateTime<chrono::Utc> = approval["expires_at"]
        .as_str()
        .and_then(|at| at.parse().ok())
        .expect("an RFC 3339 expiry");
    assert!(expires > chrono::Utc::now());

    // Approving sends the browser to the client with a code that works.
    let approved = decide(client, &id, "approve").await;
    assert_eq!(approved["decision"], "redirect", "{approved}");
    let url = approved["url"].as_str().expect("a URL");
    assert!(url.starts_with(LOOPBACK_REDIRECT), "{url}");
    assert_eq!(query_param(url, "state").as_deref(), Some("state-123"));
    assert_eq!(query_param(url, "iss").as_deref(), Some("http://localhost"));
    let code = query_param(url, "code").expect("a code");
    let tokens = exchange(client, &client_id, LOOPBACK_REDIRECT, &code, &verifier).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());

    // The authorization is spent: neither read nor decided again.
    assert_eq!(lookup_approval(client, &id).await["lookup"], "ended");
    assert_eq!(decide(client, &id, "approve").await["decision"], "ended");

    // Denying sends `access_denied`, with no code.
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;
    let denied = decide(client, &id, "deny").await;
    assert_eq!(denied["decision"], "redirect", "{denied}");
    let url = denied["url"].as_str().expect("a URL");
    assert!(url.starts_with(LOOPBACK_REDIRECT), "{url}");
    assert_eq!(query_param(url, "error").as_deref(), Some("access_denied"));
    assert_eq!(query_param(url, "state").as_deref(), Some("state-123"));
    assert_eq!(query_param(url, "iss").as_deref(), Some("http://localhost"));
    assert_eq!(query_param(url, "code"), None);
}

#[tokio::test]
async fn agent_017_approvals_end_for_another_users_authorization() {
    let world = approval_world().await;
    let client = &world.client;
    let owner = new_account("facade-owner");
    let other = new_account("facade-other");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &owner).await;
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;

    sign_in(client, &other).await;
    let seen = lookup_approval(client, &id).await;
    assert_eq!(seen, json!({ "lookup": "ended" }), "nothing of it shows");
    assert_eq!(
        decide(client, &id, "approve").await,
        json!({ "decision": "ended" })
    );

    // The other user's attempts spent nothing: the owner still decides.
    sign_in(client, &owner).await;
    assert_eq!(lookup_approval(client, &id).await["lookup"], "pending");
    assert_eq!(decide(client, &id, "approve").await["decision"], "redirect");
}

#[tokio::test]
async fn agent_017_approvals_ask_a_guest_to_sign_in() {
    let world = approval_world().await;
    let client = &world.client;
    let user = new_account("facade-guest");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    // A guest is asked to sign in before any id is read, a made-up one too.
    for id in ["made-up", ""] {
        assert_eq!(
            lookup_approval(client, id).await,
            json!({ "lookup": "sign_in" })
        );
    }
    assert_eq!(
        decide(client, "made-up", "approve").await,
        json!({ "decision": "sign_in" })
    );
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;
    sign_out(client).await;
    assert_eq!(
        lookup_approval(client, &id).await,
        json!({ "lookup": "sign_in" })
    );
    assert_eq!(
        decide(client, &id, "approve").await,
        json!({ "decision": "sign_in" })
    );
    // Signed in again, the authorization is still there to decide.
    sign_in(client, &user).await;
    assert_eq!(decide(client, &id, "deny").await["decision"], "redirect");
}

#[tokio::test]
async fn agent_017_pending_approval_carries_no_client_chosen_url() {
    const CLIENT: &str = "https://tools.client.test/oauth/client.json";
    const REDIRECT: &str = "https://tools.client.test/callback";
    const HOME: &str = "https://tools.client.test/home";
    const LOGO: &str = "https://tools.client.test/logo.png";
    let world = approval_world().await;
    let client = &world.client;
    world.resolver.answer("tools.client.test", "93.184.215.14");
    world.documents.serve(
        CLIENT,
        &json!({
            "client_id": CLIENT,
            "client_name": "Document Tool",
            "client_uri": HOME,
            "logo_uri": LOGO,
            "redirect_uris": [REDIRECT],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
        }),
    );
    let user = new_account("facade-document");
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let authorized = client
        .get(authorize_path(CLIENT, REDIRECT, &challenge, "posts:read"))
        .send()
        .await;
    assert_eq!(authorized.status(), 302, "{}", authorized.body_text());
    let id = query_param(&location(&authorized), "authorization").expect("the id");
    let pending = lookup_approval(client, &id).await;
    let approval = pending["approval"].as_object().expect("a pending approval");
    let mut fields: Vec<&str> = approval.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "client_host",
            "client_name",
            "expires_at",
            "id",
            "redirect_uri",
            "scopes"
        ]
    );
    assert_eq!(approval["client_host"], "tools.client.test");
    let text = pending.to_string();
    for chosen in [HOME, LOGO, CLIENT] {
        assert!(!text.contains(chosen), "{chosen} in {text}");
    }
}

#[tokio::test]
async fn agent_017_facade_needs_the_plugins_mounted() {
    let world = approval_world().await;
    // No OAuth server: neither half has anything to act on.
    let bare = TestClient::new(consent_routes(Router::new()), registry());
    let approval = bare
        .get("/test/consent/approval?authorization=x")
        .send()
        .await;
    assert_eq!(approval.status(), 500, "{}", approval.body_text());
    let claim = bare
        .get("/test/consent/claim?claim_attempt_token=x")
        .send()
        .await;
    assert_eq!(claim.status(), 500, "{}", claim.body_text());
    // The OAuth server without agent auth serves approvals, not claims.
    let claim = world
        .client
        .get("/test/consent/claim?claim_attempt_token=x")
        .send()
        .await;
    assert_eq!(claim.status(), 500, "{}", claim.body_text());
}

#[tokio::test]
async fn agent_017_consent_refuses_an_agents_tool_call() {
    let world = approval_world().await;
    let client = &world.client;
    let user = new_account("facade-tool");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;

    if let Ok(mut target) = TOOL_TARGET.lock() {
        *target = id.clone();
    }

    // A browser tool call, with the user's session.
    let browser = client.tool("approve_consent", json!({})).send().await;
    assert_eq!(browser.status(), 403, "{}", browser.body_text());
    assert!(
        browser.body_text().contains("person"),
        "{}",
        browser.body_text()
    );

    // A call through the MCP endpoint, as the same user.
    let mcp = client
        .mcp()
        .as_caller(ToolCaller::user(crate::fixture::TestUser::arc(&user)))
        .call("approve_consent", json!({}))
        .await;
    // The endpoint reports the route's refusal as a failed call.
    assert!(mcp.is_error(), "{:?}", mcp.response());
    assert!(mcp.text().contains("person"), "{}", mcp.text());

    // Neither touched the pending authorization.
    assert_eq!(lookup_approval(client, &id).await["lookup"], "pending");
}

#[tokio::test]
async fn agent_017_a_decision_without_the_csrf_token_is_refused() {
    let world = approval_world().await;
    let client = &world.client;
    let user = new_account("facade-csrf");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;
    let refused = client
        .post("/test/consent/decide")
        .form(&[("authorization", id.as_str()), ("decision", "approve")])
        .send()
        .await;
    assert_eq!(refused.status(), 419, "{}", refused.body_text());
    assert_consent_headers(&refused, "a CSRF refusal");
    assert_eq!(lookup_approval(client, &id).await["lookup"], "pending");
}

/// The header a kit's form tool submits with: an agent filling and sending
/// a page's own form.
const FORM_TOOL: &str = "Suprnova-Form-Tool";

#[tokio::test]
async fn agent_017_consent_refuses_a_form_tool_submission() {
    let world = approval_world().await;
    let client = &world.client;
    let user = new_account("facade-form-tool");
    let client_id = register(client, LOOPBACK_REDIRECT).await;
    sign_in(client, &user).await;
    let (_, challenge) = pkce();
    let id = authorize(client, &client_id, &challenge).await;
    // The user's own session and CSRF token, as a form tool on one of the
    // application's pages sends them: only the header tells it apart.
    let token = csrf(client).await;
    let refused = client
        .post("/test/consent/decide")
        .header(FORM_TOOL, "1")
        .form(&[
            ("_token", token.as_str()),
            ("authorization", id.as_str()),
            ("decision", "approve"),
        ])
        .send()
        .await;
    assert_eq!(refused.status(), 403, "{}", refused.body_text());
    assert!(
        refused.body_text().contains("person"),
        "{}",
        refused.body_text()
    );
    assert_consent_headers(&refused, "a form tool's refusal");
    assert_eq!(lookup_approval(client, &id).await["lookup"], "pending");
    // The framework names the header, and the kits send that name.
    assert_eq!(suprnova::agents::FORM_TOOL_HEADER, FORM_TOOL);
}

#[tokio::test]
async fn agent_017_claims_refuse_a_form_tool_submission() {
    let world = claim_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("facade-claim-form-tool");
    let registration = service_auth(client, &account_email(&user)).await;
    let (token, code) = attempt_of(&registration);
    sign_in(client, &user).await;
    let csrf = csrf(client).await;
    let refused = client
        .post("/test/consent/confirm")
        .header(FORM_TOOL, "1")
        .form(&[
            ("_token", csrf.as_str()),
            ("claim_attempt_token", token.as_str()),
            ("user_code", code.as_str()),
        ])
        .send()
        .await;
    assert_eq!(refused.status(), 403, "{}", refused.body_text());
    assert_eq!(lookup_claim(client, &token).await["lookup"], "pending");
}

// ------------------------------------------------------------- claims ----

#[tokio::test]
async fn agent_017_default_claim_refuses_a_form_tool_submission() {
    default_claim_refuses_agent_header("Suprnova-Form-Tool").await;
}

#[tokio::test]
async fn agent_017_default_claim_refuses_a_browser_tool_call() {
    default_claim_refuses_agent_header("Suprnova-Tool-Call").await;
}

/// The built-in form must refuse tools before they can consume a valid claim.
async fn default_claim_refuses_agent_header(header: &str) {
    let world = claim_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("default-claim-agent-header");
    let registration = service_auth(client, &account_email(&user)).await;
    let (token, code) = attempt_of(&registration);
    sign_in(client, &user).await;
    let page = client
        .get(crate::oauth::path_of(
            registration["claim"]["verification_uri"].as_str().unwrap(),
        ))
        .send()
        .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let csrf = crate::oauth::form_token(&page.body_text());
    let form = [
        ("_token", csrf.as_str()),
        ("claim_attempt_token", token.as_str()),
        ("user_code", code.as_str()),
    ];
    let refused = client
        .post("/agent/claim")
        .header(header, "1")
        .form(&form)
        .send()
        .await;
    assert_eq!(refused.status(), 403, "{header}: {}", refused.body_text());
    assert!(
        refused.body_text().contains("person"),
        "{}",
        refused.body_text()
    );
    assert_consent_headers(&refused, "the default claim's refusal");
    assert_eq!(lookup_claim(client, &token).await["lookup"], "pending");
    let claimed = client.post("/agent/claim").form(&form).send().await;
    assert_eq!(claimed.status(), 200, "{}", claimed.body_text());
    assert_eq!(
        lookup_claim(client, &token).await,
        json!({ "lookup": "refused", "refusal": "NotFound" })
    );
    let accounts = client.get("/test/facade/agents").send().await.json();
    assert!(
        accounts.as_array().is_some_and(|accounts| accounts
            .iter()
            .any(|account| account["id"] == registration["registration_id"])),
        "the person's submission claimed the agent: {accounts}"
    );
}

#[tokio::test]
async fn agent_017_claims_read_and_confirm() {
    let world = claim_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("facade-claim");
    let registration = service_auth(client, &account_email(&user)).await;
    let (token, code) = attempt_of(&registration);
    sign_in(client, &user).await;

    let pending = lookup_claim(client, &token).await;
    assert_eq!(pending["lookup"], "pending", "{pending}");
    let claim = &pending["claim"];
    assert_eq!(claim["registration_type"], "service_auth");
    assert_eq!(claim["provider"], Value::Null);
    assert_eq!(claim["signed_in_as"], account_email(&user));
    assert_eq!(claim["email"], account_email(&user));
    let scopes = claim["scopes"].as_array().expect("scopes");
    assert!(!scopes.is_empty(), "{claim}");
    for scope in scopes {
        let described = match scope["name"].as_str() {
            Some("posts:read") => "Read your posts",
            Some("posts:write") => "Create and edit your posts",
            other => panic!("a scope agent auth did not declare: {other:?}"),
        };
        assert_eq!(scope["description"], described);
    }
    let expires = claim["expires_in_seconds"].as_u64().expect("seconds");
    assert!(expires > 0 && expires <= 600, "{expires}");
    assert_eq!(claim["attempts_remaining"], 5);
    assert_eq!(claim["first_time_account"], true);
    assert_eq!(claim["first_time_provider"], false);

    let done = confirm_claim(client, &token, &code).await;
    assert_eq!(done["confirmation"], "claimed", "{done}");
    assert_eq!(done["registration_id"], registration["registration_id"]);
    let accounts = client.get("/test/facade/agents").send().await.json();
    assert!(
        accounts.as_array().is_some_and(|accounts| accounts
            .iter()
            .any(|account| account["id"] == registration["registration_id"])),
        "the user's agents list it: {accounts}"
    );
    // A confirmed attempt is not there to read or confirm again.
    assert_eq!(
        lookup_claim(client, &token).await,
        json!({ "lookup": "refused", "refusal": "NotFound" })
    );
    assert_eq!(
        confirm_claim(client, &token, &code).await,
        json!({ "confirmation": "refused", "refusal": "NotFound" })
    );
}

#[tokio::test]
async fn agent_017_claims_count_wrong_codes() {
    let world = claim_world(|agent| agent.max_user_code_failures(3)).await;
    let client = &world.client;
    let user = new_account("facade-wrong");
    let registration = service_auth(client, &account_email(&user)).await;
    let (token, code) = attempt_of(&registration);
    sign_in(client, &user).await;
    for remaining in [2, 1] {
        assert_eq!(
            confirm_claim(client, &token, wrong(&code)).await,
            json!({ "confirmation": "wrong_code", "attempts_remaining": remaining })
        );
    }
    assert_eq!(
        lookup_claim(client, &token).await["claim"]["attempts_remaining"],
        1
    );
    assert_eq!(
        confirm_claim(client, &token, wrong(&code)).await,
        json!({ "confirmation": "wrong_code", "attempts_remaining": 0 })
    );
    // The attempt is spent: the right code no longer works.
    assert_eq!(
        confirm_claim(client, &token, &code).await,
        json!({ "confirmation": "refused", "refusal": "Ended" })
    );
    assert_eq!(
        lookup_claim(client, &token).await,
        json!({ "lookup": "refused", "refusal": "Ended" })
    );
}

#[tokio::test]
async fn agent_017_claims_refuse_another_account_and_an_unverified_address() {
    let world = claim_world(|agent| agent).await;
    let client = &world.client;
    let owner = new_account("facade-claim-owner");
    let other = new_account("facade-claim-other");
    let registration = service_auth(client, &account_email(&owner)).await;
    let (token, code) = attempt_of(&registration);

    // A guest is asked to sign in, before anything about the token is read.
    for token in [token.as_str(), "nope"] {
        assert_eq!(
            lookup_claim(client, token).await,
            json!({ "lookup": "refused", "refusal": "SignIn" })
        );
        assert_eq!(
            confirm_claim(client, token, &code).await,
            json!({ "confirmation": "refused", "refusal": "SignIn" })
        );
    }

    // Another account learns only that the claim is not theirs.
    sign_in(client, &other).await;
    let seen = lookup_claim(client, &token).await;
    assert_eq!(
        seen,
        json!({ "lookup": "refused", "refusal": "OtherAccount" })
    );
    assert!(!seen.to_string().contains(&account_email(&owner)));
    assert_eq!(
        confirm_claim(client, &token, &code).await,
        json!({ "confirmation": "refused", "refusal": "OtherAccount" })
    );

    // A token that names no attempt.
    for token in ["nope".to_owned(), "x".repeat(600)] {
        assert_eq!(
            lookup_claim(client, &token).await,
            json!({ "lookup": "refused", "refusal": "NotFound" })
        );
    }

    // An address the application has not verified is refused the same.
    let unverified = new_account("facade-claim-unverified");
    mark_unverified(&unverified);
    let theirs = service_auth(client, &account_email(&unverified)).await;
    let (their_token, their_code) = attempt_of(&theirs);
    sign_in(client, &unverified).await;
    assert_eq!(
        lookup_claim(client, &their_token).await,
        json!({ "lookup": "refused", "refusal": "OtherAccount" })
    );
    assert_eq!(
        confirm_claim(client, &their_token, &their_code).await,
        json!({ "confirmation": "refused", "refusal": "OtherAccount" })
    );

    // None of it spent the owner's attempt.
    sign_out(client).await;
    sign_in(client, &owner).await;
    assert_eq!(
        confirm_claim(client, &token, &code).await["confirmation"],
        "claimed"
    );
}

#[tokio::test]
async fn agent_017_claims_end_after_the_code_expires() {
    let world = claim_world(|agent| agent.user_code_lifetime(Duration::from_secs(1))).await;
    let client = &world.client;
    let user = new_account("facade-expiry");
    let registration = service_auth(client, &account_email(&user)).await;
    let (token, code) = attempt_of(&registration);
    sign_in(client, &user).await;
    assert_eq!(lookup_claim(client, &token).await["lookup"], "pending");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        lookup_claim(client, &token).await,
        json!({ "lookup": "refused", "refusal": "Ended" })
    );
    assert_eq!(
        confirm_claim(client, &token, &code).await,
        json!({ "confirmation": "refused", "refusal": "Ended" })
    );
}

#[tokio::test]
async fn agent_008_pending_claim_names_the_trust_list_provider_only() {
    let world = claim_world(with_id_jags).await;
    let client = &world.client;
    let user = new_account("facade-provider");
    // The provider names the user and the agent; the page shows neither.
    let claims = id_jag_claims("facade-subject", &account_email(&user));
    assert_eq!(claims["name"], "Provider-Chosen Name");
    let response = present(client, &world.id_jag(&claims)).await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    let body = response.json();
    let (token, _) = attempt_of(&body);
    sign_in(client, &user).await;
    let pending = lookup_claim(client, &token).await;
    let claim = &pending["claim"];
    assert_eq!(
        claim["registration_type"], "identity_assertion",
        "{pending}"
    );
    assert_eq!(claim["provider"], PROVIDER_NAME);
    assert_eq!(claim["first_time_provider"], true);
    let text = pending.to_string();
    for supplied in ["Provider-Chosen Name", "test-platform", "chat-1"] {
        assert!(!text.contains(supplied), "{supplied} in {text}");
    }
}

// ------------------------------------------------------------ headers ----

/// A page that renders the request's nonce, as a kit's document does.
async fn nonce_page(request: Request) -> suprnova::Response {
    Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
}

/// Consent routes under `ConsentPageHeaders` beside an ordinary page: an
/// HTML page, an Inertia-style JSON answer, a page that sets its own
/// framing headers, and one whose route also overrides the outer policy.
fn header_routes() -> Router {
    Router::new()
        .get("/consent/page", nonce_page)
        .middleware(ConsentPageHeaders)
        .get("/consent/json", |_request: Request| async move {
            Ok(HttpResponse::json(json!({ "component": "Agents/Approve" })))
        })
        .middleware(ConsentPageHeaders)
        .get("/consent/own", |_request: Request| async move {
            Ok(HttpResponse::html("own")
                .header(
                    "Content-Security-Policy",
                    "default-src 'none'; frame-ancestors *",
                )
                .header("X-Frame-Options", "SAMEORIGIN")
                .header("Cache-Control", "public, max-age=600")
                .header("Referrer-Policy", "unsafe-url"))
        })
        .middleware(ConsentPageHeaders)
        .get("/consent/overridden", nonce_page)
        .middleware(ConsentPageHeaders)
        .middleware(
            SecurityHeadersOverride::new()
                .header("x-frame-options", Some("SAMEORIGIN"))
                .and_then(|policy| policy.header("referrer-policy", Some("origin")))
                .expect("a valid override")
                .content_security_policy(Some(
                    ContentSecurityPolicy::new(
                        "default-src 'self'; script-src 'nonce-{nonce}'; frame-ancestors 'self'",
                    )
                    .expect("a valid policy"),
                )),
        )
        .get("/plain", nonce_page)
        .into()
}

/// The enforced framing directives of a policy.
fn frame_ancestors(policy: &str) -> Vec<String> {
    policy
        .split(';')
        .map(str::trim)
        .filter(|directive| {
            directive
                .split_whitespace()
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
        })
        .map(str::to_owned)
        .collect()
}

/// The headers every consent answer carries, whatever else it does.
fn assert_consent_headers(response: &TestResponse, what: &str) {
    assert_eq!(response.header("x-frame-options"), Some("DENY"), "{what}");
    assert_eq!(response.header("cache-control"), Some("no-store"), "{what}");
    assert_eq!(
        response.header("referrer-policy"),
        Some("no-referrer"),
        "{what}"
    );
    let policy = response
        .header("content-security-policy")
        .unwrap_or_else(|| panic!("{what}: an enforced policy"));
    assert_eq!(
        frame_ancestors(policy),
        ["frame-ancestors 'none'"],
        "{what}: {policy}"
    );
}

#[tokio::test]
async fn agent_017_consent_headers_refuse_framing_under_strict_balanced_relaxed_and_off() {
    for (preset, headers) in [
        ("strict", Some(SecurityHeaders::strict())),
        ("balanced", Some(SecurityHeaders::balanced())),
        ("relaxed", Some(SecurityHeaders::relaxed())),
        ("off", Some(SecurityHeaders::off())),
        ("none installed", None),
    ] {
        let registry = match headers {
            Some(headers) => MiddlewareRegistry::new().append(headers),
            None => MiddlewareRegistry::new(),
        };
        let client = TestClient::new(header_routes(), registry);
        let page = client.get("/consent/page").send().await;
        page.assert_ok();
        assert_consent_headers(&page, preset);
        let policy = page.header("content-security-policy").unwrap_or_default();
        let nonce = page.body_text();
        if matches!(preset, "strict" | "balanced") {
            // The nonce policy stays whole: only its framing changes.
            assert_eq!(nonce.len(), 22, "{preset}: a nonce");
            assert!(
                policy.contains(&format!("'nonce-{nonce}'")),
                "{preset}: {policy}"
            );
            assert!(policy.contains("form-action 'self'"), "{preset}: {policy}");
            assert!(policy.contains("object-src 'none'"), "{preset}: {policy}");
        } else {
            assert_eq!(policy, "frame-ancestors 'none'", "{preset}");
        }
        // An Inertia visit's JSON answer is guarded too.
        assert_consent_headers(&client.get("/consent/json").send().await, preset);
        // The page's own framing headers give way; the rest of its policy
        // stays.
        let own = client.get("/consent/own").send().await;
        assert_consent_headers(&own, preset);
        assert!(
            own.header("content-security-policy")
                .is_some_and(|policy| policy.contains("default-src 'none'")),
            "{preset}: {:?}",
            own.header("content-security-policy")
        );
        // A route override of the outer policy does not undo it.
        let overridden = client.get("/consent/overridden").send().await;
        assert_consent_headers(&overridden, preset);
        if headers_installed(preset) {
            let policy = overridden
                .header("content-security-policy")
                .unwrap_or_default();
            assert!(
                policy.contains(&format!("'nonce-{}'", overridden.body_text())),
                "{preset}: the override's nonce policy stays: {policy}"
            );
        }
    }
    // The control: without the middleware, balanced allows same-origin
    // framing, which is what a consent page must not.
    let client = TestClient::new(
        header_routes(),
        MiddlewareRegistry::new().append(SecurityHeaders::balanced()),
    );
    let plain = client.get("/plain").send().await;
    assert_eq!(
        frame_ancestors(plain.header("content-security-policy").unwrap_or_default()),
        ["frame-ancestors 'self'"]
    );
    assert_eq!(plain.header("x-frame-options"), Some("SAMEORIGIN"));
}

/// Whether the preset installs the middleware a route override changes.
fn headers_installed(preset: &str) -> bool {
    preset != "none installed"
}

/// Consent routes that fail in each way a page can: an error page, a
/// store error, a notice, a form the CSRF middleware refuses, and a panic.
fn error_routes() -> Router {
    Router::new()
        .get("/consent/failed", |_request: Request| async move {
            Err(HttpResponse::html("<p>failed</p>").status(500))
        })
        .middleware(ConsentPageHeaders)
        .get("/consent/unavailable", |_request: Request| async move {
            Err(HttpResponse::from(FrameworkError::domain(
                "the authorization server is unavailable",
                503,
            )))
        })
        .middleware(ConsentPageHeaders)
        .get("/consent/ended", |_request: Request| async move {
            Ok(HttpResponse::html("<p>ended</p>").status(404))
        })
        .middleware(ConsentPageHeaders)
        .post("/consent/form", |_request: Request| async move {
            Ok(HttpResponse::text("accepted"))
        })
        .middleware(ConsentPageHeaders)
        .get("/consent/panics", |_request: Request| async move {
            if std::hint::black_box(true) {
                panic!("a handler bug");
            }
            Ok(HttpResponse::text("unreachable"))
        })
        .middleware(ConsentPageHeaders)
        .into()
}

#[tokio::test]
async fn agent_017_consent_headers_guard_error_answers() {
    let _container = container();
    for preset in ["balanced", "none installed"] {
        let mut registry = registry();
        if preset == "balanced" {
            registry = registry.append(SecurityHeaders::balanced());
        }
        let client = TestClient::new(error_routes(), registry);
        let failed = client.get("/consent/failed").send().await;
        assert_eq!(failed.status(), 500);
        assert_consent_headers(&failed, &format!("{preset}: a failed page"));
        let unavailable = client.get("/consent/unavailable").send().await;
        assert_eq!(unavailable.status(), 503);
        assert_consent_headers(&unavailable, &format!("{preset}: a store error"));
        let ended = client.get("/consent/ended").send().await;
        assert_eq!(ended.status(), 404);
        assert_consent_headers(&ended, &format!("{preset}: a notice"));
        // The CSRF middleware's refusal of a form without its token.
        let refused = client
            .post("/consent/form")
            .form(&[("x", "1")])
            .send()
            .await;
        assert_eq!(refused.status(), 419, "{}", refused.body_text());
        assert_consent_headers(&refused, &format!("{preset}: a CSRF refusal"));
        if preset == "balanced" {
            // A panic is answered past the route's middleware; the policy
            // the middleware changed still frames the answer.
            let panicked = client.get("/consent/panics").send().await;
            assert_eq!(panicked.status(), 500);
            assert_eq!(panicked.header("x-frame-options"), Some("DENY"));
            assert_eq!(panicked.header("cache-control"), Some("no-store"));
            assert_eq!(panicked.header("referrer-policy"), Some("no-referrer"));
        }
    }
}

/// A middleware registered before the Inertia layer that refuses the
/// request, as a throttle or an auth guard does: its answer reaches the
/// server's error decision, which renders the error page after the chain.
struct Throttled;

#[async_trait::async_trait]
impl Middleware for Throttled {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(HttpResponse::from(FrameworkError::domain(
            "Too many requests.",
            429,
        )))
    }
}

/// Render the error page the server decides for every error answer.
fn render_error_pages() {
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        Some(error.render("Error", json!({ "status": status })))
    });
}

/// The `content-type` of an answer, without its parameters.
fn media_type(response: &TestResponse) -> String {
    response
        .header("content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[tokio::test]
async fn agent_017_consent_error_page_rendered_after_the_chain_keeps_the_policy() {
    let _container = container();
    render_error_pages();
    let client = TestClient::new(
        Router::new()
            .get("/consent/page", nonce_page)
            .middleware(ConsentPageHeaders),
        MiddlewareRegistry::new()
            .append(SecurityHeaders::balanced())
            .append(Throttled),
    );
    let page = client
        .get("/consent/page?authorization=a1b2")
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(page.status(), 429, "{}", page.body_text());
    assert_eq!(
        media_type(&page),
        "text/html",
        "the error page: {}",
        page.body_text()
    );
    assert_consent_headers(&page, "the error page");
    let policy = page.header("content-security-policy").unwrap_or_default();
    assert!(policy.contains("'nonce-"), "the nonce policy: {policy}");
    assert!(policy.contains("form-action 'self'"), "{policy}");
    assert!(policy.contains("object-src 'none'"), "{policy}");
}

#[tokio::test]
async fn agent_017_error_page_keeps_a_no_store_the_answer_carried() {
    // No consent middleware here: the page renderer itself keeps what the
    // answer said about storing it.
    let _container = container();
    render_error_pages();
    let client = TestClient::new(
        Router::new().get("/private", |_request: Request| async move {
            Err(
                HttpResponse::from(FrameworkError::domain("Not yours.", 403))
                    .header("Cache-Control", "no-store"),
            )
        }),
        MiddlewareRegistry::new(),
    );
    let page = client
        .get("/private")
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(page.status(), 403, "{}", page.body_text());
    assert_eq!(
        media_type(&page),
        "text/html",
        "the error page: {}",
        page.body_text()
    );
    let cache = page.header("cache-control").unwrap_or_default();
    assert!(cache.contains("no-store"), "{cache}");
}

#[tokio::test]
async fn agent_017_consent_headers_enforce_framing_under_report_only() {
    let client = TestClient::new(
        header_routes(),
        MiddlewareRegistry::new().append(SecurityHeaders::balanced().report_only(true)),
    );
    let page = client.get("/consent/page").send().await;
    // A report-only policy does not enforce `frame-ancestors`, so an
    // enforced one carries it alone.
    assert_consent_headers(&page, "report-only");
    assert_eq!(
        page.header("content-security-policy"),
        Some("frame-ancestors 'none'")
    );
    let reported = page
        .header("content-security-policy-report-only")
        .expect("the report-only policy stays");
    assert!(
        reported.contains(&format!("'nonce-{}'", page.body_text())),
        "{reported}"
    );
    assert_eq!(frame_ancestors(reported), ["frame-ancestors 'none'"]);
    // An ordinary page keeps report-only alone.
    let plain = client.get("/plain").send().await;
    assert_eq!(plain.header("content-security-policy"), None);
}

#[tokio::test]
async fn agent_017_default_approval_page_keeps_its_own_policy_under_the_global_one() {
    // The default page posts a native form whose answer redirects to the
    // client's origin. Under the global `form-action 'self'` a browser
    // blocks that redirect, so the page answers with a policy of its own,
    // which the global one does not replace.
    let _container = container();
    let db = database().await;
    let client = TestClient::new(
        router(AgentsConfig::new().oauth_server(OAuthServer::new().connection(db))),
        registry().append(SecurityHeaders::balanced()),
    );
    let user = new_account("default-page-policy");
    let client_id = register(&client, LOOPBACK_REDIRECT).await;
    let guest = client.get("/oauth/approve?authorization=x").send().await;
    assert_eq!(guest.status(), 302);
    sign_in(&client, &user).await;
    let (_, challenge) = pkce();
    let authorized = client
        .get(authorize_path(
            &client_id,
            LOOPBACK_REDIRECT,
            &challenge,
            "posts:read",
        ))
        .send()
        .await;
    let page = client
        .get(crate::oauth::path_of(&location(&authorized)))
        .send()
        .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let ended = client.get("/oauth/approve?authorization=x").send().await;
    assert_eq!(ended.status(), 404);
    for (response, what) in [(&guest, "guest"), (&page, "page"), (&ended, "ended")] {
        assert_eq!(
            response.header("content-security-policy"),
            Some("frame-ancestors 'none'"),
            "{what}: no form-action reaches the page"
        );
        assert_eq!(response.header("x-frame-options"), Some("DENY"), "{what}");
        assert_eq!(response.header("cache-control"), Some("no-store"), "{what}");
    }
}

// ----------------------------------------------------------- test key ----

/// The key id of an identity assertion.
fn assertion_kid(assertion: &str) -> String {
    let header = assertion.split('.').next().expect("a JWT header");
    let header: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(header)
            .expect("base64url"),
    )
    .expect("JSON");
    header["kid"].as_str().expect("a kid").to_owned()
}

/// A router with agent auth configured with no key of its own.
fn keyless(db: &sea_orm::DatabaseConnection) -> Result<Router, FrameworkError> {
    Router::new().try_agents(
        AgentsConfig::new()
            .oauth_server(OAuthServer::new().connection(db.clone()))
            .agent_auth(AgentAuth::new()),
    )
}

/// A client of a node with agent auth configured with no key of its own.
fn keyless_node(db: &sea_orm::DatabaseConnection) -> TestClient {
    TestClient::new(
        router(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db.clone()))
                .agent_auth(AgentAuth::new()),
        ),
        registry(),
    )
}

#[tokio::test]
async fn agent_017_test_agent_auth_key_builds_the_router_without_the_environment() {
    // The key is installed for the whole process, which no other test may
    // see.
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_017_test_agent_auth_key_builds_the_router_without_the_environment",
    )
    .await
    {
        return;
    }
    let _container = container();
    let db = agent_database().await;
    let names = [
        "AGENT_AUTH_KEY_ID",
        "AGENT_AUTH_PRIVATE_KEY",
        "AGENT_AUTH_PREVIOUS_KEY_ID",
        "AGENT_AUTH_PREVIOUS_PRIVATE_KEY",
        "AGENT_AUTH_PREVIOUS_ROTATED_AT",
    ];
    // SAFETY: this test runs alone in its own process.
    unsafe {
        for name in names {
            std::env::remove_var(name);
        }
    }
    assert!(keyless(&db).is_err(), "without the seam, no key fails");

    suprnova::testing::install_test_agent_auth_key();
    suprnova::testing::install_test_agent_auth_key();
    let first = keyless_node(&db);
    let second = keyless_node(&db);
    let registration = anonymous(&first).await;
    let assertion = registration["identity_assertion"]
        .as_str()
        .expect("an identity assertion");
    let kid = assertion_kid(assertion);
    assert!(kid.starts_with("agent-test"), "{kid}");
    // One key for the process: another node verifies what the first signed.
    let tokens = second
        .post("/oauth/token")
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());

    // A key the environment names wins over the seam.
    let pem = crate::agent_auth::service_pem();
    // SAFETY: as above.
    unsafe {
        std::env::set_var("AGENT_AUTH_KEY_ID", "env-1");
        std::env::set_var("AGENT_AUTH_PRIVATE_KEY", &pem);
    }
    let named = keyless_node(&db);
    let registration = anonymous(&named).await;
    assert_eq!(
        assertion_kid(
            registration["identity_assertion"]
                .as_str()
                .expect("an identity assertion")
        ),
        "env-1"
    );
    // Half a key in the environment is still a configuration error.
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("AGENT_AUTH_PRIVATE_KEY");
    }
    assert!(keyless(&db).is_err(), "a key id without its key fails");
}
