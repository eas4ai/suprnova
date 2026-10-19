//! AGENT-017 in a generated Live application: the approval, claim and
//! connected-agents pages, the sign-in and registration that return to
//! them, and the migrator that creates what they need.
//!
//! Written into a fresh `suprnova new --frontend live` scaffold as
//! `tests/agent_pages.rs` by
//! `agent_017_a_generated_live_application_serves_the_agent_pages` in
//! `suprnova-cli/tests/scaffold_snapshot.rs`, which replaces `__PACKAGE__`
//! with the generated crate's name. It boots what `tests/documents.rs`
//! boots, with the routes served through `agents::serve` as `cmd/main.rs`
//! serves them, and drives the pages with `TestClient`: each page's native
//! forms posted as a browser posts them, and the kit's Live sign-in and
//! registration forms through the Live action endpoint, as the runtime
//! calls it.
//!
//! One test runs the cases in order on one application and one cookie jar,
//! signing out between the users who sign in, as a person would; the second
//! user who looks at another's agents is the test client's `acting_as`. Each
//! case prints `case <name> ... ok` when it passes, and the harness checks
//! every line.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sha2::{Digest, Sha256};
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::mail::MailFake;
use suprnova::serde_json::{self, Value, json};
use suprnova::testing::{TestClient, TestContainer, TestDatabase, TestRequest, TestResponse};
use suprnova::{
    App, AppConfig, Auth, AuthConfig, AuthManager, CacheStore, Config, EloquentUserProvider,
    InMemoryCache, MiddlewareRegistry, Queue, SyncQueueDriver,
};
use url::Url;

use __PACKAGE__::migrations::Migrator;
use __PACKAGE__::models::user::User;
use __PACKAGE__::{agents, bootstrap, live, routes};

/// The application's public URL, the OAuth server's issuer: HTTPS, or HTTP
/// on a loopback host.
const APP_URL: &str = "http://localhost";

/// The MCP endpoint a client's token is for.
const RESOURCE: &str = "http://localhost/mcp";

/// The redirect URI the test's MCP client registers: loopback HTTP, on an
/// origin other than the application's.
const CLIENT_REDIRECT: &str = "http://127.0.0.1:9/callback";

/// The password of every account the test makes.
const PASSWORD: &str = "agents-kit-password-42";

/// The grant an agent polls the token endpoint with for its claimed tokens.
const CLAIM_GRANT: &str = "urn:workos:agent-auth:grant-type:claim";

/// What the claim page says when the claim is for another account, or for
/// an address the application has not verified.
const OTHER_ACCOUNT: &str = "This claim is for another account, or your address is not verified \
                             yet. Sign in with the address the agent named.";

/// The application, booted once, and what the cases share.
struct World {
    client: TestClient,
    mail: MailFake,
    db: TestDatabase,
    /// An account made directly, for the sign-in cases.
    grace: User,
    /// A second account, which must see nothing of another user's agents.
    bea: User,
}

/// Boot the generated application as `cmd/main.rs` does, minus the
/// listener: the database with every migration, the auth provider, the Live
/// registry, the HTTP stack, and the routes through `agents::serve`. Mail
/// is captured.
async fn boot() -> World {
    // The keys `.env` holds for the server; a test process loads no `.env`.
    suprnova::testing::install_test_encryption_key();
    suprnova::testing::install_test_agent_auth_key();
    // The OAuth server's issuer, which a client checks every answer against.
    Config::register(AppConfig::builder().url(APP_URL).build());
    let db = TestDatabase::fresh::<Migrator>()
        .await
        .expect("a fresh database with every migration");
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));
    let mail = suprnova::Mail::fake();
    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register the users provider");
    App::bind_if_absent::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    suprnova::rate_limit::bootstrap_default().await;
    App::singleton(live::registry().expect("the Live component registry"));
    live::authorize();
    bootstrap::register_http_stack();
    let served = agents::serve(routes::register()).expect("the routes with the agents served");
    let router = live::routes_with_render_cache(served)
        .await
        .expect("the routes with the render cache");
    let runtime = prepare_live_router_for_test(&router).expect("the Live runtime");
    App::singleton(runtime);
    let client = TestClient::new(router, MiddlewareRegistry::from_global())
        .with_peer_address("127.0.0.1".parse().expect("an address"));
    let grace = User::create("Grace Hopper", "grace@example.test", PASSWORD)
        .await
        .expect("Grace's account");
    let bea = User::create("Bea Example", "bea@example.test", PASSWORD)
        .await
        .expect("Bea's account");
    World {
        client,
        mail,
        db,
        grace,
        bea,
    }
}

// -------------------------------------------------------------- helpers ----

/// A `GET` of a page, as a browser asks for one.
fn page(client: &TestClient, path: &str) -> TestRequest {
    client.get(path).header("Accept", "text/html")
}

/// A native form post of `fields`, as a browser sends one.
async fn post_form(client: &TestClient, path: &str, fields: &[(&str, &str)]) -> TestResponse {
    client
        .post(path)
        .header("Accept", "text/html")
        .form(&fields)
        .send()
        .await
}

/// The path and query of an absolute or rooted address.
fn path_of(address: &str) -> String {
    let url = Url::parse(APP_URL)
        .and_then(|base| base.join(address))
        .unwrap_or_else(|error| panic!("{address}: {error}"));
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// The query parameter `name` of an absolute or rooted address.
fn query_param(address: &str, name: &str) -> Option<String> {
    Url::parse(APP_URL)
        .and_then(|base| base.join(address))
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Where a redirect sends the browser, as a path and query.
fn location(response: &TestResponse) -> String {
    let location = response.header("location").unwrap_or_else(|| {
        panic!(
            "a redirect, got {} {}",
            response.status(),
            response.body_text()
        )
    });
    path_of(location)
}

/// Undo the escaping a template applies to an attribute or a text: the
/// named entities and any numeric one, decimal or hexadecimal.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .map(|hex| u32::from_str_radix(hex, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The value of the first field named `name` on `html`.
fn hidden(html: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = html
        .find(&marker)
        .unwrap_or_else(|| panic!("no `{name}` field in:\n{html}"))
        + marker.len();
    let end = html[start..].find('"').expect("the value ends") + start;
    unescape(&html[start..end])
}

/// The value of the attribute `name` in the tag `tag`.
fn attribute<'a>(tag: &'a str, name: &str) -> &'a str {
    let marker = format!("{name}=\"");
    let start = tag
        .find(&marker)
        .unwrap_or_else(|| panic!("no {name} in {tag}"))
        + marker.len();
    let end = tag[start..].find('"').expect("the attribute ends") + start;
    &tag[start..end]
}

/// A one-time value of the shape the Live protocol takes: 16 bytes,
/// base64url without padding, different on every call.
fn one_time() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    URL_SAFE_NO_PAD.encode(format!("{:016}", SEQUENCE.fetch_add(1, Ordering::Relaxed)))
}

/// The signed snapshot the page carries for its island `key`.
fn island_snapshot(html: &str, key: &str) -> Value {
    let needle = format!("data-suprnova-live-document-key=\"{key}\"");
    let at = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no island {key} in:\n{html}"));
    let start = html[..at].rfind('<').expect("the island's tag opens");
    let end = html[at..].find('>').expect("the island's tag closes") + at;
    let encoded = attribute(&html[start..end], "data-suprnova-live-snapshot");
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("the snapshot is base64url");
    serde_json::from_slice(&bytes).expect("the snapshot is JSON")
}

/// Run the action `action` of the form island `form` on `html` with the
/// fields `fields`, as the Live runtime sends it: the fields synced, then
/// the action invoked, with the browser's same-origin proof. The answer.
async fn live_action(
    client: &TestClient,
    html: &str,
    form: &str,
    fields: Value,
    action: &str,
) -> Value {
    let envelope = island_snapshot(html, form);
    let (base_revision, snapshot) = match &envelope["body"]["revision"] {
        Value::String(revision) => (
            revision.clone(),
            json!({"envelope": envelope, "kind": "instance"}),
        ),
        Value::Number(revision) => (
            revision.to_string(),
            json!({"envelope": envelope, "kind": "instance"}),
        ),
        _ => (
            "0".to_owned(),
            json!({"browser_nonce": one_time(), "envelope": envelope, "kind": "seed_promotion"}),
        ),
    };
    let mut operations: Vec<Value> = fields
        .as_object()
        .expect("the fields are an object")
        .keys()
        .map(|field| json!({"field": field, "kind": "sync_model"}))
        .collect();
    operations.push(json!({"arguments": {}, "kind": "invoke_action", "name": action}));
    let body = json!({
        "base_revision": base_revision,
        "child_parameters": null,
        "component": format!("__PACKAGE__.{form}"),
        "correlation_id": one_time(),
        "extensions": {"x_suprnova_live_document_key_v1": form},
        "idempotency_key": one_time(),
        "model_proposals": fields,
        "operations": operations,
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": snapshot,
        "snapshot_schema_version": 1,
    });
    let answer = client
        .post("/__live/action")
        .header("Sec-Fetch-Site", "same-origin")
        .body(
            "application/vnd.suprnova.live+json; charset=utf-8; version=2",
            serde_json::to_vec(&body).expect("encode the action"),
        )
        .send()
        .await;
    assert_eq!(
        answer.status(),
        200,
        "the Live action {form}.{action}: {}",
        answer.body_text()
    );
    answer.json()
}

/// Where a Live action sent the browser: protocol 2 names the page in a
/// `navigated` URL intent, which the runtime loads.
fn live_redirect(answer: &Value) -> String {
    let intent = &answer["url_intent"];
    let navigated = (intent["kind"] == "navigated")
        .then(|| intent["target"].as_str())
        .flatten();
    answer["redirect"]
        .as_str()
        .or(navigated)
        .map(path_of)
        .unwrap_or_else(|| panic!("the action sends the browser on: {answer}"))
}

/// Sign in through the kit's Live sign-in form on the page at `login`
/// (`/login`, with a `return_to` or without); where the form sends the
/// browser.
async fn sign_in(client: &TestClient, login: &str, email: &str) -> String {
    let form = page(client, login).send().await;
    assert_eq!(form.status(), 200, "GET {login}: {}", form.body_text());
    let answer = live_action(
        client,
        &form.body_text(),
        "login-form",
        json!({"email": email, "password": PASSWORD, "remember": false}),
        "login",
    )
    .await;
    live_redirect(&answer)
}

/// Register through the kit's Live registration form; the action's answer.
async fn register(client: &TestClient, name: &str, email: &str) -> Value {
    let form = page(client, "/register").send().await;
    assert_eq!(form.status(), 200, "GET /register: {}", form.body_text());
    live_action(
        client,
        &form.body_text(),
        "register-form",
        json!({
            "name": name,
            "email": email,
            "password": PASSWORD,
            "password_confirmation": PASSWORD,
        }),
        "register",
    )
    .await
}

/// Sign the session out with the sign-out form every signed-in page
/// carries, so the next request is a guest's.
async fn sign_out(client: &TestClient) {
    let signed_in = page(client, "/connected-agents").send().await;
    assert_eq!(signed_in.status(), 200, "{}", signed_in.body_text());
    let token = hidden(&signed_in.body_text(), "_token");
    let out = post_form(client, "/logout", &[("_token", token.as_str())]).await;
    assert_eq!(location(&out), "/", "{}", out.body_text());
    assert_eq!(
        page(client, "/connected-agents").send().await.status(),
        302,
        "the session is a guest's"
    );
}

/// Open the verification link the kit mailed to `email`, as the signed-in
/// user it belongs to.
async fn verify_email(world: &World, email: &str) {
    let prefix = format!("{APP_URL}/verify-email/verify?token=");
    let link = world
        .mail
        .captured()
        .iter()
        .rev()
        .filter(|message| message.has_to(email))
        .filter_map(|message| message.text.clone())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| {
                    line.split_once(&prefix)
                        .map(|(_, token)| token.trim().to_owned())
                })
                .collect::<Vec<_>>()
        })
        .next()
        .unwrap_or_else(|| {
            panic!(
                "no verification link went to {email}; {} messages were sent",
                world.mail.count()
            )
        });
    let verified = page(&world.client, &format!("/verify-email/verify?token={link}"))
        .send()
        .await;
    assert_eq!(
        location(&verified),
        "/dashboard",
        "{}",
        verified.body_text()
    );
}

/// Every consent answer refuses framing, caching and referrers, and keeps
/// the policy's nonce sources and `form-action 'self'`.
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
        .unwrap_or_else(|| panic!("{what}: no Content-Security-Policy"));
    assert!(
        policy.contains("frame-ancestors 'none'") && !policy.contains("frame-ancestors 'self'"),
        "{what}: {policy}"
    );
    assert!(policy.contains("'nonce-"), "{what}: {policy}");
    assert!(policy.contains("form-action 'self'"), "{what}: {policy}");
}

/// A PKCE verifier and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = format!("verifier-{}-{}", one_time(), one_time());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The authorization request the client sends the browser to.
fn authorize_path(client_id: &str, challenge: &str) -> String {
    let mut url = Url::parse(&format!("{APP_URL}/oauth/authorize")).expect("the endpoint");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", CLIENT_REDIRECT)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", RESOURCE)
        .append_pair("state", "state-123");
    path_of(url.as_str())
}

/// Register the MCP client dynamically (RFC 7591); its id.
async fn register_client(client: &TestClient) -> String {
    let registered = client
        .post("/oauth/register")
        .json(&json!({
            "client_name": "Example Tool",
            "redirect_uris": [CLIENT_REDIRECT],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
        }))
        .send()
        .await;
    assert_eq!(registered.status(), 201, "{}", registered.body_text());
    registered.json()["client_id"]
        .as_str()
        .expect("a client id")
        .to_owned()
}

/// From the authorization endpoint, as the signed-in user, to the approval
/// page: the authorization's id and the page.
async fn open_approval(client: &TestClient, authorize: &str) -> (String, TestResponse) {
    let authorized = page(client, authorize).send().await;
    let approve = location(&authorized);
    assert!(approve.starts_with("/oauth/approve?"), "{approve}");
    let id = query_param(&approve, "authorization").expect("the authorization's id");
    let approval = page(client, &approve).send().await;
    assert_eq!(approval.status(), 200, "{}", approval.body_text());
    (id, approval)
}

/// Post the decision `decision` on the authorization `id` with the form
/// the approval page carries.
async fn decide(
    client: &TestClient,
    approval: &TestResponse,
    id: &str,
    decision: &str,
) -> TestResponse {
    let token = hidden(&approval.body_text(), "_token");
    post_form(
        client,
        "/oauth/approve",
        &[
            ("_token", token.as_str()),
            ("authorization", id),
            ("decision", decision),
        ],
    )
    .await
}

/// Where the returning page sends the browser: its meta refresh, which its
/// link names too.
fn returning_to(returning: &TestResponse) -> Url {
    let html = returning.body_text();
    let marker = "<meta http-equiv=\"refresh\" content=\"0; url=";
    let start = html
        .find(marker)
        .unwrap_or_else(|| panic!("no meta refresh in:\n{html}"))
        + marker.len();
    let end = html[start..].find('"').expect("the refresh ends") + start;
    let target = unescape(&html[start..end]);
    assert!(
        html.contains(&format!("href=\"{}\"", &html[start..end])),
        "the page links where it refreshes to:\n{html}"
    );
    Url::parse(&target).unwrap_or_else(|error| panic!("{target}: {error}"))
}

/// The path and the sorted query pairs of a rooted address: the request
/// it makes, whatever order its parameters come in.
fn request_of(address: &str) -> (String, Vec<(String, String)>) {
    let url = Url::parse(APP_URL)
        .and_then(|base| base.join(address))
        .unwrap_or_else(|error| panic!("{address}: {error}"));
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    pairs.sort();
    (url.path().to_owned(), pairs)
}

/// The value of the query parameter `name` of `url`.
fn param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The ids in the revoke forms of the connected-agents page, apps first.
fn revoke_ids(html: &str, kind: &str) -> Vec<String> {
    let marker = format!("action=\"/connected-agents/{kind}/");
    html.split(&marker)
        .skip(1)
        .map(|rest| rest[..rest.find("/revoke\"").expect("a revoke action")].to_owned())
        .collect()
}

/// The `n` in a page's "`n` wrong entries allowed".
fn entries_allowed(html: &str) -> u64 {
    let end = html
        .find(" wrong entries allowed")
        .unwrap_or_else(|| panic!("the claim page says how many wrong entries it allows:\n{html}"));
    let start = html[..end].rfind(' ').expect("a number") + 1;
    html[start..end].parse().expect("a count")
}

// ---------------------------------------------------------------- cases ----

/// AGENT-017: the kit's own migrator creates both plugins' tables and the
/// index that keeps an address unique whatever its case, on SQLite.
async fn agent_017_live_migrator_creates_the_plugins_tables_and_the_email_index(world: &World) {
    let names: Vec<String> = world
        .db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'index')",
        ))
        .await
        .expect("read the schema")
        .into_iter()
        .map(|row| row.try_get_by_index::<String>(0).expect("a name"))
        .collect();
    for table in [
        "auth_oauth_clients",
        "auth_oauth_authorizations",
        "auth_oauth_grants",
        "auth_oauth_tokens",
        "auth_agent_registrations",
        "auth_agent_grants",
        "auth_agent_replays",
        "auth_agent_events",
    ] {
        assert!(
            names.iter().any(|name| name == table),
            "no {table}: {names:?}"
        );
    }
    let index =
        suprnova::auth::migrations::UniqueEmailIgnoringCase::on("users", "email").index_name();
    assert!(names.contains(&index), "no {index}: {names:?}");
    User::create("Case Example", "case@example.test", PASSWORD)
        .await
        .expect("the first address");
    assert!(
        User::create("Case Again", "Case@Example.test", PASSWORD)
            .await
            .is_err(),
        "an address that differs only in case is refused"
    );
}

/// AGENT-017: the generated application serves the MCP endpoint, both
/// plugins' discovery documents with the agent auth block, auth.md, and an
/// anonymous agent registration.
async fn agent_017_live_serves_the_endpoint_and_both_plugins(world: &World) {
    let client = &world.client;
    let init = client.mcp().initialize().await;
    assert_eq!(init.status(), 200, "{init:?}");
    assert!(
        init.result()
            .is_some_and(|result| result["protocolVersion"].is_string()),
        "{init:?}"
    );
    let resource = client
        .get("/.well-known/oauth-protected-resource")
        .send()
        .await;
    assert_eq!(resource.status(), 200, "{}", resource.body_text());
    assert_eq!(resource.json()["resource"], RESOURCE);
    let metadata = client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await;
    assert_eq!(metadata.status(), 200, "{}", metadata.body_text());
    let metadata = metadata.json();
    assert_eq!(metadata["issuer"], APP_URL);
    assert_eq!(
        metadata["agent_auth"]["skill"],
        format!("{APP_URL}/auth.md")
    );
    assert_eq!(
        metadata["agent_auth"]["identity_endpoint"],
        format!("{APP_URL}/agent/identity")
    );
    let skill = client.get("/auth.md").send().await;
    assert_eq!(skill.status(), 200, "{}", skill.body_text());
    assert!(!skill.body_text().is_empty());
    let anonymous = client
        .post("/agent/identity")
        .json(&json!({"type": "anonymous"}))
        .send()
        .await;
    assert_eq!(anonymous.status(), 200, "{}", anonymous.body_text());
    assert_eq!(anonymous.json()["registration_type"], "anonymous");
}

/// The five shapes of `return_to` a browser would read as another host:
/// another host by `//`, by a backslash, by a scheme, by a tab the browser
/// drops, and by a leading space.
const FOREIGN: [&str; 5] = [
    "//evil.example/x",
    "/\\evil.example",
    "https://evil.example/x",
    "/\t/evil.example",
    " //evil.example",
];

/// `/login` with `return_to` set to `target`, encoded.
fn login_returning_to(target: &str) -> String {
    let mut url = Url::parse(&format!("{APP_URL}/login")).expect("the sign-in page");
    url.query_pairs_mut().append_pair("return_to", target);
    path_of(url.as_str())
}

/// AGENT-017: a `return_to` a browser would read as another host never
/// sends anyone off the application. A guest who signs in goes home, and a
/// signed-in user who opens the link goes home.
async fn agent_017_live_login_ignores_a_foreign_return_to(world: &World) {
    let client = &world.client;
    for target in FOREIGN {
        let login = login_returning_to(target);
        assert_eq!(
            sign_in(client, &login, &world.grace.email).await,
            "/dashboard",
            "a guest from {target:?} goes home"
        );
        let signed_in = page(client, &login).send().await;
        assert_eq!(
            location(&signed_in),
            "/dashboard",
            "a signed-in user from {target:?} goes home"
        );
        sign_out(client).await;
    }
}

/// AGENT-017: a same-origin `return_to` is followed, by a guest who signs
/// in through `login.continue` and by a user who is signed in already. The
/// address is spent: the next sign-in goes home.
async fn agent_017_live_login_returns_to_a_same_origin_address(world: &World) {
    let client = &world.client;
    let target = "/agent/claim?claim_attempt_token=x";
    let login = login_returning_to(target);
    assert_eq!(
        sign_in(client, &login, &world.grace.email).await,
        "/login/continue",
        "the Live form continues to the intended address by its named route"
    );
    let continued = page(client, "/login/continue").send().await;
    assert_eq!(location(&continued), target);
    let spent = page(client, "/login/continue").send().await;
    assert_eq!(location(&spent), "/dashboard", "the address was spent");
    let signed_in = page(client, &login).send().await;
    assert_eq!(
        location(&signed_in),
        target,
        "a signed-in user follows it too"
    );
    sign_out(client).await;
    assert_eq!(
        sign_in(client, "/login", &world.grace.email).await,
        "/dashboard",
        "a second sign-in goes home"
    );
    sign_out(client).await;
}

/// AGENT-017 and ruling 7: a guest an agent's claim link sent to sign in
/// who registers instead lands back on the claim page, which asks an
/// unverified address to verify first. Once verified, the page shows the
/// claim. The address is spent afterwards.
async fn agent_017_live_registration_continues_to_the_intended_address(world: &World) {
    let client = &world.client;
    let email = "newcomer@example.test";
    let registration = client
        .post("/agent/identity")
        .json(&json!({"type": "service_auth", "login_hint": email}))
        .send()
        .await;
    assert_eq!(registration.status(), 200, "{}", registration.body_text());
    let claim_page = path_of(
        registration.json()["claim"]["verification_uri"]
            .as_str()
            .expect("a verification URI"),
    );
    let guest = page(client, &claim_page).send().await;
    assert_eq!(location(&guest), "/login", "a guest signs in first");
    let answer = register(client, "Newcomer", email).await;
    assert_eq!(live_redirect(&answer), "/login/continue", "{answer}");
    let continued = page(client, "/login/continue").send().await;
    assert_eq!(location(&continued), claim_page);
    let spent = page(client, "/login/continue").send().await;
    assert_eq!(location(&spent), "/dashboard", "the address was spent");
    let unverified = page(client, &claim_page).send().await;
    assert_eq!(unverified.status(), 403, "{}", unverified.body_text());
    assert!(
        unverified.body_text().contains(OTHER_ACCOUNT),
        "{}",
        unverified.body_text()
    );
    assert_consent_headers(&unverified, "the claim page refusing an unverified address");
    verify_email(world, email).await;
    let verified = page(client, &claim_page).send().await;
    assert_eq!(verified.status(), 200, "{}", verified.body_text());
    sign_out(client).await;
    assert_eq!(sign_in(client, "/login", email).await, "/dashboard");
    sign_out(client).await;
}

/// AGENT-008 and AGENT-017: registration stores the address in lowercase
/// and refuses one that differs from a registered address only in case.
async fn agent_008_live_registration_lowercases_and_refuses_a_mixed_case_duplicate(world: &World) {
    let client = &world.client;
    let answer = register(client, "Mixed Case", "  Mixed.Case@Example.TEST ").await;
    assert_eq!(live_redirect(&answer), "/verify-email", "{answer}");
    let stored = User::find_by_email("mixed.case@example.test")
        .await
        .expect("look the address up")
        .expect("the account");
    assert_eq!(stored.email, "mixed.case@example.test");
    sign_out(client).await;
    let again = register(client, "Mixed Again", "MIXED.case@example.test").await;
    let html = again["render"]["html"].as_str().unwrap_or_default();
    assert!(
        html.contains("This email is already registered."),
        "{again}"
    );
}

/// The kit's registration bounds: a name of at most 255 characters and an
/// address of at most 254.
async fn kit_live_registration_refuses_a_name_over_255_and_an_email_over_254(world: &World) {
    let client = &world.client;
    let long_name = register(client, &"n".repeat(256), "long.name@example.test").await;
    let html = long_name["render"]["html"].as_str().unwrap_or_default();
    assert!(
        html.contains("Name must be 2 to 255 characters"),
        "{long_name}"
    );
    // A well-formed address, its local part and labels within their own
    // limits, one character over the bound.
    let address = format!(
        "{}@{}.{}.{}.test",
        "a".repeat(64),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(57)
    );
    assert_eq!(address.len(), 255);
    let long_email = register(client, "Long Address", &address).await;
    let html = long_email["render"]["html"].as_str().unwrap_or_default();
    assert!(
        html.contains("Email must be at most 254 characters"),
        "{long_email}"
    );
}

/// AGENT-005 and AGENT-017: the approval round trip. A guest the
/// authorization endpoint sends to sign in comes back to it through
/// `login.continue`, sees the client, its host and where they return, and
/// approves. The decision answers with the returning page, whose meta
/// refresh and link carry the redirect with `code`, `state` and `iss`; the
/// code exchanges for a token the MCP endpoint takes. A denial returns
/// `access_denied`, and a spent authorization has ended. The client's
/// access token.
async fn agent_017_live_approval_round_trip(world: &World, email: &str) -> String {
    let client = &world.client;
    let client_id = register_client(client).await;
    let (verifier, challenge) = pkce();
    let authorize = authorize_path(&client_id, &challenge);
    let guest = page(client, &authorize).send().await;
    let login = location(&guest);
    assert!(login.starts_with("/login?"), "{login}");
    let return_to = query_param(&login, "return_to").expect("a return address");
    assert_eq!(
        request_of(&return_to),
        request_of(&authorize),
        "the sign-in link names the authorization request"
    );
    assert_eq!(sign_in(client, &login, email).await, "/login/continue");
    let continued = page(client, "/login/continue").send().await;
    assert_eq!(location(&continued), return_to);
    let (id, approval) = open_approval(client, &authorize).await;
    assert_consent_headers(&approval, "the approval page");
    let html = approval.body_text();
    for part in [
        "Allow <bdi>Example Tool</bdi> to act for you?",
        "<bdi>127.0.0.1",
        "<bdi><code>http://127.0.0.1:9/callback</code></bdi>",
    ] {
        assert!(
            html.contains(part),
            "the approval page lacks {part}:\n{html}"
        );
    }
    assert!(
        !html.contains("suprnova-live-config") && !html.contains("<script id"),
        "the approval page loads no Live runtime:\n{html}"
    );

    let returning = decide(client, &approval, &id, "approve").await;
    assert_eq!(returning.status(), 200, "{}", returning.body_text());
    assert_consent_headers(&returning, "the returning page");
    assert!(
        returning
            .body_text()
            .contains("Returning you to <bdi>Example Tool</bdi>"),
        "{}",
        returning.body_text()
    );
    let reached = returning_to(&returning);
    assert_eq!(&reached[..url::Position::AfterPath], CLIENT_REDIRECT);
    assert_eq!(
        param(&reached, "state").as_deref(),
        Some("state-123"),
        "{reached}"
    );
    assert_eq!(
        param(&reached, "iss").as_deref(),
        Some(APP_URL),
        "{reached}"
    );
    let code = param(&reached, "code").expect("a code");

    let tokens = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", CLIENT_REDIRECT),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    let listed = client.mcp().with_token(token.as_str()).list().await;
    assert_eq!(listed.status(), 200, "{listed:?}");

    let spent = decide(client, &approval, &id, "approve").await;
    assert_eq!(spent.status(), 404, "{}", spent.body_text());
    assert!(spent.body_text().contains("This request has ended."));

    let (_, challenge) = pkce();
    let (denied_id, denial) = open_approval(client, &authorize_path(&client_id, &challenge)).await;
    let denied = decide(client, &denial, &denied_id, "deny").await;
    assert_eq!(denied.status(), 200, "{}", denied.body_text());
    let reached = returning_to(&denied);
    assert_eq!(
        param(&reached, "error").as_deref(),
        Some("access_denied"),
        "{reached}"
    );
    assert_eq!(
        param(&reached, "state").as_deref(),
        Some("state-123"),
        "{reached}"
    );
    assert_eq!(
        param(&reached, "iss").as_deref(),
        Some(APP_URL),
        "{reached}"
    );
    token
}

/// AGENT-009 and AGENT-017: the claim round trip. An agent registers with
/// the signed-in, verified user's address; the claim page shows who it
/// acts as and how many wrong codes it takes; a wrong code shows the page
/// again with the attempts left; the right one connects the agent, whose
/// tokens then work at the MCP endpoint. The agent's access token.
async fn agent_017_live_claim_round_trip(world: &World, email: &str) -> String {
    let client = &world.client;
    let registration = client
        .post("/agent/identity")
        .json(&json!({"type": "service_auth", "login_hint": email}))
        .send()
        .await;
    assert_eq!(registration.status(), 200, "{}", registration.body_text());
    let registration = registration.json();
    let claim = &registration["claim"];
    let user_code = claim["user_code"].as_str().expect("a user code").to_owned();
    let claim_page = path_of(
        claim["verification_uri"]
            .as_str()
            .expect("a verification URI"),
    );
    assert!(
        claim_page.starts_with("/agent/claim?claim_attempt_token="),
        "{claim_page}"
    );

    let shown = page(client, &claim_page).send().await;
    assert_eq!(shown.status(), 200, "{}", shown.body_text());
    assert_consent_headers(&shown, "the claim page");
    let html = shown.body_text();
    for part in [
        "Let an agent act for you?",
        "An agent that named your email address asks to act as you.",
        format!("You are signed in as <strong><bdi>{email}</bdi></strong>.").as_str(),
        format!("The agent named <strong><bdi>{email}</bdi></strong>.").as_str(),
        "no agent has acted for your account before",
    ] {
        assert!(html.contains(part), "the claim page lacks {part}:\n{html}");
    }
    let allowed = entries_allowed(&html);
    let attempt = hidden(&html, "claim_attempt_token");
    let token = hidden(&html, "_token");

    let mut wrong = user_code.clone();
    let first = if wrong.starts_with('1') { "2" } else { "1" };
    wrong.replace_range(0..1, first);
    let refused = post_form(
        client,
        "/agent/claim",
        &[
            ("_token", token.as_str()),
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", wrong.as_str()),
        ],
    )
    .await;
    assert_eq!(refused.status(), 422, "{}", refused.body_text());
    assert_consent_headers(&refused, "the claim page after a wrong code");
    let message = format!("That code is wrong. {} attempts left.", allowed - 1);
    assert!(
        refused.body_text().contains(&message),
        "{message}:\n{}",
        refused.body_text()
    );

    let claimed = post_form(
        client,
        "/agent/claim",
        &[
            ("_token", token.as_str()),
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", user_code.as_str()),
        ],
    )
    .await;
    assert_eq!(claimed.status(), 303, "{}", claimed.body_text());
    assert_eq!(location(&claimed), "/connected-agents");
    let connected = page(client, "/connected-agents").send().await;
    assert!(
        connected.body_text().contains("The agent is connected."),
        "{}",
        connected.body_text()
    );
    assert!(
        connected
            .body_text()
            .contains("Registered with your email address")
    );

    let tokens = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", CLAIM_GRANT),
            (
                "claim_token",
                registration["claim_token"].as_str().expect("a claim token"),
            ),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let agent_token = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    let listed = client.mcp().with_token(agent_token.as_str()).list().await;
    assert_eq!(listed.status(), 200, "{listed:?}");
    agent_token
}

/// AGENT-010 and AGENT-017: the connected-agents page lists the signed-in
/// user's client grants and agent registrations, and only theirs. Another
/// user sees none of them and cannot revoke them; the owner's revoke ends
/// each token at once.
async fn agent_017_live_connected_agents_lists_and_revokes_only_the_users_own(
    world: &World,
    client_token: &str,
    agent_token: &str,
) {
    let owner = &world.client;
    let listed = page(owner, "/connected-agents").send().await;
    assert_eq!(listed.status(), 200, "{}", listed.body_text());
    assert_consent_headers(&listed, "the connected-agents page");
    let html = listed.body_text();
    assert!(html.contains("<bdi>Example Tool</bdi>"), "{html}");
    let grants = revoke_ids(&html, "clients");
    let registrations = revoke_ids(&html, "agents");
    assert_eq!(grants.len(), 1, "{html}");
    assert_eq!(registrations.len(), 1, "{html}");

    let stranger = owner.clone().acting_as(&world.bea);
    let theirs = page(&stranger, "/connected-agents").send().await;
    assert_eq!(theirs.status(), 200, "{}", theirs.body_text());
    let html = theirs.body_text();
    assert!(html.contains("No apps can act for you."), "{html}");
    assert!(html.contains("No agents can act for you."), "{html}");
    assert!(!html.contains("Example Tool"), "{html}");
    let token = hidden(&html, "_token");
    for path in [
        format!("/connected-agents/clients/{}/revoke", grants[0]),
        format!("/connected-agents/agents/{}/revoke", registrations[0]),
    ] {
        let refused = post_form(&stranger, &path, &[("_token", token.as_str())]).await;
        assert_eq!(refused.status(), 404, "{path}: {}", refused.body_text());
        assert_consent_headers(&refused, "a revoke of another user's");
    }
    assert_eq!(
        owner.mcp().with_token(client_token).list().await.status(),
        200,
        "another user's revoke changed nothing"
    );

    let token = hidden(&listed.body_text(), "_token");
    for (path, bearer) in [
        (
            format!("/connected-agents/clients/{}/revoke", grants[0]),
            client_token,
        ),
        (
            format!("/connected-agents/agents/{}/revoke", registrations[0]),
            agent_token,
        ),
    ] {
        let revoked = post_form(owner, &path, &[("_token", token.as_str())]).await;
        assert_eq!(revoked.status(), 303, "{path}: {}", revoked.body_text());
        assert_eq!(location(&revoked), "/connected-agents");
        assert_eq!(
            owner.mcp().with_token(bearer).list().await.status(),
            401,
            "{path} ends the token"
        );
    }
    let emptied = page(owner, "/connected-agents").send().await.body_text();
    assert!(emptied.contains("No apps can act for you."), "{emptied}");
    assert!(emptied.contains("No agents can act for you."), "{emptied}");
}

/// AGENT-017: every consent form needs the session's CSRF token, and the
/// refusal still refuses framing, caching and referrers.
async fn agent_017_live_consent_forms_need_the_csrf_token(world: &World) {
    let client = &world.client;
    for path in [
        "/oauth/approve",
        "/agent/claim",
        "/connected-agents/clients/any/revoke",
        "/connected-agents/agents/any/revoke",
    ] {
        let refused = post_form(client, path, &[("authorization", "any")]).await;
        assert_eq!(refused.status(), 419, "{path}: {}", refused.body_text());
        assert_eq!(refused.header("x-frame-options"), Some("DENY"), "{path}");
        assert_eq!(refused.header("cache-control"), Some("no-store"), "{path}");
    }
}

/// AGENT-005 and AGENT-017: a consent page's refusals refuse framing,
/// caching and referrers too, and say what went wrong in the error
/// layout.
async fn agent_017_live_consent_pages_refuse_framing_caching_and_referrers(world: &World) {
    let client = &world.client;
    for (path, status, message) in [
        (
            "/oauth/approve",
            400,
            "This link does not name an authorization request.",
        ),
        (
            "/oauth/approve?authorization=nothing-pending",
            404,
            "This request has ended. Start again from the app that asked.",
        ),
        (
            "/agent/claim",
            400,
            "This link does not name a claim. Ask the agent for a new one.",
        ),
        (
            "/agent/claim?claim_attempt_token=cat_unknown",
            404,
            "This link does not name a claim. Ask the agent for a new one.",
        ),
    ] {
        let refused = page(client, path).send().await;
        assert_eq!(refused.status(), status, "{path}: {}", refused.body_text());
        assert_consent_headers(&refused, path);
        let html = refused.body_text();
        assert!(
            html.contains(&format!("id=\"error-status\">{status}<")) && html.contains(message),
            "{path}: {html}"
        );
    }
}

/// Run `case`, then say so on its own line, which the harness reads.
macro_rules! case {
    ($name:ident ( $($argument:expr),* )) => {{
        let value = $name($($argument),*).await;
        println!("case {} ... ok", stringify!($name));
        value
    }};
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_pages_serve_their_flows() {
    // The task-local service container follows every await across runtime
    // worker threads.
    TestContainer::scope(async {
        let world = boot().await;
        case!(agent_017_live_migrator_creates_the_plugins_tables_and_the_email_index(&world));
        case!(agent_017_live_serves_the_endpoint_and_both_plugins(&world));
        case!(agent_017_live_login_ignores_a_foreign_return_to(&world));
        case!(agent_017_live_login_returns_to_a_same_origin_address(
            &world
        ));
        case!(agent_017_live_registration_continues_to_the_intended_address(&world));
        case!(agent_008_live_registration_lowercases_and_refuses_a_mixed_case_duplicate(&world));
        case!(kit_live_registration_refuses_a_name_over_255_and_an_email_over_254(&world));
        // One verified user who signs in through the approval's return
        // address and then claims an agent.
        let email = "ada@example.test";
        let answer = register(&world.client, "Ada Lovelace", email).await;
        assert_eq!(live_redirect(&answer), "/verify-email", "{answer}");
        verify_email(&world, email).await;
        sign_out(&world.client).await;
        let client_token = case!(agent_017_live_approval_round_trip(&world, email));
        let agent_token = case!(agent_017_live_claim_round_trip(&world, email));
        case!(
            agent_017_live_connected_agents_lists_and_revokes_only_the_users_own(
                &world,
                &client_token,
                &agent_token
            )
        );
        case!(agent_017_live_consent_forms_need_the_csrf_token(&world));
        case!(agent_017_live_consent_pages_refuse_framing_caching_and_referrers(&world));
    })
    .await;
}
