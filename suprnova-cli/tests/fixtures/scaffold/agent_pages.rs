//! AGENT-017 in a generated Inertia application: the MCP endpoint, the
//! OAuth server and agent auth that `src/agents.rs` serves, the approval,
//! claim and connected-agents pages of `controllers::agents`, the return
//! address a sign-in link carries, and the kit's migrator on SQLite.
//!
//! Written into a freshly scaffolded Vue project by `scaffold_snapshot.rs`
//! (`agent_017_a_generated_inertia_application_serves_the_agent_pages`),
//! which replaces `__PACKAGE__` with the generated crate's name and runs it
//! under the settings `suprnova new` wrote into `.env`, the signing key
//! included. Every request goes through the application's own router and
//! middleware: the session, CSRF, Inertia, security headers and error page.
//!
//! One test runs every case in turn, on one database, and prints
//! `case <name> ... ok` after each; the harness checks every line.

use std::time::Duration;

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use suprnova::serde_json::{Value, json};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{MiddlewareRegistry, Model};

use __PACKAGE__::models::user::User;

/// The issuer, the application's URL.
const ISSUER: &str = "http://localhost";
/// The MCP endpoint, the resource every token is for.
const RESOURCE: &str = "http://localhost/mcp";
/// A loopback MCP client's redirect URI, on another origin than the
/// application's, as every client's is.
const LOOPBACK: &str = "http://127.0.0.1:7777/callback";
/// A PKCE verifier and its S256 challenge, `BASE64URL(SHA-256(verifier))`.
const VERIFIER: &str = "kit-agent-pages-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";
const CHALLENGE: &str = "P7iaGGMa6Y5nSTrWAxNQfIwnwsSaBJZkwT3UeiTntw8";
/// Every account's password.
const PASSWORD: &str = "agent-pages-password-42";
/// The built script the harness puts under `public/`, which every page
/// loads after its document.
const SCRIPT: &str = "/assets/app-agents.js";

/// Boot the generated application the way `cmd/main.rs` does, minus the
/// listener, with every migration of its own migrator, and capture mail.
async fn setup() -> suprnova::mail::MailFake {
    use sea_orm_migration::MigratorTrait;
    assert_eq!(
        std::env::var("APP_ENV").as_deref(),
        Ok("test"),
        "the harness runs this binary with APP_ENV=test and never loads .env"
    );
    __PACKAGE__::config::register_all();
    suprnova::Crypt::init(suprnova::EncryptionKey::from_env().expect("APP_KEY is set"));
    // The application migrates before its bootstrap runs, which reads the
    // notes' search tables.
    suprnova::DB::init().await.expect("connect to the database");
    __PACKAGE__::migrations::Migrator::up(suprnova::DB::connection().unwrap().inner(), None)
        .await
        .expect("the kit's migrations apply");
    __PACKAGE__::bootstrap::register().await;
    // The cache the server binds at boot, which the rate limits count in:
    // the MCP endpoint's request limit among them.
    suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(std::sync::Arc::new(
        suprnova::InMemoryCache::new(),
    ));
    suprnova::rate_limit::bootstrap_default().await;
    __PACKAGE__::bootstrap::register_http_stack();
    suprnova::Mail::fake()
}

/// The application's routes with the endpoint, both plugins and the
/// discovery documents, as `cmd/main.rs` serves them.
fn router() -> suprnova::Router {
    __PACKAGE__::agents::serve(__PACKAGE__::routes::register())
        .expect("src/agents.rs serves the agents with the key in .env")
}

/// A browser of its own, with its own cookies, or a client that sends
/// none, such as an MCP client or an agent.
struct Browser {
    client: TestClient,
    /// The `XSRF-TOKEN` cookie's value, which the Inertia client sends
    /// back as `X-XSRF-TOKEN`.
    xsrf: Option<String>,
}

impl Browser {
    fn new() -> Self {
        Self {
            // `TestClient` sends `Host: localhost`, the host of `APP_URL`.
            client: TestClient::new(router(), MiddlewareRegistry::from_global())
                // Password hashing in a debug build takes its time.
                .timeout(Duration::from_secs(60)),
            xsrf: None,
        }
    }

    fn keep(&mut self, response: TestResponse) -> TestResponse {
        if let Some(token) = response.cookie("XSRF-TOKEN") {
            self.xsrf = Some(token);
        }
        response
    }

    /// A document load: a link followed, an address typed.
    async fn open(&mut self, path: &str) -> TestResponse {
        let response = self
            .client
            .get(path)
            .header("Accept", "text/html")
            .send()
            .await;
        self.keep(response)
    }

    /// A script the page loads, as the browser fetches it after the page:
    /// the session may record it as the previous URL, and a consent page
    /// sends no referrer, so nothing here may go back to it.
    async fn load_script(&mut self) {
        let script = self
            .client
            .get(SCRIPT)
            .header("Accept", "*/*")
            .send()
            .await;
        assert_eq!(script.status(), 200, "GET {SCRIPT}");
    }

    /// An Inertia visit.
    async fn visit(&mut self, path: &str) -> TestResponse {
        let response = self.client.get(path).inertia().send().await;
        self.keep(response)
    }

    /// An Inertia form submission, with the CSRF token.
    async fn submit(&mut self, path: &str, body: Value) -> TestResponse {
        let token = self
            .xsrf
            .clone()
            .expect("a page set the XSRF-TOKEN cookie first");
        let response = self
            .client
            .post(path)
            .inertia()
            .header("X-XSRF-TOKEN", token)
            .json(&body)
            .send()
            .await;
        self.keep(response)
    }

    /// An Inertia form submission without the CSRF token, as another
    /// site's page would send it.
    async fn submit_without_token(&mut self, path: &str, body: Value) -> TestResponse {
        let response = self.client.post(path).inertia().json(&body).send().await;
        self.keep(response)
    }

    /// Sign in from `login` (the sign-in page, with any `return_to`) as
    /// `email`; the answer to the form.
    async fn sign_in_from(&mut self, login: &str, email: &str) -> TestResponse {
        let page = self.open(login).await;
        assert_eq!(page.status(), 200, "GET {login}: {}", page.body_text());
        self.submit(
            "/login",
            json!({ "email": email, "password": PASSWORD, "remember": false }),
        )
        .await
    }

    async fn sign_in(&mut self, email: &str) {
        let signed_in = self.sign_in_from("/login", email).await;
        assert_eq!(
            redirect_path(&signed_in),
            "/dashboard",
            "{email} signs in: {}",
            signed_in.body_text()
        );
    }

    async fn sign_out(&mut self) {
        let out = self.submit("/logout", json!({})).await;
        assert!(
            matches!(out.status(), 302 | 303),
            "sign-out: {} {}",
            out.status(),
            out.body_text()
        );
    }
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

/// `value` with its percent escapes and `+` decoded, as a query reads.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).expect("ASCII escape");
                out.push(u8::from_str_radix(hex, 16).expect("a hex escape"));
                at += 3;
            }
            b'+' => {
                out.push(b' ');
                at += 1;
            }
            byte => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).expect("UTF-8")
}

/// The path and query of an absolute address on the application, or of a
/// path. An address on another origin fails the test.
fn path_of(address: &str) -> String {
    match address.strip_prefix(ISSUER) {
        Some(path) => path.to_owned(),
        None if address.starts_with('/') && !address.starts_with("//") => address.to_owned(),
        None => panic!("{address} is not on this application"),
    }
}

/// The query parameter `name` of `address`, decoded.
fn query_param(address: &str, name: &str) -> Option<String> {
    let (_, query) = address.split_once('?')?;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| decode(key) == name)
        .map(|(_, value)| decode(value))
}

/// The path of `address` and its query parameters, decoded and sorted: the
/// request it makes, whatever order its parameters were written in.
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

/// Where a response sends the browser: its `X-Inertia-Location` or its
/// `Location`.
fn destination(response: &TestResponse) -> String {
    response
        .header("x-inertia-location")
        .or_else(|| response.header("location"))
        .unwrap_or_else(|| {
            panic!(
                "a redirect, got {} {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

/// The path an ordinary redirect (302 or 303) on the application goes to.
fn redirect_path(response: &TestResponse) -> String {
    assert!(
        matches!(response.status(), 302 | 303) && response.header("x-inertia-location").is_none(),
        "an ordinary redirect, got {} {:?} {}",
        response.status(),
        response.header("x-inertia-location"),
        response.body_text()
    );
    path_of(&destination(response))
}

/// The address a location visit (409 `X-Inertia-Location`) leaves for.
fn location_visit(response: &TestResponse) -> String {
    assert_eq!(
        response.status(),
        409,
        "a location visit, got {}",
        response.body_text()
    );
    response
        .header("x-inertia-location")
        .unwrap_or_else(|| panic!("no X-Inertia-Location: {}", response.body_text()))
        .to_owned()
}

/// A user who has not verified their address.
async fn user(name: &str, email: &str) -> User {
    User::create(name, email, PASSWORD)
        .await
        .unwrap_or_else(|error| panic!("create {email}: {error}"))
}

/// A user whose address is verified, as the mailed link records it.
async fn verified_user(name: &str, email: &str) -> User {
    let mut created = user(name, email).await;
    created.email_verified_at = Some(chrono::Utc::now());
    created
        .save()
        .await
        .unwrap_or_else(|error| panic!("verify {email}: {error}"));
    created
}

/// Register a loopback MCP client dynamically (RFC 7591); its id.
async fn register_client(machine: &mut Browser, name: &str) -> String {
    let registered = machine
        .client
        .post("/oauth/register")
        .json(&json!({
            "client_name": name,
            "redirect_uris": [LOOPBACK],
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

/// The authorization request a client sends the browser to.
fn authorize_path(client_id: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={CHALLENGE}\
         &code_challenge_method=S256&resource={}&state=state-123",
        encode(client_id),
        encode(LOOPBACK),
        encode(RESOURCE),
    )
}

/// Exchange an authorization code; the access token.
async fn exchange(machine: &mut Browser, client_id: &str, code: &str) -> String {
    let tokens = machine
        .client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", LOOPBACK),
            ("client_id", client_id),
            ("code_verifier", VERIFIER),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned()
}

/// The status `tools/list` answers with `token`.
async fn mcp_status(machine: &Browser, token: &str) -> u16 {
    machine.client.mcp().with_token(token).list().await.status()
}

/// As the signed-in `browser`, send the browser to the authorization
/// endpoint and land on the approval page; the pending authorization's id.
async fn open_approval(browser: &mut Browser, client_id: &str) -> String {
    let authorized = browser.open(&authorize_path(client_id)).await;
    assert_eq!(authorized.status(), 302, "{}", authorized.body_text());
    let approval = path_of(&destination(&authorized));
    assert!(approval.starts_with("/oauth/approve?"), "{approval}");
    query_param(&approval, "authorization").expect("the approval page names the authorization")
}

/// Approve `client_id` for the signed-in `browser` and exchange the code;
/// the access token.
async fn connect_client(browser: &mut Browser, machine: &mut Browser, client_id: &str) -> String {
    let id = open_approval(browser, client_id).await;
    let decided = browser
        .submit(
            "/oauth/approve",
            json!({ "authorization": id, "decision": "approve" }),
        )
        .await;
    let back = location_visit(&decided);
    let code = query_param(&back, "code").expect("the code");
    exchange(machine, client_id, &code).await
}

/// Register an agent by `service_auth` for `email`; the registration.
async fn service_auth(machine: &mut Browser, email: &str) -> Value {
    let registered = machine
        .client
        .post("/agent/identity")
        .json(&json!({ "type": "service_auth", "login_hint": email }))
        .send()
        .await;
    assert_eq!(registered.status(), 200, "{}", registered.body_text());
    registered.json()
}

/// Poll for a claimed agent's tokens; the access token.
async fn poll(machine: &mut Browser, claim_token: &str) -> String {
    let tokens = machine
        .client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
            ("claim_token", claim_token),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned()
}

/// Have the signed-in, verified `browser` claim a `service_auth` agent
/// registered for `email`; the agent's access token.
async fn claim_agent(browser: &mut Browser, machine: &mut Browser, email: &str) -> String {
    let registration = service_auth(machine, email).await;
    let claim = &registration["claim"];
    let page = path_of(
        claim["verification_uri"]
            .as_str()
            .expect("a verification URI"),
    );
    let token = query_param(&page, "claim_attempt_token").expect("an attempt token");
    assert_eq!(browser.open(&page).await.status(), 200);
    let confirmed = browser
        .submit(
            "/agent/claim",
            json!({
                "claim_attempt_token": token,
                "user_code": claim["user_code"].as_str().expect("a user code"),
            }),
        )
        .await;
    assert_eq!(redirect_path(&confirmed), "/connected-agents");
    poll(
        machine,
        registration["claim_token"].as_str().expect("a claim token"),
    )
    .await
}

/// The signed-in `browser`'s connected agents page props.
async fn connected(browser: &mut Browser) -> Value {
    let page = browser.visit("/connected-agents").await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let page = page.inertia_page();
    assert_eq!(page["component"], "Agents/Connected");
    page["props"].clone()
}

// ---------------------------------------------------------------- cases ----

/// The endpoint, both plugins' discovery documents and auth.md, and an
/// anonymous agent registration.
async fn agent_017_inertia_serves_the_endpoint_and_both_plugins() {
    let machine = Browser::new();
    let init = machine.client.mcp().initialize().await;
    assert_eq!(init.status(), 200, "{init:?}");
    assert!(
        init.result()
            .is_some_and(|result| result["protocolVersion"].is_string())
    );

    let resource = machine
        .client
        .get("/.well-known/oauth-protected-resource/mcp")
        .send()
        .await;
    assert_eq!(resource.status(), 200, "{}", resource.body_text());
    assert_eq!(resource.json()["resource"], RESOURCE);

    let metadata = machine
        .client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await;
    assert_eq!(metadata.status(), 200, "{}", metadata.body_text());
    let metadata = metadata.json();
    assert_eq!(metadata["issuer"], ISSUER);
    assert_eq!(metadata["agent_auth"]["skill"], format!("{ISSUER}/auth.md"));
    assert_eq!(
        metadata["agent_auth"]["identity_endpoint"],
        format!("{ISSUER}/agent/identity")
    );

    let skill = machine.client.get("/auth.md").send().await;
    assert_eq!(skill.status(), 200);
    assert!(
        skill.body_text().contains("/agent/identity"),
        "{}",
        skill.body_text()
    );

    let anonymous = machine
        .client
        .post("/agent/identity")
        .json(&json!({ "type": "anonymous" }))
        .send()
        .await;
    assert_eq!(anonymous.status(), 200, "{}", anonymous.body_text());
    let anonymous = anonymous.json();
    assert_eq!(anonymous["registration_type"], "anonymous");
    assert!(anonymous["registration_id"].is_string(), "{anonymous}");
}

/// An MCP client's whole flow through the kit's approval page: the guest
/// signs in through the kit's sign-in page and comes back, the page shows
/// the client, and the decision leaves for the client's redirect URI by a
/// location visit with the code, the state and the issuer.
async fn agent_017_inertia_approval_round_trip() {
    verified_user("Alice", "alice@example.test").await;
    let mut machine = Browser::new();
    let client_id = register_client(&mut machine, "Kit Agent Tool").await;
    let mut alice = Browser::new();

    // A guest goes to sign in, with the authorization request to come back to.
    let authorize = authorize_path(&client_id);
    let sent = alice.open(&authorize).await;
    assert_eq!(sent.status(), 302, "{}", sent.body_text());
    let login = path_of(&destination(&sent));
    let return_to = query_param(&login, "return_to").expect("a return_to");
    assert_eq!(request_of(&return_to), request_of(&authorize), "{login}");
    let signed_in = alice.sign_in_from(&login, "alice@example.test").await;
    // The authorization endpoint is not a page the SPA renders.
    let back = location_visit(&signed_in);
    assert_eq!(path_of(&back), return_to);
    let approve = alice.open(&path_of(&back)).await;
    assert_eq!(approve.status(), 302, "{}", approve.body_text());
    let page_path = path_of(&destination(&approve));
    let id = query_param(&page_path, "authorization").expect("an authorization id");

    // The kit's page, as a first visit and as an Inertia visit.
    let first = alice.open(&page_path).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    assert_eq!(first.inertia_page()["component"], "Agents/Approve");
    let page = alice.visit(&page_path).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let approval = page.inertia_page()["props"]["approval"].clone();
    assert_eq!(approval["id"], id.as_str());
    assert_eq!(approval["client_name"], "Kit Agent Tool");
    assert_eq!(approval["client_host"], "127.0.0.1");
    assert_eq!(approval["redirect_uri"], LOOPBACK);
    assert_eq!(approval["scopes"], json!([]));
    assert!(approval.get("logo_uri").is_none() && approval.get("client_uri").is_none());
    assert!(
        page.page_tools().is_empty(),
        "a consent page offers no tool"
    );

    // Allow: a location visit to the client, never a redirect a form
    // policy would govern.
    let decided = alice
        .submit(
            "/oauth/approve",
            json!({ "authorization": id, "decision": "approve" }),
        )
        .await;
    let to_client = location_visit(&decided);
    assert!(
        to_client.starts_with(&format!("{LOOPBACK}?")),
        "{to_client}"
    );
    assert_eq!(
        query_param(&to_client, "state").as_deref(),
        Some("state-123")
    );
    assert_eq!(query_param(&to_client, "iss").as_deref(), Some(ISSUER));
    let code = query_param(&to_client, "code").expect("the code");
    let token = exchange(&mut machine, &client_id, &code).await;
    let init = machine.client.mcp().with_token(&token).initialize().await;
    assert_eq!(init.status(), 200, "{init:?}");
    let list = machine.client.mcp().with_token(&token).list().await;
    assert_eq!(list.status(), 200, "{list:?}");

    // The next page clears the browser's history of the approval page.
    let connected = alice.visit("/connected-agents").await.inertia_page();
    assert_eq!(connected["clearHistory"], true, "{connected}");
    assert_eq!(
        connected["props"]["clients"][0]["client_name"],
        "Kit Agent Tool"
    );
    // The decided authorization has ended.
    assert_eq!(alice.visit(&page_path).await.status(), 404);

    // Deny: the client gets `access_denied`.
    let id = open_approval(&mut alice, &client_id).await;
    let denied = alice
        .submit(
            "/oauth/approve",
            json!({ "authorization": id, "decision": "deny" }),
        )
        .await;
    let to_client = location_visit(&denied);
    assert_eq!(
        query_param(&to_client, "error").as_deref(),
        Some("access_denied"),
        "{to_client}"
    );

    // Another word decides nothing. The page sends no referrer, so the
    // answer is an error page rather than a way back.
    let id = open_approval(&mut alice, &client_id).await;
    let refused = alice
        .submit(
            "/oauth/approve",
            json!({ "authorization": id, "decision": "maybe" }),
        )
        .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(
        alice
            .visit(&format!("/oauth/approve?authorization={}", encode(&id)))
            .await
            .status(),
        200
    );

    // No id, an unknown id, and a guest.
    assert_eq!(alice.open("/oauth/approve").await.status(), 400);
    assert_eq!(
        alice
            .open("/oauth/approve?authorization=nope")
            .await
            .status(),
        404
    );
    let mut guest = Browser::new();
    let sent = guest
        .open(&format!("/oauth/approve?authorization={}", encode(&id)))
        .await;
    assert_eq!(redirect_path(&sent), "/login");
}

/// An agent's claim through the kit's claim page: the guest signs in and
/// comes back, the page shows the claim, a wrong code comes back as the
/// code field's error with the attempts left, the right code connects the
/// agent, and the agent's token works.
async fn agent_017_inertia_claim_round_trip() {
    verified_user("Bob", "bob@example.test").await;
    let mut machine = Browser::new();
    let registration = service_auth(&mut machine, "bob@example.test").await;
    assert_eq!(registration["registration_type"], "service_auth");
    let claim = &registration["claim"];
    let uri = claim["verification_uri"]
        .as_str()
        .expect("a verification URI");
    assert!(
        uri.starts_with(&format!("{ISSUER}/agent/claim?claim_attempt_token=")),
        "the verification URI is the kit's claim page: {uri}"
    );
    let page_path = path_of(uri);
    let attempt = query_param(&page_path, "claim_attempt_token").expect("an attempt token");
    let code = claim["user_code"].as_str().expect("a user code").to_owned();

    let mut bob = Browser::new();
    let sent = bob.open(&page_path).await;
    assert_eq!(redirect_path(&sent), "/login");
    let signed_in = bob.sign_in_from("/login", "bob@example.test").await;
    assert_eq!(path_of(&location_visit(&signed_in)), page_path);

    let first = bob.open(&page_path).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    bob.load_script().await;
    let page = bob.visit(&page_path).await;
    let page = page.inertia_page();
    assert_eq!(page["component"], "Agents/Claim");
    let props = &page["props"];
    assert_eq!(props["claim_attempt_token"], attempt.as_str());
    assert_eq!(props["claim"]["registration_type"], "service_auth");
    assert_eq!(props["claim"]["signed_in_as"], "bob@example.test");
    assert_eq!(props["claim"]["email"], "bob@example.test");
    assert_eq!(props["claim"]["provider"], Value::Null);
    let attempts = props["claim"]["attempts_remaining"]
        .as_u64()
        .expect("the attempts left");
    assert!(attempts > 1, "{props}");

    // A wrong code: back to the page, with the mistake and the attempts.
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let refused = bob
        .submit(
            "/agent/claim",
            json!({ "claim_attempt_token": attempt, "user_code": wrong }),
        )
        .await;
    assert_eq!(redirect_path(&refused), page_path);
    let again = bob.visit(&page_path).await.inertia_page();
    assert_eq!(
        again["props"]["errors"]["user_code"],
        format!("That code is wrong. {} attempts left.", attempts - 1),
        "{again}"
    );
    assert_eq!(again["props"]["claim"]["attempts_remaining"], attempts - 1);

    // The right code.
    let confirmed = bob
        .submit(
            "/agent/claim",
            json!({ "claim_attempt_token": attempt, "user_code": code }),
        )
        .await;
    assert_eq!(redirect_path(&confirmed), "/connected-agents");
    let listed = bob.visit("/connected-agents").await.inertia_page();
    assert_eq!(listed["clearHistory"], true, "{listed}");
    assert_eq!(
        listed["flash"]["toast"]["message"],
        "The agent is connected."
    );
    assert_eq!(
        listed["props"]["agents"][0]["id"],
        registration["registration_id"]
    );
    assert_eq!(
        listed["props"]["agents"][0]["registration_type"],
        "service_auth"
    );

    let token = poll(
        &mut machine,
        registration["claim_token"].as_str().expect("a claim token"),
    )
    .await;
    assert_eq!(mcp_status(&machine, &token).await, 200);

    // A claim for an address its user has not verified is refused, with the
    // words the facade's refusal gets.
    user("Carol", "carol@example.test").await;
    let registration = service_auth(&mut machine, "carol@example.test").await;
    let page_path = path_of(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("a verification URI"),
    );
    let mut carol = Browser::new();
    carol.sign_in("carol@example.test").await;
    let refused = carol.open(&page_path).await;
    assert_eq!(refused.status(), 403, "{}", refused.body_text());
    assert_eq!(
        refused.inertia_page()["props"]["message"],
        "This claim is for another account, or your address is not verified yet. Sign in with \
         the address the agent named."
    );

    // No token, and an unknown token.
    assert_eq!(bob.open("/agent/claim").await.status(), 400);
    assert_eq!(
        bob.open("/agent/claim?claim_attempt_token=cat_nope")
            .await
            .status(),
        404
    );
}

/// Two users: each sees only their own clients and agents, cannot revoke
/// the other's, and a revoke ends the tokens at once.
async fn agent_017_inertia_connected_agents_lists_and_revokes_only_the_users_own() {
    verified_user("Dave", "dave@example.test").await;
    verified_user("Erin", "erin@example.test").await;
    let mut machine = Browser::new();
    let client_id = register_client(&mut machine, "Dave's Tool").await;

    let mut dave = Browser::new();
    dave.sign_in("dave@example.test").await;
    let client_token = connect_client(&mut dave, &mut machine, &client_id).await;
    let agent_token = claim_agent(&mut dave, &mut machine, "dave@example.test").await;
    let props = connected(&mut dave).await;
    let grant = props["clients"][0]["id"]
        .as_str()
        .expect("a grant")
        .to_owned();
    let registration = props["agents"][0]["id"]
        .as_str()
        .expect("a registration")
        .to_owned();
    assert_eq!(
        props["clients"].as_array().map(Vec::len),
        Some(1),
        "{props}"
    );
    assert_eq!(props["agents"].as_array().map(Vec::len), Some(1), "{props}");

    let mut erin = Browser::new();
    erin.sign_in("erin@example.test").await;
    let props = connected(&mut erin).await;
    assert_eq!(props["clients"], json!([]), "{props}");
    assert_eq!(props["agents"], json!([]), "{props}");
    for path in [
        format!("/connected-agents/clients/{}/revoke", encode(&grant)),
        format!("/connected-agents/agents/{}/revoke", encode(&registration)),
    ] {
        let refused = erin.submit(&path, json!({})).await;
        assert_eq!(refused.status(), 404, "{path}: {}", refused.body_text());
    }
    assert_eq!(mcp_status(&machine, &client_token).await, 200);
    assert_eq!(mcp_status(&machine, &agent_token).await, 200);

    connected(&mut dave).await;
    dave.load_script().await;
    let revoked = dave
        .submit(
            &format!("/connected-agents/clients/{}/revoke", encode(&grant)),
            json!({}),
        )
        .await;
    assert_eq!(redirect_path(&revoked), "/connected-agents");
    assert_eq!(mcp_status(&machine, &client_token).await, 401);
    assert_eq!(mcp_status(&machine, &agent_token).await, 200);
    let revoked = dave
        .submit(
            &format!("/connected-agents/agents/{}/revoke", encode(&registration)),
            json!({}),
        )
        .await;
    assert_eq!(redirect_path(&revoked), "/connected-agents");
    assert_eq!(mcp_status(&machine, &agent_token).await, 401);
    let props = connected(&mut dave).await;
    assert_eq!(props["clients"], json!([]), "{props}");
    assert_eq!(props["agents"], json!([]), "{props}");

    // A guest is sent to sign in.
    let mut guest = Browser::new();
    assert_eq!(
        redirect_path(&guest.open("/connected-agents").await),
        "/login"
    );
}

/// Every consent form needs the session's CSRF token.
async fn agent_017_inertia_consent_forms_need_the_csrf_token() {
    verified_user("Frank", "frank@example.test").await;
    let mut frank = Browser::new();
    frank.sign_in("frank@example.test").await;
    for (path, body) in [
        (
            "/oauth/approve",
            json!({ "authorization": "x", "decision": "approve" }),
        ),
        (
            "/agent/claim",
            json!({ "claim_attempt_token": "x", "user_code": "123456" }),
        ),
        ("/connected-agents/clients/x/revoke", json!({})),
        ("/connected-agents/agents/x/revoke", json!({})),
    ] {
        let refused = frank.submit_without_token(path, body).await;
        assert_eq!(refused.status(), 419, "{path}: {}", refused.body_text());
    }
}

/// The consent pages' answers refuse framing, caching and referrers, on a
/// first visit, an Inertia visit and an error, and keep the rest of the
/// page's policy, its nonce among it. Another page keeps the preset's own
/// framing rule.
async fn agent_017_inertia_consent_pages_refuse_framing_caching_and_referrers() {
    verified_user("Grace", "grace.headers@example.test").await;
    let mut grace = Browser::new();
    grace.sign_in("grace.headers@example.test").await;
    let check = |what: &str, response: &TestResponse| {
        let csp = response
            .header("content-security-policy")
            .unwrap_or_else(|| panic!("{what}: no Content-Security-Policy"));
        assert!(csp.contains("frame-ancestors 'none'"), "{what}: {csp}");
        assert!(!csp.contains("frame-ancestors 'self'"), "{what}: {csp}");
        assert_eq!(response.header("x-frame-options"), Some("DENY"), "{what}");
        assert!(
            response
                .header("cache-control")
                .is_some_and(|value| value.contains("no-store")),
            "{what}: {:?}",
            response.header("cache-control")
        );
        assert_eq!(
            response.header("referrer-policy"),
            Some("no-referrer"),
            "{what}"
        );
    };
    let first = grace.open("/connected-agents").await;
    assert_eq!(first.status(), 200);
    check("a first visit", &first);
    let csp = first.header("content-security-policy").unwrap_or_default();
    assert!(csp.contains("'nonce-"), "the nonce policy stays: {csp}");
    assert!(
        csp.contains("form-action 'self'"),
        "the form policy stays: {csp}"
    );
    let visit = grace.visit("/connected-agents").await;
    assert_eq!(visit.status(), 200);
    check("an Inertia visit", &visit);
    let error = grace.open("/oauth/approve").await;
    assert_eq!(error.status(), 400);
    check("an error", &error);
    let claim = grace
        .open("/agent/claim?claim_attempt_token=cat_nope")
        .await;
    check("a refused claim", &claim);

    let dashboard = grace.open("/dashboard").await;
    assert_eq!(dashboard.status(), 200);
    assert!(
        dashboard
            .header("content-security-policy")
            .is_some_and(|csp| csp.contains("frame-ancestors 'self'")),
        "the dashboard keeps the preset's framing rule: {:?}",
        dashboard.header("content-security-policy")
    );
}

/// A same-origin `return_to` is followed, by a guest who signs in or
/// registers and by a user already signed in, once.
async fn agent_017_login_returns_to_a_same_origin_address() {
    verified_user("Heidi", "heidi@example.test").await;
    let target = "/agent/claim?claim_attempt_token=x";
    let login = format!("/login?return_to={}", encode(target));

    // A guest signs in and lands there.
    let mut heidi = Browser::new();
    let signed_in = heidi.sign_in_from(&login, "heidi@example.test").await;
    assert_eq!(path_of(&location_visit(&signed_in)), target);
    // Signed in, the same link goes straight there.
    let sent = heidi.open(&login).await;
    assert_eq!(redirect_path(&sent), target);
    // The address was spent: the next sign-in goes home.
    heidi.sign_out().await;
    heidi.sign_in("heidi@example.test").await;

    // A guest who registers instead also lands there.
    let mut ivan = Browser::new();
    assert_eq!(ivan.open(&login).await.status(), 200);
    assert_eq!(ivan.open("/register").await.status(), 200);
    let registered = ivan
        .submit(
            "/register",
            json!({
                "name": "Ivan",
                "email": "ivan@example.test",
                "password": PASSWORD,
                "password_confirmation": PASSWORD,
            }),
        )
        .await;
    assert_eq!(path_of(&location_visit(&registered)), target);
    ivan.sign_out().await;
    ivan.sign_in("ivan@example.test").await;
}

/// A `return_to` a browser would read as another site, or as no path of
/// this application, is never followed: by a guest who signs in, nor by a
/// signed-in user. Both go home.
async fn agent_017_login_ignores_a_foreign_return_to() {
    verified_user("Judy", "judy@example.test").await;
    for foreign in [
        "//evil.example/x",
        "/\\evil.example",
        "https://evil.example/x",
        "/\t/evil.example",
        " //evil.example",
    ] {
        let login = format!("/login?return_to={}", encode(foreign));
        let mut judy = Browser::new();
        let signed_in = judy.sign_in_from(&login, "judy@example.test").await;
        assert_eq!(
            redirect_path(&signed_in),
            "/dashboard",
            "guest, {foreign:?}"
        );
        let sent = judy.open(&login).await;
        assert_eq!(redirect_path(&sent), "/dashboard", "signed in, {foreign:?}");
        let visit = judy.visit(&login).await;
        assert_eq!(
            path_of(&location_visit(&visit)),
            "/dashboard",
            "signed in, {foreign:?}"
        );
    }
}

/// The kit's own migrator on SQLite: both plugins' tables, the index that
/// keeps an address unique whatever its case, and a case-only duplicate
/// refused.
async fn agent_017_kit_migrator_creates_the_plugins_tables_and_the_email_index_on_sqlite() {
    let db = suprnova::DB::connection().unwrap();
    let rows = db
        .inner()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT type, name FROM sqlite_master WHERE type IN ('table', 'index')".to_owned(),
        ))
        .await
        .expect("read the schema");
    let names: Vec<(String, String)> = rows
        .iter()
        .map(|row| {
            (
                row.try_get::<String>("", "type").expect("type"),
                row.try_get::<String>("", "name").expect("name"),
            )
        })
        .collect();
    let has = |kind: &str, name: &str| names.iter().any(|(k, n)| k == kind && n == name);
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
        assert!(has("table", table), "no table {table}: {names:?}");
    }
    let index =
        suprnova::auth::migrations::UniqueEmailIgnoringCase::on("users", "email").index_name();
    assert!(has("index", &index), "no index {index}: {names:?}");

    user("Ada", "ada@example.test").await;
    let duplicate = User::create("Ada", "Ada@Example.test", PASSWORD).await;
    assert!(
        duplicate.is_err(),
        "a case-only duplicate address must be refused"
    );
}

/// Registration folds the address, and refuses one that differs from a
/// registered address only by case.
async fn agent_008_registration_lowercases_and_refuses_a_mixed_case_duplicate() {
    let mut first = Browser::new();
    assert_eq!(first.open("/register").await.status(), 200);
    let registered = first
        .submit(
            "/register",
            json!({
                "name": "Mallory Mixed",
                "email": "  Mallory@Example.TEST ",
                "password": PASSWORD,
                "password_confirmation": PASSWORD,
            }),
        )
        .await;
    assert_eq!(redirect_path(&registered), "/verify-email");
    let stored = User::find_by_email("mallory@example.test")
        .await
        .expect("read the user")
        .expect("the user exists");
    assert_eq!(stored.email, "mallory@example.test");

    let mut second = Browser::new();
    assert_eq!(second.open("/register").await.status(), 200);
    let refused = second
        .submit(
            "/register",
            json!({
                "name": "Mallory Again",
                "email": "MALLORY@example.test",
                "password": PASSWORD,
                "password_confirmation": PASSWORD,
            }),
        )
        .await;
    assert_eq!(redirect_path(&refused), "/register");
    let page = second.visit("/register").await.inertia_page();
    assert_eq!(
        page["props"]["errors"]["email"], "This email is already registered.",
        "{page}"
    );
}

/// Registration takes a name of up to 255 characters and an address of up
/// to 254, and refuses one longer.
async fn kit_registration_refuses_a_name_over_255_and_an_email_over_254() {
    // 64 + 1 + 189 = 254 characters, every label within its own bound.
    let domain = format!(
        "{}.{}.{}.test",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(56)
    );
    let longest = format!("{}@{domain}", "l".repeat(64));
    assert_eq!(longest.len(), 254);
    let register = |name: String, email: String| {
        json!({
            "name": name,
            "email": email,
            "password": PASSWORD,
            "password_confirmation": PASSWORD,
        })
    };

    let mut too_long = Browser::new();
    assert_eq!(too_long.open("/register").await.status(), 200);
    let refused = too_long
        .submit("/register", register("n".repeat(256), longest.clone()))
        .await;
    assert_eq!(redirect_path(&refused), "/register");
    let page = too_long.visit("/register").await.inertia_page();
    assert_eq!(
        page["props"]["errors"]["name"], "Name must be 2 to 255 characters",
        "{page}"
    );
    let refused = too_long
        .submit(
            "/register",
            register("Long Address".to_owned(), format!("x{longest}")),
        )
        .await;
    assert_eq!(redirect_path(&refused), "/register");
    let page = too_long.visit("/register").await.inertia_page();
    assert!(page["props"]["errors"]["email"].is_string(), "{page}");
    assert!(
        User::find_by_email(&format!("x{longest}"))
            .await
            .expect("read the user")
            .is_none()
    );

    let mut longest_allowed = Browser::new();
    assert_eq!(longest_allowed.open("/register").await.status(), 200);
    let registered = longest_allowed
        .submit("/register", register("n".repeat(255), longest.clone()))
        .await;
    assert_eq!(redirect_path(&registered), "/verify-email");
}

macro_rules! cases {
    ($($case:ident),* $(,)?) => {
        $(
            $case().await;
            println!("case {} ... ok", stringify!($case));
        )*
    };
}

#[tokio::test]
async fn agent_pages() {
    let _mail = setup().await;
    cases!(
        agent_017_kit_migrator_creates_the_plugins_tables_and_the_email_index_on_sqlite,
        agent_017_inertia_serves_the_endpoint_and_both_plugins,
        agent_017_inertia_approval_round_trip,
        agent_017_inertia_claim_round_trip,
        agent_017_inertia_connected_agents_lists_and_revokes_only_the_users_own,
        agent_017_inertia_consent_forms_need_the_csrf_token,
        agent_017_inertia_consent_pages_refuse_framing_caching_and_referrers,
        agent_017_login_returns_to_a_same_origin_address,
        agent_017_login_ignores_a_foreign_return_to,
        agent_008_registration_lowercases_and_refuses_a_mixed_case_duplicate,
        kit_registration_refuses_a_name_over_255_and_an_email_over_254,
    );
}
