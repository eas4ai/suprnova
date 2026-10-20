//! Agents in the dogfood application: the MCP endpoint at `/mcp`, the OAuth
//! server and agent auth `src/agents.rs` serves, the approval page, the
//! sign-in it sends a guest to, and the three tools: `posts.list` for
//! anyone, `posts.create` for a signed-in user, and the headless Live tool
//! `tally.add`.
//!
//! Each test boots the router `cmd/main.rs` serves, agents and Live and the
//! RenderCache included, behind the global middleware stack
//! `bootstrap::register_http_stack` installs, over a fresh in-memory
//! database with every migration of the application's migrator. Requests go
//! through `TestClient`, which keeps cookies between requests as a browser
//! does. The signing key agent auth needs is one the test process makes
//! (`install_test_agent_auth_key`), so no key sits in the source.

mod test_environment;

use std::sync::Arc;
use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::render_cache::{CoordinatorConfig, L1Config, RenderCacheConfig};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{App, EncryptionKey, MiddlewareRegistry, Model, ToolCaller, attrs};
use tokio::sync::{Mutex, MutexGuard};

use app::agents::scope;
use app::live::providers::upload_finalizer::AppUploadFinalizer;
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;

/// Every test here swaps the process-wide container, database and global
/// middleware, so they run one at a time under `cargo test`.
static LOCK: Mutex<()> = Mutex::const_new(());

/// A client registers localhost without a port, then opens its listener on
/// a port of its own. Authorization and exchange must accept that port.
const REGISTERED_LOOPBACK: &str = "http://localhost/callback";
const LOOPBACK: &str = "http://localhost:7777/callback";
/// A PKCE verifier and its S256 challenge, `BASE64URL(SHA-256(verifier))`.
const VERIFIER: &str = "kit-agent-pages-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";
const CHALLENGE: &str = "P7iaGGMa6Y5nSTrWAxNQfIwnwsSaBJZkwT3UeiTntw8";
/// Every account's password.
const PASSWORD: &str = "dogfood-agents-password-42";

/// One booted application and the lock that keeps it to itself.
struct Dogfood {
    /// A browser with no one signed in, until a test signs someone in.
    client: TestClient,
    connection: sea_orm::DatabaseConnection,
    _lock: MutexGuard<'static, ()>,
}

/// Boot the application as `cmd/main.rs` serves it, minus the listener.
async fn boot() -> Dogfood {
    test_environment::testing();
    let lock = LOCK.lock().await;
    suprnova::testing::install_test_agent_auth_key();
    suprnova::Crypt::init(EncryptionKey::generate());

    let connection = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&connection, None)
        .await
        .expect("the application's migrations apply");
    App::singleton(suprnova::DbConnection::from_raw(connection.clone()));
    // The user provider and auth manager `bootstrap::register` installs: the
    // sign-in finds a user by address through them, and a token's user by id.
    app::bootstrap::register_auth().expect("register the auth manager");
    suprnova::container::provider::bootstrap().expect("register injectable services");
    // The cache `Server::run` boots: the session lock, the rate limits and
    // the tool budgets count in it.
    App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(suprnova::InMemoryCache::new()));
    // The rate limiter `bootstrap::register` binds: the OAuth server draws
    // its registration budget from it, and refuses to run without one.
    suprnova::rate_limit::bootstrap_default().await;

    // The Live bindings `bootstrap::register` installs.
    App::singleton(app::live::registry().expect("Live component registry"));
    App::singleton(
        suprnova::live::LiveUploadHost::new()
            .with_finalizer(Arc::new(AppUploadFinalizer::default())),
    );
    app::live::providers::authorize_live();

    suprnova::middleware::clear_global_middleware_for_test();
    app::bootstrap::register_http_stack();
    let mut config = RenderCacheConfig::from_env().expect("render cache configuration");
    // An ambient RenderCache profile must not move where these tests read.
    config.l1 = L1Config::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let routes = app::agents::serve(app::routes::register())
        .expect("src/agents.rs serves the agents with the test key");
    let router = app::live::routes_with_render_cache_with_config(routes, config)
        .await
        .expect("install Live routes and the RenderCache middleware");
    let runtime = prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    App::singleton(runtime);

    Dogfood {
        // Password hashing in a debug build takes its time.
        client: TestClient::new(router, MiddlewareRegistry::from_global())
            .timeout(Duration::from_secs(60)),
        connection,
        _lock: lock,
    }
}

/// A user whose password is [`PASSWORD`].
async fn user(name: &str, email: &str) -> User {
    let password = suprnova::hashing::hash_async(PASSWORD)
        .await
        .expect("hash the password");
    User::create(attrs! {
        name: name,
        email: email,
        password: password,
    })
    .await
    .unwrap_or_else(|error| panic!("create {email}: {error}"))
}

/// `user` as a token caller with `scopes`, as the OAuth server's resolver
/// makes one.
fn caller(user: &User, scopes: &[suprnova::agents::Scope]) -> ToolCaller {
    ToolCaller::user(Arc::new(user.clone())).scopes(scopes.iter().copied())
}

/// `value` percent-encoded for a query, every byte but the unreserved ones.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// `value` with each `%XX` turned back into its byte.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = value
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8(out).expect("UTF-8")
}

/// The value of the query parameter `name` in `url`.
fn query_param(url: &str, name: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (decode(key) == name).then(|| decode(value))
    })
}

/// The path of `address` and its query parameters, decoded and sorted: the
/// request it makes, whatever order and encoding its parameters take.
fn request_of(address: &str) -> (String, Vec<(String, String)>) {
    let (path, query) = address.split_once('?').unwrap_or((address, ""));
    let mut pairs: Vec<(String, String)> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), decode(value))
        })
        .collect();
    pairs.sort();
    (path.to_owned(), pairs)
}

/// The path and query of `url`, without a scheme and host.
fn path_of(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => rest
            .find('/')
            .map_or_else(|| "/".to_owned(), |at| rest[at..].to_owned()),
        None => url.to_owned(),
    }
}

/// Where a redirect sends the browser.
fn location(response: &TestResponse) -> String {
    response
        .header("location")
        .unwrap_or_else(|| {
            panic!(
                "a redirect, not {}: {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

/// Where an Inertia location visit sends the browser.
fn location_visit(response: &TestResponse) -> String {
    assert_eq!(response.status(), 409, "{}", response.body_text());
    response
        .header("x-inertia-location")
        .unwrap_or_else(|| panic!("no X-Inertia-Location: {}", response.body_text()))
        .to_owned()
}

/// The resource every token is for: the MCP endpoint's URL, as the
/// protected resource metadata names it.
async fn resource(client: &TestClient) -> String {
    let document = client
        .get("/.well-known/oauth-protected-resource/mcp")
        .send()
        .await;
    assert_eq!(document.status(), 200, "{}", document.body_text());
    document.json()["resource"]
        .as_str()
        .expect("the resource")
        .to_owned()
}

/// Register a loopback MCP client dynamically (RFC 7591), as Claude Code
/// does on its first sign-in; its id.
async fn register_client(client: &TestClient, name: &str) -> String {
    let registered = client
        .post("/oauth/register")
        .json(&json!({
            "client_name": name,
            "redirect_uris": [REGISTERED_LOOPBACK],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
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

/// The authorization request a client sends the browser to, asking for
/// `posts:write`.
fn authorize_path(client_id: &str, resource: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={CHALLENGE}\
         &code_challenge_method=S256&resource={}&scope={}&state=state-123",
        encode(client_id),
        encode(LOOPBACK),
        encode(resource),
        encode(scope::POSTS_WRITE.name()),
    )
}

/// The `XSRF-TOKEN` cookie the client holds, which the Inertia client
/// sends back as `X-XSRF-TOKEN`.
fn xsrf(response: &TestResponse) -> String {
    response
        .cookie("XSRF-TOKEN")
        .expect("the page set the XSRF-TOKEN cookie")
}

/// Open `login` (the sign-in page, with any `return_to`) and post the
/// sign-in form as the Inertia client does; the answer to the form.
async fn sign_in_from(
    client: &TestClient,
    login: &str,
    email: &str,
    password: &str,
) -> TestResponse {
    let page = client.get(login).inertia().send().await;
    assert_eq!(page.status(), 200, "GET {login}: {}", page.body_text());
    assert_eq!(page.inertia_page()["component"], "auth/Login");
    client
        .post("/login")
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&page))
        .json(&json!({ "email": email, "password": password, "remember": false }))
        .send()
        .await
}

#[tokio::test]
async fn agent_dogfood_lists_every_tool_without_a_token() {
    let app = boot().await;
    let listed = app.client.mcp().list().await;
    assert_eq!(listed.status(), 200, "{listed:?}");
    let mut names = listed.tool_names();
    names.sort();
    assert_eq!(
        names,
        vec!["posts.create", "posts.list", "tally.add"],
        "a client without a token sees every tool, so calling one that needs a token sends it to sign in"
    );
}

#[tokio::test]
async fn agent_dogfood_runs_the_public_tool_without_a_token() {
    let app = boot().await;
    let author = user("Ada", "ada@example.test").await;
    for (title, public) in [("Open", true), ("Draft", false)] {
        Post::create(attrs! {
            author_id: author.id,
            title: title,
            body: "Text",
            is_public: public,
        })
        .await
        .expect("create a post");
    }

    let result = app.client.mcp().call("posts.list", json!({})).await;
    assert_eq!(result.status(), 200, "{:?}", result.response());
    assert!(!result.is_error(), "{}", result.text());
    let structured = result.structured().expect("structured content");
    let titles: Vec<&str> = structured["posts"]
        .as_array()
        .expect("posts")
        .iter()
        .filter_map(|post| post["title"].as_str())
        .collect();
    assert_eq!(titles, vec!["Open"], "only public posts: {structured}");
}

#[tokio::test]
async fn agent_dogfood_sends_a_caller_without_a_token_to_sign_in_for_the_user_tool() {
    let app = boot().await;
    let result = app
        .client
        .mcp()
        .call(
            "posts.create",
            json!({ "title": "No token", "body": "Text" }),
        )
        .await;
    assert_eq!(result.status(), 401, "{:?}", result.response());
    let www = result
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(www.starts_with("Bearer "), "{www}");
    assert!(
        www.contains(r#"resource_metadata=""#)
            && www.contains(r#"/.well-known/oauth-protected-resource/mcp""#),
        "{www}"
    );
    assert!(www.contains(r#"scope="posts:write""#), "{www}");
    let count = Post::query().count().await.expect("count the posts");
    assert_eq!(count, 0, "the tool did not run");
}

#[tokio::test]
async fn agent_dogfood_serves_the_discovery_documents() {
    let app = boot().await;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = app.client.get(path).send().await;
        assert_eq!(response.status(), 200, "{path}: {}", response.body_text());
        let document = response.json();
        assert!(
            document["resource"]
                .as_str()
                .is_some_and(|resource| resource.ends_with("/mcp")),
            "{path}: {document}"
        );
        assert_eq!(
            document["scopes_supported"],
            json!(["posts:write"]),
            "{path}"
        );
        assert!(
            document["authorization_servers"]
                .as_array()
                .is_some_and(|servers| servers.len() == 1),
            "{path}: {document}"
        );
    }
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        let response = app.client.get(path).send().await;
        assert_eq!(response.status(), 200, "{path}: {}", response.body_text());
        let document = response.json();
        assert!(
            document["authorization_endpoint"]
                .as_str()
                .is_some_and(|url| url.ends_with("/oauth/authorize")),
            "{path}: {document}"
        );
        assert!(
            document["agent_auth"]["identity_endpoint"]
                .as_str()
                .is_some_and(|url| url.ends_with("/agent/identity")),
            "agent auth is on: {document}"
        );
    }
}

#[tokio::test]
async fn agent_dogfood_user_tool_writes_a_post_for_the_user() {
    let app = boot().await;
    let ada = user("Ada", "ada@example.test").await;

    // A token without the write scope reaches nothing.
    let refused = app
        .client
        .mcp()
        .as_caller(caller(&ada, &[]))
        .call(
            "posts.create",
            json!({ "title": "No scope", "body": "Text" }),
        )
        .await;
    assert_eq!(refused.status(), 403, "{:?}", refused.response());
    assert!(
        refused
            .response()
            .header("www-authenticate")
            .is_some_and(|www| www.contains("insufficient_scope")),
        "{:?}",
        refused.response()
    );

    let result = app
        .client
        .mcp()
        .as_caller(caller(&ada, &[scope::POSTS_WRITE]))
        .call(
            "posts.create",
            json!({ "title": "From an agent", "body": "Written over MCP", "is_public": true }),
        )
        .await;
    assert!(!result.is_error(), "{}", result.text());
    let structured = result.structured().expect("structured content").clone();
    assert_eq!(structured["title"], "From an agent");
    assert_eq!(structured["author_id"], ada.id);
    let id = structured["id"].as_i64().expect("the post's id");
    let post = Post::find_by_id(id)
        .await
        .expect("read the post")
        .expect("the post exists");
    assert_eq!(post.author_id, ada.id, "the post is the caller's");
    assert_eq!(post.body, "Written over MCP");
    assert!(post.is_public);

    // The form's rules come back as a tool error naming the field.
    let invalid = app
        .client
        .mcp()
        .as_caller(caller(&ada, &[scope::POSTS_WRITE]))
        .call("posts.create", json!({ "title": "  ", "body": "Text" }))
        .await;
    assert!(invalid.is_error(), "{:?}", invalid.response());
    assert!(invalid.text().contains("title"), "{}", invalid.text());
}

#[tokio::test]
async fn agent_dogfood_headless_live_tool_runs_for_a_user_and_keeps_its_state() {
    let app = boot().await;
    let ada = user("Ada", "ada@example.test").await;
    let mcp = || app.client.mcp().as_caller(caller(&ada, &[]));

    let first = mcp()
        .call(
            "tally.add",
            json!({ "mount": { "start": 2 }, "arguments": { "amount": 3 } }),
        )
        .await;
    assert!(!first.is_error(), "{:?}", first.response());
    let structured = first.structured().expect("structured content");
    assert_eq!(structured["outcome"], "accepted", "{structured}");
    assert_eq!(structured["fields"]["count"], "5", "{structured}");
    let handle = first.handle().expect("a handle").to_owned();

    // The handle carries the state to the next call.
    let second = mcp()
        .call(
            "tally.add",
            json!({ "handle": handle, "arguments": { "amount": 4 } }),
        )
        .await;
    assert!(!second.is_error(), "{:?}", second.response());
    assert_eq!(
        second
            .structured()
            .map(|structured| structured["fields"]["count"].clone()),
        Some(json!("9"))
    );

    // Another user cannot act on Ada's tally.
    let bo = user("Bo", "bo@example.test").await;
    let other = app
        .client
        .mcp()
        .as_caller(caller(&bo, &[]))
        .call(
            "tally.add",
            json!({ "handle": second.handle().unwrap_or_default(), "arguments": { "amount": 1 } }),
        )
        .await;
    assert!(other.is_error(), "{:?}", other.response());
}

#[tokio::test]
async fn agent_dogfood_headless_live_tool_needs_a_token() {
    let app = boot().await;
    let result = app
        .client
        .mcp()
        .call(
            "tally.add",
            json!({ "mount": { "start": 2 }, "arguments": { "amount": 3 } }),
        )
        .await;
    assert_eq!(result.status(), 401, "{:?}", result.response());
    assert!(result.structured().is_none(), "the tool did not run");
    assert!(
        result
            .response()
            .header("www-authenticate")
            .is_some_and(|www| www.contains("resource_metadata=")),
        "{:?}",
        result.response()
    );
}

#[tokio::test]
async fn agent_dogfood_approval_page_shows_the_client_and_its_scope() {
    let app = boot().await;
    let ada = user("Ada", "ada@example.test").await;
    let resource = resource(&app.client).await;
    let client_id = register_client(&app.client, "Dogfood Agent").await;

    let signed_in = app.client.clone().acting_as(&ada);
    let authorized = signed_in
        .get(authorize_path(&client_id, &resource))
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(authorized.status(), 302, "{}", authorized.body_text());
    let page_path = path_of(&location(&authorized));
    assert!(page_path.starts_with("/oauth/approve?"), "{page_path}");
    let id = query_param(&page_path, "authorization").expect("an authorization id");

    let page = signed_in.get(&page_path).inertia().send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let page = page.inertia_page();
    assert_eq!(page["component"], "Agents/Approve");
    let approval = &page["props"]["approval"];
    assert_eq!(approval["id"], id.as_str());
    assert_eq!(approval["client_name"], "Dogfood Agent");
    assert_eq!(approval["client_host"], "localhost");
    assert_eq!(approval["redirect_uri"], LOOPBACK);
    assert_eq!(
        approval["scopes"],
        json!([{ "name": "posts:write", "description": "Write posts for you" }])
    );

    // A guest who opens the page goes to sign in first.
    let guest = app
        .client
        .get(&page_path)
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(
        path_of(&location(&guest)),
        "/login",
        "{}",
        guest.body_text()
    );
}

#[tokio::test]
async fn agent_dogfood_a_guest_signs_in_and_approves_a_client_whose_token_writes_a_post() {
    let app = boot().await;
    let ada = user("Ada", "ada@example.test").await;
    let resource = resource(&app.client).await;
    let client_id = register_client(&app.client, "Dogfood Agent").await;
    let authorize = authorize_path(&client_id, &resource);

    // The authorization endpoint sends a guest to sign in, with the request
    // to come back to.
    let sent = app
        .client
        .get(&authorize)
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(sent.status(), 302, "{}", sent.body_text());
    let login = path_of(&location(&sent));
    assert!(login.starts_with("/login?"), "{login}");
    let return_to = query_param(&login, "return_to").expect("a return_to");
    assert_eq!(request_of(&return_to), request_of(&authorize), "{login}");

    // The sign-in continues there: a location visit, since the
    // authorization endpoint is no page the Inertia client renders.
    let signed_in = sign_in_from(&app.client, &login, "ada@example.test", PASSWORD).await;
    let back = path_of(&location_visit(&signed_in));
    assert_eq!(request_of(&back), request_of(&authorize), "{back}");

    let approve = app
        .client
        .get(&back)
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(approve.status(), 302, "{}", approve.body_text());
    let page_path = path_of(&location(&approve));
    let id = query_param(&page_path, "authorization").expect("an authorization id");
    let page = app.client.get(&page_path).inertia().send().await;
    assert_eq!(page.inertia_page()["component"], "Agents/Approve");

    // Allow: the browser leaves for the client with the code.
    let decided = app
        .client
        .post("/oauth/approve")
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&page))
        .json(&json!({ "authorization": id, "decision": "approve" }))
        .send()
        .await;
    let back = location_visit(&decided);
    assert!(back.starts_with(&format!("{LOOPBACK}?")), "{back}");
    assert_eq!(query_param(&back, "state").as_deref(), Some("state-123"));
    let code = query_param(&back, "code").expect("the code");

    // The client exchanges the code, and its token writes a post as Ada.
    let tokens = app
        .client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", LOOPBACK),
            ("client_id", client_id.as_str()),
            ("code_verifier", VERIFIER),
            ("resource", resource.as_str()),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    let result = app
        .client
        .mcp()
        .with_token(&token)
        .call(
            "posts.create",
            json!({ "title": "Through the token", "body": "Text" }),
        )
        .await;
    assert!(!result.is_error(), "{:?}", result.response());
    let structured = result.structured().expect("structured content");
    assert_eq!(structured["author_id"], ada.id);

    // The same real OAuth token can start a Live component with no browser.
    let tally = app
        .client
        .mcp()
        .with_token(&token)
        .call(
            "tally.add",
            json!({ "mount": { "start": 1 }, "arguments": { "amount": 2 } }),
        )
        .await;
    assert!(!tally.is_error(), "{:?}", tally.response());
    assert_eq!(
        tally.structured().expect("tally state")["fields"]["count"],
        "3"
    );

    // The connected-agents page lists the client.
    let connected = app.client.get("/connected-agents").inertia().send().await;
    assert_eq!(connected.status(), 200, "{}", connected.body_text());
    let csrf = xsrf(&connected);
    let connected = connected.inertia_page();
    assert_eq!(connected["component"], "Agents/Connected");
    assert_eq!(
        connected["props"]["clients"][0]["client_name"],
        "Dogfood Agent"
    );

    let grant = connected["props"]["clients"][0]["id"]
        .as_str()
        .expect("the client grant");
    let revoked = app
        .client
        .post(format!("/connected-agents/clients/{grant}/revoke"))
        .inertia()
        .header("X-XSRF-TOKEN", csrf)
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(revoked.status(), 303, "{}", revoked.body_text());
    let refused = app
        .client
        .mcp()
        .with_token(&token)
        .call(
            "posts.create",
            json!({ "title": "Revoked", "body": "Text" }),
        )
        .await;
    assert_eq!(refused.status(), 401, "{:?}", refused.response());
}

#[tokio::test]
async fn agent_dogfood_a_guest_signs_in_claims_an_agent_and_revokes_it() {
    let app = boot().await;
    let mut ada = user("Ada", "ada@example.test").await;
    ada.email_verified_at = Some(chrono::Utc::now());
    ada.save().await.expect("verify Ada's address");
    let registered = app
        .client
        .post("/agent/identity")
        .json(&json!({ "type": "service_auth", "login_hint": "ada@example.test" }))
        .send()
        .await;
    assert_eq!(registered.status(), 200, "{}", registered.body_text());
    let registration = registered.json();
    let claim = &registration["claim"];
    let path = path_of(claim["verification_uri"].as_str().expect("the claim page"));
    let attempt = query_param(&path, "claim_attempt_token").expect("an attempt token");

    let guest = app
        .client
        .get(&path)
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(path_of(&location(&guest)), "/login");
    let signed_in = sign_in_from(&app.client, "/login", "ada@example.test", PASSWORD).await;
    assert_eq!(path_of(&location_visit(&signed_in)), path);
    let page = app.client.get(&path).inertia().send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert_eq!(page.header("referrer-policy"), Some("no-referrer"));
    let props = page.inertia_page();
    assert_eq!(props["component"], "Agents/Claim");
    assert_eq!(props["props"]["claim"]["signed_in_as"], "ada@example.test");
    assert_eq!(props["props"]["claim"]["scopes"][0]["name"], "posts:write");
    let code = claim["user_code"].as_str().expect("the claim code");
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let refused = app
        .client
        .post("/agent/claim")
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&page))
        .json(&json!({ "claim_attempt_token": attempt, "user_code": wrong }))
        .send()
        .await;
    assert_eq!(request_of(&location(&refused)), request_of(&path));
    let page = app.client.get(&path).inertia().send().await;
    assert!(page.inertia_page()["props"]["errors"]["user_code"].is_string());
    let confirmed = app
        .client
        .post("/agent/claim")
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&page))
        .json(&json!({ "claim_attempt_token": attempt, "user_code": code }))
        .send()
        .await;
    assert_eq!(confirmed.status(), 303, "{}", confirmed.body_text());
    assert_eq!(path_of(&location(&confirmed)), "/connected-agents");

    let connected = app.client.get("/connected-agents").inertia().send().await;
    assert_eq!(
        connected.inertia_page()["props"]["agents"][0]["id"],
        registration["registration_id"]
    );
    let resource = resource(&app.client).await;
    let tokens = app
        .client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
            (
                "claim_token",
                registration["claim_token"]
                    .as_str()
                    .expect("the claim token"),
            ),
            ("resource", resource.as_str()),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    let listed = app.client.mcp().with_token(&token).list().await;
    assert_eq!(listed.status(), 200, "{listed:?}");

    let id = registration["registration_id"]
        .as_str()
        .expect("the registration id");
    let revoked = app
        .client
        .post(format!("/connected-agents/agents/{id}/revoke"))
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&connected))
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(revoked.status(), 303, "{}", revoked.body_text());
    let listed = app.client.mcp().with_token(&token).list().await;
    assert_eq!(listed.status(), 401, "{listed:?}");
}

#[tokio::test]
async fn agent_dogfood_sign_in_refuses_a_wrong_password_and_a_foreign_return_to() {
    let app = boot().await;
    user("Ada", "ada@example.test").await;

    let wrong = sign_in_from(
        &app.client,
        "/login",
        "ada@example.test",
        "not-the-password",
    )
    .await;
    assert!(
        matches!(wrong.status(), 302 | 303),
        "a failed sign-in goes back to the form: {} {}",
        wrong.status(),
        wrong.body_text()
    );
    let guarded = app
        .client
        .get("/connected-agents")
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(path_of(&location(&guarded)), "/login", "still a guest");

    // A sign-in link that names another site continues on this one.
    let signed_in = sign_in_from(
        &app.client,
        "/login?return_to=https%3A%2F%2Fevil.example%2Fsteal",
        "ada@example.test",
        PASSWORD,
    )
    .await;
    let destination = match signed_in.status() {
        409 => location_visit(&signed_in),
        _ => location(&signed_in),
    };
    assert_eq!(path_of(&destination), "/", "{destination}");
    assert!(!destination.contains("evil"), "{destination}");
}

#[tokio::test]
async fn agent_dogfood_registration_signs_the_new_user_in_at_the_same_origin_return_to() {
    let app = boot().await;
    user("Dee", "dee@example.test").await;
    let page = app
        .client
        .get("/register?return_to=%2Fconnected-agents")
        .inertia()
        .send()
        .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert_eq!(page.inertia_page()["component"], "auth/Register");
    let register = |name: &'static str, email: &'static str| {
        app.client
            .post("/register")
            .header("Accept", "application/json")
            .header("X-XSRF-TOKEN", xsrf(&page))
            .json(&json!({
                "name": name,
                "email": email,
                "password": PASSWORD,
                "password_confirmation": PASSWORD,
            }))
            .send()
    };

    // An address that has an account, in another case, is refused.
    let duplicate = register("Dee again", "DEE@Example.test").await;
    assert_eq!(duplicate.status(), 422, "{}", duplicate.body_text());
    assert!(
        duplicate.json()["errors"]["email"].is_array(),
        "{}",
        duplicate.body_text()
    );

    let registered = register("Cy", " Cy@Example.test ").await;
    assert!(
        matches!(registered.status(), 302 | 303 | 409),
        "{} {}",
        registered.status(),
        registered.body_text()
    );
    let destination = match registered.status() {
        409 => location_visit(&registered),
        _ => location(&registered),
    };
    assert_eq!(path_of(&destination), "/connected-agents", "{destination}");
    let created = User::query()
        .filter("email", "cy@example.test")
        .first()
        .await
        .expect("read the user")
        .expect("the account exists under the folded address");
    assert_ne!(created.password, PASSWORD, "the password is stored hashed");
    assert!(
        suprnova::hashing::verify(PASSWORD, &created.password).expect("verify the hash"),
        "the stored hash is the password's"
    );
    let connected = app.client.get("/connected-agents").inertia().send().await;
    assert_eq!(
        connected.status(),
        200,
        "signed in: {}",
        connected.body_text()
    );
}

#[tokio::test]
async fn agent_dogfood_registration_ignores_a_foreign_return_to() {
    let app = boot().await;
    let page = app
        .client
        .get("/register?return_to=%2F%2Fevil.example")
        .inertia()
        .send()
        .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let registered = app
        .client
        .post("/register")
        .inertia()
        .header("X-XSRF-TOKEN", xsrf(&page))
        .json(&json!({
            "name": "Cy",
            "email": "cy@example.test",
            "password": PASSWORD,
            "password_confirmation": PASSWORD,
        }))
        .send()
        .await;
    assert!(
        matches!(registered.status(), 302 | 303 | 409),
        "{} {}",
        registered.status(),
        registered.body_text()
    );
    let destination = match registered.status() {
        409 => location_visit(&registered),
        _ => location(&registered),
    };
    assert_eq!(path_of(&destination), "/", "{destination}");
    assert!(!destination.contains("evil"), "{destination}");
}

#[tokio::test]
async fn agent_dogfood_an_existing_database_takes_the_agents_tables_on_the_next_migrate() {
    let app = boot().await;
    let total = Migrator::migrations().len();
    // The agents migrations and the address index come last, so a database
    // migrated before them has every other one.
    let added = 3;
    let existing = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&existing, Some((total - added) as u32))
        .await
        .expect("migrate as before the agents");
    assert_eq!(
        Migrator::get_pending_migrations(&existing)
            .await
            .expect("read the pending migrations")
            .len(),
        added
    );
    Migrator::up(&existing, None)
        .await
        .expect("the next migrate applies the agents migrations");
    for connection in [&existing, &app.connection] {
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
            let found = connection
                .query_one_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    format!(
                        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
                    ),
                ))
                .await
                .expect("read the schema");
            assert!(found.is_some(), "{table} exists");
        }
        let index = connection
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND tbl_name = 'users' \
                 AND sql LIKE '%UNIQUE%LOWER%'"
                    .to_owned(),
            ))
            .await
            .expect("read the schema");
        assert!(index.is_some(), "one account per address, ignoring case");
    }
}
