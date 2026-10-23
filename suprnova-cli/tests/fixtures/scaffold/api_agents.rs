//! AGENT-017 in a generated API application: the MCP endpoint, the OAuth
//! server and agent auth with the framework's default approval, claim and
//! connected-agents pages, the browser sign-in those pages send a guest to,
//! and connected agents over JSON.
//!
//! Written into a fresh `--api` scaffold by
//! `suprnova-cli/tests/scaffold_snapshot.rs`
//! (`agent_017_a_generated_api_application_serves_agents`), which replaces
//! `__PACKAGE__` with the generated crate's name and runs it with the
//! scaffold's own `.env` settings, `APP_URL=http://localhost` and a fresh
//! SQLite database, one case at a time. The cases share one boot, done as
//! the server does it: the migrator, then `bootstrap::register`, then the
//! routes through `agents::serve`. Each case drives the application through
//! `TestClient`, which keeps cookies as a browser does.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use base64::Engine as _;
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;
use sha2::{Digest, Sha256};
use suprnova::serde_json::{Value, json};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{Auth, Authenticatable, DB, HttpResponse, MiddlewareRegistry, Request, Response};

use __PACKAGE__::migrations::Migrator;
use __PACKAGE__::models::user::User;

/// Every account's password.
const PASSWORD: &str = "correct horse battery";

/// The MCP endpoint, the resource a client asks a token for.
const RESOURCE: &str = "http://localhost/mcp";

/// The loopback redirect URI an MCP client registers.
const REDIRECT: &str = "http://127.0.0.1:9/callback";

/// Where the kit's sign-in sends a user when no page is named.
const HOME: &str = "/oauth/connected";

/// What the sign-in page answers a wrong address or password with.
const INVALID: &str = "These credentials do not match our records.";

/// What the sign-in page answers an account with a second factor with.
const SECOND_FACTOR: &str =
    "This account signs in with a second factor, which this page does not offer.";

/// The runtime every case runs on, so the database pool the boot opens
/// outlives each case.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime")
    })
}

/// Run `case` after the boot, on the shared runtime.
fn run<F: Future<Output = ()>>(case: F) {
    runtime().block_on(async {
        boot().await;
        case.await;
    });
}

/// Boot the generated application once, as `serve` does without a
/// listener: the configuration, the migrator on its own connection, then
/// the bootstrap, which opens the database, installs Magnetar and
/// registers the HTTP stack.
async fn boot() {
    static BOOTED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    BOOTED
        .get_or_init(|| async {
            assert_eq!(
                std::env::var("APP_ENV").as_deref(),
                Ok("test"),
                "the harness runs this binary with APP_ENV=test and never loads .env"
            );
            __PACKAGE__::config::register_all();
            // Mail stays in memory, where a case reads the links it carries.
            mail();
            let url = std::env::var("DATABASE_URL").expect("the harness names the database");
            let db = Database::connect(&url)
                .await
                .expect("connect to the database");
            Migrator::up(&db, None).await.expect("the migrations apply");
            db.close().await.expect("close the migration connection");
            __PACKAGE__::bootstrap::register().await;
            // What the server boots before it listens: the cache, which the
            // MCP endpoint's request limit counts in, and the rate limiter.
            suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(std::sync::Arc::new(
                suprnova::InMemoryCache::new(),
            ));
            suprnova::rate_limit::bootstrap_default().await;
        })
        .await;
}

/// The mail the application sent, kept in memory for the whole run.
fn mail() -> &'static suprnova::mail::MailFake {
    static MAIL: OnceLock<suprnova::mail::MailFake> = OnceLock::new();
    MAIL.get_or_init(suprnova::Mail::fake)
}

/// How many messages went to `email`.
fn mails_to(email: &str) -> usize {
    mail()
        .captured()
        .iter()
        .filter(|message| message.to.iter().any(|to| to.email == email))
        .count()
}

/// The path and query of the newest verification link mailed to `email`.
fn mailed_link(email: &str) -> String {
    let marker = "/email/verify?token=";
    mail()
        .captured()
        .iter()
        .rev()
        .filter(|message| message.to.iter().any(|to| to.email == email))
        .filter_map(|message| message.text.as_deref())
        .find_map(|text| {
            let start = text.find(marker)?;
            let token: String = text[start + marker.len()..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            Some(format!("{marker}{token}"))
        })
        .unwrap_or_else(|| panic!("no verification link was mailed to {email}"))
}

/// An address no other client of this run has. Each client gets one, as
/// browsers on different machines would, so a limit counted per address
/// counts each client apart. The addresses are in 198.18.0.0/15, a block
/// set aside for tests that no proxy list trusts.
fn next_address() -> IpAddr {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let offset = NEXT.fetch_add(1, Ordering::Relaxed);
    IpAddr::V4(Ipv4Addr::from(
        u32::from(Ipv4Addr::new(198, 18, 0, 0)) + offset,
    ))
}

/// A browser, or any other client, with no cookies yet, at an address of
/// its own.
fn browser() -> TestClient {
    let router = __PACKAGE__::agents::serve(__PACKAGE__::routes::register())
        .expect("the routes with agents");
    // A password hash in a debug build takes a while.
    TestClient::new(router, MiddlewareRegistry::from_global())
        .timeout(Duration::from_secs(60))
        .with_peer_address(next_address())
}

/// A browser with two routes added for this test, each answering the id of
/// the user `Auth::user()` finds, or `null`: `GET /whoami`, which gets no
/// session, and `GET /oauth/connected/whoami`, under the connected-agents
/// page, which gets the consent pages' session.
fn browser_with_whoami() -> TestClient {
    async fn whoami(_request: Request) -> Response {
        let user = Auth::user().await.map_err(HttpResponse::from)?;
        Ok(HttpResponse::json(json!({
            "id": user.map(|user| user.get_auth_identifier()),
        })))
    }
    let routes: suprnova::Router = __PACKAGE__::routes::register()
        .get("/whoami", whoami)
        .get("/oauth/connected/whoami", whoami)
        .into();
    let router = __PACKAGE__::agents::serve(routes).expect("the routes with agents");
    TestClient::new(router, MiddlewareRegistry::from_global())
        .timeout(Duration::from_secs(60))
        .with_peer_address(next_address())
}

/// How many sessions the session store holds.
async fn session_rows() -> i64 {
    DB::connection()
        .expect("the database")
        .inner()
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) FROM sessions",
        ))
        .await
        .expect("count the sessions")
        .expect("a count")
        .try_get_by_index::<i64>(0)
        .expect("a number")
}

/// The id of the account `email` names.
async fn user_id(email: &str) -> String {
    User::find_by_email(email)
        .await
        .expect("read the account")
        .expect("the account")
        .id
        .to_string()
}

/// Register `email` through the JSON API, as a client of it does.
async fn register(email: &str) {
    let response = browser()
        .post("/api/auth/register")
        .header("Accept", "application/json")
        .json(&json!({ "email": email, "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status(), 202, "{}", response.body_text());
}

/// Verify `email` by following the newest link mailed to it, in a browser
/// of its own: as a guest, through the kit's sign-in and back. A claim
/// needs the address the agent named, verified.
async fn verify(email: &str) {
    let link = mailed_link(email);
    let visitor = browser();
    let to_sign_in = visitor
        .get(&link)
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
    let sign_in = path_of(&location(&to_sign_in));
    assert!(sign_in.starts_with("/login?"), "{sign_in}");
    let signed_in = sign_in_at(&visitor, &sign_in, email, PASSWORD).await;
    assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
    assert!(
        lands_on(&location(&signed_in), &link),
        "{}",
        location(&signed_in)
    );
    let page = visitor.get(&link).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    page.assert_see("Your email address is verified.");
}

/// The `Location` an answer sends the browser to.
fn location(response: &TestResponse) -> String {
    response
        .header("location")
        .unwrap_or_else(|| {
            panic!(
                "a redirect, got {} {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

/// The path and query of an absolute or host-relative address.
fn path_of(address: &str) -> String {
    let url = url::Url::parse("http://localhost")
        .expect("a base")
        .join(address)
        .expect("an address");
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    }
}

/// A query parameter of an address.
fn query_param(address: &str, name: &str) -> Option<String> {
    url::Url::parse("http://localhost")
        .expect("a base")
        .join(address)
        .expect("an address")
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// `value` encoded for a query.
fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The path of an address and its query's pairs, sorted: the request it
/// names, whatever order its query was written in.
fn request_of(address: &str) -> (String, Vec<(String, String)>) {
    let url = url::Url::parse("http://localhost")
        .expect("a base")
        .join(address)
        .expect("an address");
    let mut pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    pairs.sort();
    (url.path().to_owned(), pairs)
}

/// Whether `address` is the request `path` on this application: a path,
/// or an absolute URL on its origin.
fn lands_on(address: &str, path: &str) -> bool {
    let on_this_origin = address.starts_with("http://localhost/")
        || (address.starts_with('/') && !address.starts_with("//"));
    on_this_origin && request_of(address) == request_of(path)
}

/// The value of the field `name` a page's form carries.
fn field(page: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = page
        .find(&marker)
        .unwrap_or_else(|| panic!("a form with `{name}` in {page}"))
        + marker.len();
    let end = page[start..].find('"').expect("the value ends") + start;
    page[start..end].to_owned()
}

/// The CSRF token a page's form carries.
fn form_token(page: &str) -> String {
    field(page, "_token")
}

/// The path of the first form on a page whose `action` contains `part`.
fn form_action(page: &str, part: &str) -> String {
    let marker = "action=\"";
    page.match_indices(marker)
        .map(|(start, _)| {
            let value = &page[start + marker.len()..];
            &value[..value.find('"').expect("the action ends")]
        })
        .find(|action| action.contains(part))
        .map(path_of)
        .unwrap_or_else(|| panic!("a form posting to `{part}` in {page}"))
}

/// The attributes of a `Set-Cookie` line, without the cookie's value:
/// `Path=/`, `HttpOnly` and so on.
fn cookie_attributes(line: &str) -> Vec<String> {
    line.split(';')
        .skip(1)
        .map(|attribute| attribute.trim().to_owned())
        .collect()
}

/// A PKCE verifier and its S256 challenge.
fn pkce() -> (String, String) {
    let verifier = "a-pkce-verifier-of-more-than-forty-three-characters-0123456789".to_owned();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Register an MCP client dynamically (RFC 7591); its id.
async fn register_client(client: &TestClient) -> String {
    let response = client
        .post("/oauth/register")
        .json(&json!({
            "client_name": "Example Tool",
            "redirect_uris": [REDIRECT],
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        }))
        .send()
        .await;
    assert_eq!(response.status(), 201, "{}", response.body_text());
    response.json()["client_id"]
        .as_str()
        .expect("a client id")
        .to_owned()
}

/// The authorization request an MCP client sends the browser to.
fn authorize_path(client_id: &str, challenge: &str) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", RESOURCE)
        .append_pair("state", "state-123")
        .finish();
    format!("/oauth/authorize?{query}")
}

/// Register an agent under `service_auth` naming `email`: the registration
/// agent auth answers with, its claim included.
async fn register_agent(email: &str) -> Value {
    let response = browser()
        .post("/agent/identity")
        .json(&json!({ "type": "service_auth", "login_hint": email }))
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

/// Open the sign-in page at `path` and post its form as `email` with
/// `password`, as a browser posts a form with no `action`: to the page's
/// own address. The answer to the form.
async fn sign_in_at(client: &TestClient, path: &str, email: &str, password: &str) -> TestResponse {
    let page = client.get(path).header("Accept", "text/html").send().await;
    assert_eq!(page.status(), 200, "GET {path}: {}", page.body_text());
    client
        .post(path)
        .header("Accept", "text/html")
        .form(&[
            ("_token", form_token(&page.body_text()).as_str()),
            ("email", email),
            ("password", password),
        ])
        .send()
        .await
}

/// Sign `email` in through the JSON API: its bearer token.
async fn bearer_token(email: &str) -> String {
    let response = browser()
        .post("/api/auth/login")
        .header("Accept", "application/json")
        .json(&json!({ "email": email, "password": PASSWORD }))
        .send()
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()["token"]
        .as_str()
        .expect("a bearer token")
        .to_owned()
}

/// A client of the JSON API that holds `email`'s bearer token.
async fn api_client(email: &str) -> TestClient {
    browser()
        .with_header(
            "Authorization",
            format!("Bearer {}", bearer_token(email).await),
        )
        .with_header("Accept", "application/json")
}

/// An MCP client's authorization in `browser`, a guest's, start to finish:
/// the authorization endpoint sends the guest to the kit's sign-in, the
/// guest signs in as `email` and comes back, approves on the framework's
/// default page, and the client exchanges the code it is sent. The
/// client's id and its access token.
async fn approve_through_sign_in(visitor: &TestClient, email: &str) -> (String, String) {
    let client_id = register_client(visitor).await;
    let (verifier, challenge) = pkce();
    let authorize = authorize_path(&client_id, &challenge);
    let to_sign_in = visitor.get(&authorize).send().await;
    assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
    let sign_in = path_of(&location(&to_sign_in));
    assert!(sign_in.starts_with("/login?"), "{sign_in}");
    assert!(
        lands_on(
            &query_param(&sign_in, "return_to").expect("return_to"),
            &authorize
        ),
        "{sign_in}"
    );
    let signed_in = sign_in_at(visitor, &sign_in, email, PASSWORD).await;
    assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
    assert!(
        lands_on(&location(&signed_in), &authorize),
        "{}",
        location(&signed_in)
    );
    let to_approval = visitor.get(&authorize).send().await;
    assert_eq!(to_approval.status(), 302, "{}", to_approval.body_text());
    let approval = path_of(&location(&to_approval));
    assert!(
        approval.starts_with("/oauth/approve?authorization="),
        "{approval}"
    );
    let page = visitor.get(&approval).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let id = query_param(&approval, "authorization").expect("the authorization");
    let decided = visitor
        .post(format!("/oauth/authorizations/{id}"))
        .form(&[
            ("_token", form_token(&page.body_text()).as_str()),
            ("decision", "approve"),
        ])
        .send()
        .await;
    assert!(
        matches!(decided.status(), 302 | 303),
        "{} {}",
        decided.status(),
        decided.body_text()
    );
    let back = location(&decided);
    assert!(back.starts_with(&format!("{REDIRECT}?")), "{back}");
    assert_eq!(query_param(&back, "state").as_deref(), Some("state-123"));
    assert_eq!(
        query_param(&back, "iss").as_deref(),
        Some("http://localhost")
    );
    let code = query_param(&back, "code").expect("a code");
    let tokens = browser()
        .post("/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let access = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    (client_id, access)
}

/// An agent's claim in `browser`, a guest's, start to finish: the agent
/// registers naming `email`, the claim page sends the guest to the kit's
/// sign-in, the guest signs in and comes back, enters a wrong code and
/// then the right one, and the agent exchanges its claim token. The
/// registration and the agent's access token.
async fn claim_through_sign_in(visitor: &TestClient, email: &str) -> (Value, String) {
    let registration = register_agent(email).await;
    let claim = path_of(
        registration["claim"]["verification_uri"]
            .as_str()
            .expect("a verification URI"),
    );
    let to_sign_in = visitor.get(&claim).send().await;
    assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
    let sign_in = path_of(&location(&to_sign_in));
    assert!(
        lands_on(
            &query_param(&sign_in, "return_to").expect("return_to"),
            &claim
        ),
        "{sign_in}"
    );
    let signed_in = sign_in_at(visitor, &sign_in, email, PASSWORD).await;
    assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
    assert!(
        lands_on(&location(&signed_in), &claim),
        "{}",
        location(&signed_in)
    );
    let page = visitor.get(&claim).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let text = page.body_text();
    let attempt = field(&text, "claim_attempt_token");
    let code = registration["claim"]["user_code"]
        .as_str()
        .expect("a user code")
        .to_owned();
    let mut wrong_code = code.clone();
    let last = wrong_code.pop().expect("a code");
    wrong_code.push(if last == '0' { '1' } else { '0' });
    let wrong = visitor
        .post("/agent/claim")
        .form(&[
            ("_token", form_token(&text).as_str()),
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", wrong_code.as_str()),
        ])
        .send()
        .await;
    assert_eq!(wrong.status(), 400, "{}", wrong.body_text());
    wrong.assert_see("That code is wrong.");
    let done = visitor
        .post("/agent/claim")
        .form(&[
            ("_token", form_token(&wrong.body_text()).as_str()),
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", code.as_str()),
        ])
        .send()
        .await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    done.assert_see("The agent is connected");
    let tokens = browser()
        .post("/oauth/token")
        .form(&[
            ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
            (
                "claim_token",
                registration["claim_token"].as_str().expect("a claim token"),
            ),
            ("resource", RESOURCE),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let access = tokens.json()["access_token"]
        .as_str()
        .expect("an access token")
        .to_owned();
    (registration, access)
}

/// The status of the MCP endpoint's `tools/list` called with `token`.
async fn mcp_status(token: &str) -> u16 {
    browser().mcp().with_token(token).list().await.status()
}

/// The headers every answer of the sign-in page carries.
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
        .unwrap_or_default();
    assert!(
        policy.contains("frame-ancestors 'none'"),
        "{what}: {policy}"
    );
}

/// What a check found, kept so one run reports every finding.
#[derive(Default)]
struct Findings(Vec<String>);

impl Findings {
    /// Record `what` as a failure unless `holds`, and print the outcome.
    fn check(&mut self, holds: bool, what: &str) {
        println!("{} {what}", if holds { "holds:" } else { "FAILS:" });
        if !holds {
            self.0.push(what.to_owned());
        }
    }
}

/// The spike (ruling 1). A password sign-in through `Auth::password()` at
/// the kit's `/login` binds a framework session under Magnetar's sessions,
/// and the framework's default approval and claim pages accept it. A
/// client of the JSON API that signs in and calls it with its bearer token
/// gets no cookie, and the session starts only when a page needs one.
#[test]
fn agent_017_api_session_sign_in_answers_the_default_pages() {
    run(async {
        let email = "spike@example.test";
        register(email).await;
        verify(email).await;
        let mut findings = Findings::default();

        // The session starts lazily: a guest's JSON request stores none.
        let guest = browser();
        let metadata = guest
            .get("/.well-known/oauth-authorization-server")
            .header("Accept", "application/json")
            .send()
            .await;
        findings.check(
            metadata.status() == 200,
            "the authorization server metadata answers 200",
        );
        findings.check(
            metadata.header("set-cookie").is_none(),
            "a guest's JSON request gets no cookie",
        );

        // A bearer-only JSON client: its sign-in and its requests.
        let api = browser();
        let login = api
            .post("/api/auth/login")
            .header("Accept", "application/json")
            .json(&json!({ "email": email, "password": PASSWORD }))
            .send()
            .await;
        findings.check(login.status() == 200, "the JSON sign-in answers 200");
        findings.check(
            login.header("set-cookie").is_none(),
            "the JSON sign-in sets no cookie",
        );
        let token = suprnova::serde_json::from_str::<Value>(&login.body_text())
            .ok()
            .and_then(|body| body["token"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let bearer = browser().with_header("Authorization", format!("Bearer {token}"));
        let users = bearer
            .get("/api/users")
            .header("Accept", "application/json")
            .send()
            .await;
        findings.check(users.status() == 200, "a bearer JSON request is signed in");
        findings.check(
            users.header("set-cookie").is_none(),
            "a bearer JSON request gets no cookie",
        );
        let bare = bearer.get("/api/users").send().await;
        findings.check(
            bare.status() == 200,
            "a bearer request that names no Accept is signed in",
        );
        findings.check(
            bare.header("set-cookie").is_none(),
            "a bearer request that names no Accept gets no cookie",
        );

        // The approval page sends a guest to the kit's sign-in, and comes
        // back after it.
        let client = browser();
        let client_id = register_client(&client).await;
        let (_verifier, challenge) = pkce();
        let authorize = authorize_path(&client_id, &challenge);
        let to_sign_in = client.get(&authorize).send().await;
        assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
        let sign_in = path_of(&location(&to_sign_in));
        assert!(sign_in.starts_with("/login?return_to="), "{sign_in}");
        let page = client
            .get(&sign_in)
            .header("Accept", "text/html")
            .send()
            .await;
        findings.check(
            page.header("set-cookie").is_some(),
            "the sign-in page starts the session its form needs",
        );
        let signed_in = client
            .post(&sign_in)
            .header("Accept", "text/html")
            .form(&[
                ("_token", form_token(&page.body_text()).as_str()),
                ("email", email),
                ("password", PASSWORD),
            ])
            .send()
            .await;
        assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
        assert!(
            lands_on(&location(&signed_in), &authorize),
            "{}",
            location(&signed_in)
        );

        // The default approval page accepts the session.
        let to_approval = client.get(&authorize).send().await;
        assert_eq!(to_approval.status(), 302, "{}", to_approval.body_text());
        let approval = path_of(&location(&to_approval));
        assert!(
            approval.starts_with("/oauth/approve?authorization="),
            "the authorization endpoint sends the signed-in user to approve, not to sign in: {approval}"
        );
        let approval_page = client.get(&approval).send().await;
        assert_eq!(approval_page.status(), 200, "{}", approval_page.body_text());
        approval_page.assert_see("<bdi>Example Tool</bdi>");

        // The default claim page accepts it too.
        let registration = register_agent(email).await;
        let claim = path_of(
            registration["claim"]["verification_uri"]
                .as_str()
                .expect("a verification URI"),
        );
        let claim_page = client.get(&claim).send().await;
        assert_eq!(claim_page.status(), 200, "{}", claim_page.body_text());
        claim_page
            .assert_see("Let an agent act for you?")
            .assert_see("name=\"claim_attempt_token\"");

        assert!(findings.0.is_empty(), "the spike failed: {:#?}", findings.0);
    });
}

/// AGENT-017: the generated application serves the MCP endpoint and both
/// plugins: `initialize` and discovery on `/mcp`, the protected resource
/// and authorization server metadata with the `agent_auth` block, auth.md,
/// and an anonymous agent registration, whose identity assertion the
/// application signs with the key `suprnova new` wrote into `.env`.
#[test]
fn agent_017_api_serves_the_endpoint_and_both_plugins() {
    run(async {
        let client = browser();
        let init = client
            .mcp()
            .protocol_version("2025-11-25")
            .initialize()
            .await;
        assert_eq!(init.status(), 200, "{init:?}");
        let result = init.result().cloned().unwrap_or_default();
        assert_eq!(result["protocolVersion"], "2025-11-25");
        assert!(result["serverInfo"]["name"].is_string(), "{result}");
        let discovered = client.mcp().discover().await;
        assert_eq!(discovered.status(), 200, "{discovered:?}");
        let listed = client.mcp().list().await;
        assert_eq!(listed.status(), 200, "{listed:?}");

        let resource = client
            .get("/.well-known/oauth-protected-resource/mcp")
            .send()
            .await;
        assert_eq!(resource.status(), 200, "{}", resource.body_text());
        let resource = resource.json();
        assert_eq!(resource["resource"], RESOURCE);
        assert_eq!(
            resource["authorization_servers"],
            json!(["http://localhost"])
        );

        let metadata = client
            .get("/.well-known/oauth-authorization-server")
            .send()
            .await;
        assert_eq!(metadata.status(), 200, "{}", metadata.body_text());
        let metadata = metadata.json();
        assert_eq!(metadata["issuer"], "http://localhost");
        assert_eq!(
            metadata["authorization_endpoint"],
            "http://localhost/oauth/authorize"
        );
        assert_eq!(
            metadata["registration_endpoint"],
            "http://localhost/oauth/register"
        );
        assert_eq!(metadata["agent_auth"]["skill"], "http://localhost/auth.md");
        assert_eq!(
            metadata["agent_auth"]["identity_endpoint"],
            "http://localhost/agent/identity"
        );

        let skill = client.get("/auth.md").send().await;
        assert_eq!(skill.status(), 200, "{}", skill.body_text());
        assert_eq!(
            skill.header("content-type"),
            Some("text/markdown; charset=utf-8")
        );
        skill
            .assert_see("http://localhost/agent/identity")
            .assert_see("### anonymous")
            .assert_see("### service_auth");

        let anonymous = client
            .post("/agent/identity")
            .json(&json!({ "type": "anonymous" }))
            .send()
            .await;
        assert_eq!(anonymous.status(), 200, "{}", anonymous.body_text());
        let anonymous = anonymous.json();
        assert_eq!(anonymous["registration_type"], "anonymous");
        assert!(anonymous["claim_token"].is_string(), "{anonymous}");
        assert_eq!(
            anonymous["identity_assertion"]
                .as_str()
                .unwrap_or_default()
                .split('.')
                .count(),
            3,
            "a signed assertion: {anonymous}"
        );
    });
}

/// AGENT-017: the framework's default approval and claim pages complete
/// through the kit's `/login`: the approval reaches the client, whose
/// token then lists the endpoint's tools, and the claim reaches the agent,
/// whose token does too.
#[test]
fn agent_017_api_default_approval_and_claim_pages_complete() {
    run(async {
        let email = "complete@example.test";
        register(email).await;
        verify(email).await;

        let visitor = browser();
        let (_client_id, client_token) = approve_through_sign_in(&visitor, email).await;
        assert_eq!(mcp_status(&client_token).await, 200);

        let (_registration, agent_token) = claim_through_sign_in(&browser(), email).await;
        assert_eq!(mcp_status(&agent_token).await, 200);

        // The framework's connected-agents page lists both and revokes
        // each through its own form.
        let page = visitor.get(HOME).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        let text = page.body_text();
        assert!(text.contains("<bdi>Example Tool</bdi>"), "{text}");
        assert!(text.contains("Revoke this agent"), "{text}");
        let actions: Vec<String> = text
            .split("<form method=\"post\" action=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(path_of)
            .collect();
        assert_eq!(actions.len(), 2, "{text}");
        for (action, token) in actions.iter().zip([&client_token, &agent_token]) {
            assert!(action.starts_with(&format!("{HOME}/")), "{action}");
            let revoked = visitor
                .post(action)
                .form(&[("_token", form_token(&text).as_str())])
                .send()
                .await;
            assert_eq!(revoked.status(), 303, "{action}: {}", revoked.body_text());
            assert!(
                lands_on(&location(&revoked), HOME),
                "{}",
                location(&revoked)
            );
            assert_eq!(mcp_status(token).await, 401, "{action}: the token ends");
        }
        let emptied = visitor.get(HOME).send().await.body_text();
        assert!(emptied.contains("No apps can act for you."), "{emptied}");
        assert!(emptied.contains("No agents can act for you."), "{emptied}");
    });
}

/// AGENT-017: `return_to` never sends anyone off this application. Each
/// address a browser would read as another host goes to the kit's home,
/// for a guest who then signs in and for a user already signed in; a path
/// on this application is followed both times.
#[test]
fn agent_017_api_login_return_to_stays_on_this_origin() {
    run(async {
        let email = "return-to@example.test";
        register(email).await;
        for foreign in [
            "//evil.example/x",
            "/\\evil.example",
            "https://evil.example/x",
            "/\t/evil.example",
            " //evil.example",
        ] {
            let path = format!("/login?return_to={}", encoded(foreign));
            let guest = browser();
            let signed_in = sign_in_at(&guest, &path, email, PASSWORD).await;
            assert_eq!(
                signed_in.status(),
                302,
                "{foreign:?}: {}",
                signed_in.body_text()
            );
            let to = location(&signed_in);
            assert!(lands_on(&to, HOME), "{foreign:?}: a guest went to {to}");

            let again = guest.get(&path).send().await;
            assert_eq!(again.status(), 302, "{foreign:?}: {}", again.body_text());
            let to = location(&again);
            assert!(lands_on(&to, HOME), "{foreign:?}: a user went to {to}");
        }

        let own = "/agent/claim?claim_attempt_token=x";
        let path = format!("/login?return_to={}", encoded(own));
        let guest = browser();
        let signed_in = sign_in_at(&guest, &path, email, PASSWORD).await;
        assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
        assert!(
            lands_on(&location(&signed_in), own),
            "{}",
            location(&signed_in)
        );
        let again = guest.get(&path).send().await;
        assert_eq!(again.status(), 302, "{}", again.body_text());
        assert!(lands_on(&location(&again), own), "{}", location(&again));
    });
}

/// AGENT-017: a guest the approval page sends to sign in lands back on it,
/// and the address is spent: the next sign-in in the same session goes to
/// the kit's home. The API kit has no browser registration page; an
/// account registers through `POST /api/auth/register` and signs in here.
#[test]
fn agent_017_api_sign_in_continues_to_the_intended_address_once() {
    run(async {
        let email = "intended@example.test";
        register(email).await;
        let visitor = browser();
        let client_id = register_client(&visitor).await;
        let (_verifier, challenge) = pkce();
        let authorize = authorize_path(&client_id, &challenge);
        let sign_in = path_of(&location(&visitor.get(&authorize).send().await));
        let signed_in = sign_in_at(&visitor, &sign_in, email, PASSWORD).await;
        assert!(
            lands_on(&location(&signed_in), &authorize),
            "{}",
            location(&signed_in)
        );
        let approval = path_of(&location(&visitor.get(&authorize).send().await));
        assert!(
            approval.starts_with("/oauth/approve?authorization="),
            "{approval}"
        );
        let page = visitor.get(&approval).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());

        let second = visitor
            .post("/login")
            .header("Accept", "text/html")
            .form(&[
                ("_token", form_token(&page.body_text()).as_str()),
                ("email", email),
                ("password", PASSWORD),
            ])
            .send()
            .await;
        assert_eq!(second.status(), 302, "{}", second.body_text());
        assert!(
            lands_on(&location(&second), HOME),
            "the intended address was spent: {}",
            location(&second)
        );
    });
}

/// AGENT-017: the kit's own migrator, on a fresh SQLite database, creates
/// the users table with its unique email index, the sessions table and
/// both plugins' tables, and runs again without change. An address that
/// differs only in case from one with an account adds no second account.
#[test]
fn agent_017_api_migrator_creates_the_plugins_tables_on_sqlite() {
    run(async {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("a fresh database");
        Migrator::up(&db, None).await.expect("the migrations apply");
        Migrator::up(&db, None)
            .await
            .expect("the migrations apply again");
        let tables: Vec<String> = db
            .query_all_raw(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT name FROM sqlite_master WHERE type = 'table'",
            ))
            .await
            .expect("list the tables")
            .iter()
            .map(|row| row.try_get_by_index::<String>(0).expect("a name"))
            .collect();
        for table in [
            "app_users",
            "sessions",
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
                tables.iter().any(|name| name == table),
                "{table} in {tables:?}"
            );
        }
        let unique_email = db
            .query_all_raw(sea_orm::Statement::from_string(
                db.get_database_backend(),
                "SELECT il.name AS name FROM pragma_index_list('app_users') AS il \
                 JOIN pragma_index_info(il.name) AS ii \
                 WHERE il.\"unique\" = 1 AND ii.name = 'email'",
            ))
            .await
            .expect("read the users table's indexes");
        assert_eq!(unique_email.len(), 1, "one unique index on app_users.email");
        db.close().await.expect("close");

        // The booted application's database, migrated the same way.
        register("ada@example.test").await;
        register("Ada@Example.test").await;
        let accounts = DB::connection()
            .expect("the database")
            .inner()
            .query_all_raw(sea_orm::Statement::from_string(
                sea_orm::DbBackend::Sqlite,
                "SELECT email FROM app_users WHERE LOWER(email) = 'ada@example.test'",
            ))
            .await
            .expect("read the accounts");
        assert_eq!(accounts.len(), 1, "a case-only duplicate adds no account");
    });
}

/// AGENT-017: an account that signs in with a second factor is told this
/// page does not offer one, and stays signed out: no partial sign-in.
#[test]
fn agent_017_api_sign_in_refuses_an_account_with_a_second_factor() {
    run(async {
        let email = "second-factor@example.test";
        register(email).await;
        let user = User::find_by_email(email)
            .await
            .expect("read the account")
            .expect("the account");
        DB::connection()
            .expect("the database")
            .inner()
            .execute_unprepared(&format!(
                "INSERT INTO auth_two_factor \
                 (user_id, secret, enrollment_auth_epoch, rotation_pending, confirmed_at) \
                 VALUES ('{}', X'00', 0, 0, CURRENT_TIMESTAMP)",
                user.id
            ))
            .await
            .expect("give the account a second factor");

        let visitor = browser();
        let refused = sign_in_at(&visitor, "/login", email, PASSWORD).await;
        assert_eq!(refused.status(), 422, "{}", refused.body_text());
        refused.assert_see(SECOND_FACTOR);
        let connected = visitor.get(HOME).send().await;
        assert_eq!(
            connected.status(),
            302,
            "still a guest: {}",
            connected.body_text()
        );
        assert!(path_of(&location(&connected)).starts_with("/login?"));
    });
}

/// AGENT-017: a wrong password, an address with no account and a form
/// that is not valid all get one answer, and none signs anyone in.
#[test]
fn agent_017_api_sign_in_refuses_wrong_credentials_with_one_message() {
    run(async {
        let email = "wrong@example.test";
        register(email).await;
        for (address, password) in [
            (email, "not the password"),
            ("nobody@example.test", PASSWORD),
            ("not an address", PASSWORD),
            (email, ""),
        ] {
            let visitor = browser();
            let refused = sign_in_at(&visitor, "/login", address, password).await;
            assert_eq!(refused.status(), 422, "{address}: {}", refused.body_text());
            refused.assert_see(INVALID);
            assert_consent_headers(&refused, "a refused sign-in");
            let connected = visitor.get(HOME).send().await;
            assert_eq!(connected.status(), 302, "{address}: still a guest");
        }
    });
}

/// AGENT-017: the sign-in page refuses framing, caching and referrers on
/// every answer, under the kit's default security headers, and its form
/// needs the session's CSRF token.
#[test]
fn agent_017_api_sign_in_page_refuses_framing_and_needs_the_csrf_token() {
    run(async {
        let email = "csrf@example.test";
        register(email).await;
        let visitor = browser();
        let page = visitor
            .get("/login")
            .header("Accept", "text/html")
            .send()
            .await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        assert_consent_headers(&page, "the sign-in page");
        assert!(
            page.header("content-type")
                .is_some_and(|value| value.starts_with("text/html")),
            "{:?}",
            page.header("content-type")
        );

        for token in [None, Some("not-the-token")] {
            let mut form = vec![("email", email), ("password", PASSWORD)];
            if let Some(token) = token {
                form.push(("_token", token));
            }
            let refused = visitor
                .post("/login")
                .header("Accept", "text/html")
                .form(&form)
                .send()
                .await;
            assert_eq!(refused.status(), 419, "{token:?}: {}", refused.body_text());
            assert_consent_headers(&refused, "a refused form");
        }
        let connected = visitor.get(HOME).send().await;
        assert_eq!(connected.status(), 302, "still a guest");
    });
}

/// AGENT-017: a client of the JSON API lists the MCP clients and agents
/// that act for its user, newest first, and revokes either with `DELETE`:
/// `204`, then `404` with a message once it is gone. Another user's ids are
/// not found, and a revoked client's or agent's token stops working.
#[test]
fn agent_017_api_lists_and_revokes_client_grants_and_agent_accounts() {
    run(async {
        let email = "owner@example.test";
        register(email).await;
        verify(email).await;
        let (client_id, client_token) = approve_through_sign_in(&browser(), email).await;
        let (registration, agent_token) = claim_through_sign_in(&browser(), email).await;

        let owner = api_client(email).await;
        let listed = owner.get("/api/connected-agents").send().await;
        assert_eq!(listed.status(), 200, "{}", listed.body_text());
        let listed = listed.json();
        let clients = listed["clients"].as_array().expect("clients");
        assert_eq!(clients.len(), 1, "{listed}");
        assert_eq!(clients[0]["client_id"], client_id.as_str());
        assert_eq!(clients[0]["client_name"], "Example Tool");
        let grant = clients[0]["id"].as_str().expect("a grant id").to_owned();
        let agents = listed["agents"].as_array().expect("agents");
        assert_eq!(agents.len(), 1, "{listed}");
        assert_eq!(agents[0]["registration_type"], "service_auth");
        let agent = agents[0]["id"]
            .as_str()
            .expect("a registration id")
            .to_owned();
        assert_eq!(
            registration["registration_type"], "service_auth",
            "{registration}"
        );

        // Another user sees none of it and revokes none of it.
        let stranger = "stranger@example.test";
        register(stranger).await;
        let stranger = api_client(stranger).await;
        let theirs = stranger.get("/api/connected-agents").send().await;
        assert_eq!(theirs.status(), 200, "{}", theirs.body_text());
        assert_eq!(theirs.json(), json!({ "clients": [], "agents": [] }));
        for (path, message) in [
            (
                format!("/api/connected-agents/clients/{grant}"),
                "No such connected app.",
            ),
            (
                format!("/api/connected-agents/agents/{agent}"),
                "No such connected agent.",
            ),
        ] {
            let refused = stranger.delete(&path).send().await;
            assert_eq!(refused.status(), 404, "{path}: {}", refused.body_text());
            assert_eq!(refused.json(), json!({ "message": message }));
        }
        assert_eq!(mcp_status(&client_token).await, 200);
        assert_eq!(mcp_status(&agent_token).await, 200);

        // The owner revokes both.
        for (path, message, token) in [
            (
                format!("/api/connected-agents/clients/{grant}"),
                "No such connected app.",
                &client_token,
            ),
            (
                format!("/api/connected-agents/agents/{agent}"),
                "No such connected agent.",
                &agent_token,
            ),
        ] {
            let revoked = owner.delete(&path).send().await;
            assert_eq!(revoked.status(), 204, "{path}: {}", revoked.body_text());
            assert_eq!(mcp_status(token).await, 401, "{path}: the token ends");
            let again = owner.delete(&path).send().await;
            assert_eq!(again.status(), 404, "{path}: {}", again.body_text());
            assert_eq!(again.json(), json!({ "message": message }));
        }
        let emptied = owner.get("/api/connected-agents").send().await;
        assert_eq!(emptied.json(), json!({ "clients": [], "agents": [] }));
    });
}

/// AGENT-017: the JSON routes act for a signed-in user only: a guest gets
/// `401`, and so does an MCP access token, which is not a sign-in to this
/// API.
#[test]
fn agent_017_api_connected_agents_refuse_a_guest_and_an_mcp_token() {
    run(async {
        let email = "mcp-token@example.test";
        register(email).await;
        let (_client_id, client_token) = approve_through_sign_in(&browser(), email).await;
        let guest = browser().with_header("Accept", "application/json");
        let with_mcp_token = browser()
            .with_header("Accept", "application/json")
            .with_header("Authorization", format!("Bearer {client_token}"));
        for client in [&guest, &with_mcp_token] {
            let listed = client.get("/api/connected-agents").send().await;
            assert_eq!(listed.status(), 401, "{}", listed.body_text());
            let revoked = client
                .delete("/api/connected-agents/clients/any")
                .send()
                .await;
            assert_eq!(revoked.status(), 401, "{}", revoked.body_text());
        }
        assert_eq!(
            mcp_status(&client_token).await,
            200,
            "the token still works"
        );
    });
}

/// AGENT-017: revoking takes `DELETE` only. A plain form can send `POST`
/// from another site, and the JSON API runs no CSRF check.
#[test]
fn agent_017_api_revoke_needs_delete() {
    run(async {
        let email = "delete-only@example.test";
        register(email).await;
        let owner = api_client(email).await;
        for path in [
            "/api/connected-agents/clients/any",
            "/api/connected-agents/agents/any",
        ] {
            for method in ["POST", "GET"] {
                let response = owner
                    .send(method.parse().expect("a method"), path)
                    .send()
                    .await;
                assert_eq!(
                    response.status(),
                    405,
                    "{method} {path}: {}",
                    response.body_text()
                );
            }
        }
    });
}

/// Item 3: registration stores the address with its ASCII letters
/// lowercased and its surrounding space trimmed, and signing in finds it
/// whatever its case.
#[test]
fn agent_008_api_registration_stores_the_address_lowercased() {
    run(async {
        register("  Grace@Example.TEST ").await;
        let user = User::find_by_email("grace@example.test")
            .await
            .expect("read the account")
            .expect("the account, stored in lowercase");
        assert_eq!(user.email, "grace@example.test");
        let signed_in = browser()
            .post("/api/auth/login")
            .header("Accept", "application/json")
            .json(&json!({ "email": "GRACE@example.test", "password": PASSWORD }))
            .send()
            .await;
        assert_eq!(signed_in.status(), 200, "{}", signed_in.body_text());
    });
}

/// Item 4: registration refuses an address over 254 characters, longer
/// than any deliverable one, naming the field, and takes one of 254.
#[test]
fn kit_api_registration_refuses_an_email_over_254() {
    run(async {
        let longest = format!(
            "{}@{}.{}.{}.test",
            "a".repeat(64),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(56)
        );
        assert_eq!(longest.chars().count(), 254);
        let too_long = longest.replace(".test", "d.test");
        assert_eq!(too_long.chars().count(), 255);
        let refused = browser()
            .post("/api/auth/register")
            .header("Accept", "application/json")
            .json(&json!({ "email": too_long, "password": PASSWORD }))
            .send()
            .await;
        assert_eq!(refused.status(), 422, "{}", refused.body_text());
        assert!(
            refused.json()["errors"]["email"].is_array(),
            "{}",
            refused.body_text()
        );
        register(&longest).await;
    });
}

/// Item 4: registration counts a password's characters, as the other kits
/// do: seven characters are refused even when their bytes number eight or
/// more.
#[test]
fn kit_api_registration_refuses_a_password_under_8_characters() {
    run(async {
        for password in ["seven77", "ééééééé"] {
            let refused = browser()
                .post("/api/auth/register")
                .header("Accept", "application/json")
                .json(&json!({ "email": "short@example.test", "password": password }))
                .send()
                .await;
            assert_eq!(refused.status(), 422, "{password}: {}", refused.body_text());
            assert!(
                refused.json()["errors"]["password"].is_array(),
                "{}",
                refused.body_text()
            );
        }
    });
}

/// AGENT-017: only the consent pages start a session. The JSON sign-in, a
/// bearer request with and without `Accept`, the MCP endpoint, the token
/// and registration endpoints, the discovery documents and agent auth's
/// JSON routes set no cookie and store no session; the sign-in page does
/// both.
#[test]
fn agent_017_api_only_the_consent_pages_start_a_session() {
    run(async {
        let email = "no-session@example.test";
        register(email).await;
        let (client_id, client_token) = approve_through_sign_in(&browser(), email).await;

        let before = session_rows().await;
        let client = browser();
        let token = bearer_token(email).await;
        let bearer = format!("Bearer {token}");
        let mut answers: Vec<(&str, TestResponse)> = Vec::new();
        answers.push((
            "the JSON sign-in",
            client
                .post("/api/auth/login")
                .header("Accept", "application/json")
                .json(&json!({ "email": email, "password": PASSWORD }))
                .send()
                .await,
        ));
        answers.push((
            "a bearer request asking for JSON",
            client
                .get("/api/users")
                .header("Authorization", bearer.as_str())
                .header("Accept", "application/json")
                .send()
                .await,
        ));
        answers.push((
            "a bearer request naming no Accept",
            client
                .get("/api/users")
                .header("Authorization", bearer.as_str())
                .send()
                .await,
        ));
        answers.push((
            "the JSON registration",
            client
                .post("/api/auth/register")
                .json(&json!({ "email": "another@example.test", "password": PASSWORD }))
                .send()
                .await,
        ));
        answers.push((
            "the authorization server metadata",
            client
                .get("/.well-known/oauth-authorization-server")
                .send()
                .await,
        ));
        answers.push((
            "the protected resource metadata",
            client
                .get("/.well-known/oauth-protected-resource/mcp")
                .send()
                .await,
        ));
        answers.push(("auth.md", client.get("/auth.md").send().await));
        answers.push((
            "client registration",
            client
                .post("/oauth/register")
                .json(&json!({
                    "client_name": "Another Tool",
                    "redirect_uris": [REDIRECT],
                    "token_endpoint_auth_method": "none",
                    "grant_types": ["authorization_code"],
                    "response_types": ["code"],
                }))
                .send()
                .await,
        ));
        answers.push((
            "the token endpoint",
            client
                .post("/oauth/token")
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", "not-a-token"),
                    ("client_id", client_id.as_str()),
                    ("resource", RESOURCE),
                ])
                .send()
                .await,
        ));
        answers.push((
            "an agent registration",
            client
                .post("/agent/identity")
                .json(&json!({ "type": "anonymous" }))
                .send()
                .await,
        ));
        let listed = client.mcp().with_token(&client_token).list().await;
        assert_eq!(listed.status(), 200, "{listed:?}");
        assert!(
            listed.header("set-cookie").is_none(),
            "the MCP endpoint set a cookie"
        );
        for (what, answer) in &answers {
            assert!(
                answer.status() < 500,
                "{what}: {} {}",
                answer.status(),
                answer.body_text()
            );
            assert!(
                answer.header("set-cookie").is_none(),
                "{what} set a cookie: {:?}",
                answer.header("set-cookie")
            );
        }
        assert_eq!(session_rows().await, before, "no request stored a session");

        let page = browser()
            .get("/login")
            .header("Accept", "text/html")
            .send()
            .await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        assert!(
            page.header("set-cookie").is_some(),
            "the sign-in page starts one"
        );
        assert_eq!(session_rows().await, before + 1, "and stores it");
    });
}

/// AGENT-017: `Auth::user()` finds the signed-in user, for a caller with a
/// bearer token anywhere and for a session on the consent pages. Off the
/// consent pages a browser's session is not read, so it is a guest there.
#[test]
fn agent_017_api_auth_user_reads_the_session_and_the_bearer_caller() {
    run(async {
        let email = "whoami@example.test";
        register(email).await;
        let id = user_id(email).await;

        let guest = browser_with_whoami();
        let answer = guest.get("/whoami").send().await;
        assert_eq!(answer.status(), 200, "{}", answer.body_text());
        assert_eq!(answer.json(), json!({ "id": null }));

        let token = bearer_token(email).await;
        let bearer = browser_with_whoami().with_header("Authorization", format!("Bearer {token}"));
        for path in ["/whoami", "/oauth/connected/whoami"] {
            let answer = bearer.get(path).send().await;
            assert_eq!(answer.status(), 200, "{path}: {}", answer.body_text());
            assert_eq!(answer.json(), json!({ "id": id }), "{path}");
        }

        let visitor = browser_with_whoami();
        let signed_in = sign_in_at(&visitor, "/login", email, PASSWORD).await;
        assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
        let answer = visitor.get("/oauth/connected/whoami").send().await;
        assert_eq!(answer.status(), 200, "{}", answer.body_text());
        assert_eq!(answer.json(), json!({ "id": id }), "the session's user");
        let answer = visitor.get("/whoami").send().await;
        assert_eq!(
            answer.json(),
            json!({ "id": null }),
            "no session off the consent pages"
        );
    });
}

/// The JSON API answers as it did before agents: registration `202`, sign-in
/// a token or `401`, and the user directory to a bearer token only, `401`
/// to a guest, `404` for an id with no user.
#[test]
fn agent_017_api_existing_json_routes_behave_as_before() {
    run(async {
        let email = "directory@example.test";
        register(email).await;
        let id = user_id(email).await;

        let wrong = browser()
            .post("/api/auth/login")
            .header("Accept", "application/json")
            .json(&json!({ "email": email, "password": "not the password" }))
            .send()
            .await;
        assert_eq!(wrong.status(), 401, "{}", wrong.body_text());

        let guest = browser().with_header("Accept", "application/json");
        for path in ["/api/users".to_owned(), format!("/api/users/{id}")] {
            let refused = guest.get(&path).send().await;
            assert_eq!(refused.status(), 401, "{path}: {}", refused.body_text());
        }

        let client = api_client(email).await;
        let listed = client.get("/api/users").send().await;
        assert_eq!(listed.status(), 200, "{}", listed.body_text());
        assert!(listed.body_text().contains(email), "{}", listed.body_text());
        let shown = client.get(format!("/api/users/{id}")).send().await;
        assert_eq!(shown.status(), 200, "{}", shown.body_text());
        assert!(shown.body_text().contains(email), "{}", shown.body_text());
        let missing = client.get("/api/users/999999").send().await;
        assert_eq!(missing.status(), 404, "{}", missing.body_text());
    });
}

/// AGENT-017: registration mails a new account the link that verifies its
/// address, once. Registering the address again mails nothing, so neither
/// the answer nor the mail tells which addresses have accounts.
#[test]
fn agent_017_api_registration_mails_a_verification_link() {
    run(async {
        let email = "mailed@example.test";
        register(email).await;
        assert_eq!(mails_to(email), 1, "one verification mail");
        assert!(mailed_link(email).starts_with("/email/verify?token="));
        register(email).await;
        assert_eq!(mails_to(email), 1, "an existing account is mailed nothing");
    });
}

/// AGENT-017: the link verifies the address of the user signed in where it
/// opens. Another account's session is refused and the link stays usable;
/// a guest signs in at the kit's `/login` and comes back; a second use is
/// refused. The resend route mails a new link until the address is
/// verified, and then answers `204`.
#[test]
fn agent_017_api_verification_link_verifies_the_address_once() {
    run(async {
        let email = "link@example.test";
        register(email).await;
        let client = api_client(email).await;
        let resent = client
            .post("/api/email/verification-notification")
            .send()
            .await;
        assert_eq!(resent.status(), 202, "{}", resent.body_text());
        assert_eq!(mails_to(email), 2, "the resend mails a second link");
        let link = mailed_link(email);

        let other = "link-other@example.test";
        register(other).await;
        let stranger = browser();
        let signed_in = sign_in_at(&stranger, "/login", other, PASSWORD).await;
        assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
        let refused = stranger.get(&link).send().await;
        assert_eq!(refused.status(), 403, "{}", refused.body_text());
        refused.assert_see("This link is for another account.");
        assert_consent_headers(&refused, "a refused link");

        let visitor = browser();
        let to_sign_in = visitor.get(&link).send().await;
        assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
        assert_consent_headers(&to_sign_in, "the link's redirect to sign in");
        let sign_in = path_of(&location(&to_sign_in));
        assert!(
            lands_on(
                &query_param(&sign_in, "return_to").expect("return_to"),
                &link
            ),
            "{sign_in}"
        );
        let signed_in = sign_in_at(&visitor, &sign_in, email, PASSWORD).await;
        assert!(
            lands_on(&location(&signed_in), &link),
            "{}",
            location(&signed_in)
        );
        let page = visitor.get(&link).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        page.assert_see("Your email address is verified.");
        assert_consent_headers(&page, "the verification page");
        let user = User::find_by_email(email)
            .await
            .expect("read the account")
            .expect("the account");
        assert!(user.email_verified_at.is_some(), "the address is verified");

        let again = visitor.get(&link).send().await;
        assert_eq!(again.status(), 404, "{}", again.body_text());
        again.assert_see("This link is not valid, or it was already used.");

        let done = client
            .post("/api/email/verification-notification")
            .send()
            .await;
        assert_eq!(done.status(), 204, "{}", done.body_text());
        assert_eq!(mails_to(email), 2, "a verified address is mailed nothing");
    });
}

/// AGENT-017: an agent's claim for an address its user has not verified
/// answers the facade's refusal, and goes through once the user follows
/// the link mailed at registration.
#[test]
fn agent_017_api_claim_needs_a_verified_address() {
    run(async {
        let email = "unverified@example.test";
        register(email).await;
        let registration = register_agent(email).await;
        let claim = path_of(
            registration["claim"]["verification_uri"]
                .as_str()
                .expect("a verification URI"),
        );
        let visitor = browser();
        let to_sign_in = visitor.get(&claim).send().await;
        assert_eq!(to_sign_in.status(), 302, "{}", to_sign_in.body_text());
        let signed_in =
            sign_in_at(&visitor, &path_of(&location(&to_sign_in)), email, PASSWORD).await;
        assert!(
            lands_on(&location(&signed_in), &claim),
            "{}",
            location(&signed_in)
        );
        let refused = visitor.get(&claim).send().await;
        assert_eq!(refused.status(), 403, "{}", refused.body_text());
        refused.assert_see("This claim is for another account");

        verify(email).await;
        let page = visitor.get(&claim).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        page.assert_see("Let an agent act for you?");
    });
}

/// AGENT-017: the consent session's cookie carries no `Max-Age` and no
/// `Expires`, so the browser drops it when it closes.
#[test]
fn agent_017_api_consent_session_cookie_ends_with_the_browser() {
    run(async {
        let name = suprnova::SessionConfig::from_env().cookie_name;
        let page = browser()
            .get("/login")
            .header("Accept", "text/html")
            .send()
            .await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        assert!(page.cookie(&name).is_some(), "the page starts a session");
        let line = page.header("set-cookie").unwrap_or_default();
        assert!(
            line.starts_with(&format!("{name}=")),
            "the first cookie is the session's: {:?}",
            line.split('=').next()
        );
        let attributes = cookie_attributes(line);
        assert!(
            attributes.iter().all(|attribute| {
                let attribute = attribute.to_ascii_lowercase();
                !attribute.starts_with("max-age") && !attribute.starts_with("expires")
            }),
            "{attributes:?}"
        );
    });
}

/// AGENT-017: a consent session ends after fifteen minutes without a
/// request. Requests keep it alive, so it is the idle time that counts, not
/// the time since the sign-in.
#[test]
fn agent_017_api_consent_session_ends_after_fifteen_idle_minutes() {
    run(async {
        let email = "idle@example.test";
        register(email).await;
        suprnova::testing::TestClock::scope(chrono::Utc::now(), |clock| async move {
            let visitor = browser();
            let signed_in = sign_in_at(&visitor, "/login", email, PASSWORD).await;
            assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
            for (minutes, what) in [
                (0, "just signed in"),
                (10, "after ten idle minutes"),
                (10, "ten more, twenty since the sign-in"),
            ] {
                clock.advance(chrono::Duration::minutes(minutes));
                let page = visitor.get(HOME).send().await;
                assert_eq!(page.status(), 200, "{what}: {}", page.body_text());
            }
            clock.advance(chrono::Duration::minutes(16));
            let idle = visitor.get(HOME).send().await;
            assert_eq!(idle.status(), 302, "{}", idle.body_text());
            let sign_in = path_of(&location(&idle));
            assert!(sign_in.starts_with("/login?"), "{sign_in}");
        })
        .await;
    });
}

/// AGENT-017: a signed-in user can end the consent session. The
/// verification page, which a signed-in user sees, carries the form; it
/// posts to `/logout` with the session's CSRF token, and the next
/// authorization asks for a sign-in again.
#[test]
fn agent_017_api_sign_out_ends_the_consent_session() {
    run(async {
        let email = "sign-out@example.test";
        register(email).await;
        let visitor = browser();
        let client_id = register_client(&visitor).await;
        let (_, challenge) = pkce();
        let authorize = authorize_path(&client_id, &challenge);
        let signed_in = sign_in_at(&visitor, "/login", email, PASSWORD).await;
        assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
        let to_approval = visitor.get(&authorize).send().await;
        assert_eq!(to_approval.status(), 302, "{}", to_approval.body_text());
        let approval = path_of(&location(&to_approval));
        assert!(approval.starts_with("/oauth/approve?"), "{approval}");

        let page = visitor
            .get("/email/verify?token=00")
            .header("Accept", "text/html")
            .send()
            .await;
        assert_eq!(page.status(), 404, "{}", page.body_text());
        let text = page.body_text();
        assert_eq!(form_action(&text, "/logout"), "/logout");
        let signed_out = visitor
            .post("/logout")
            .header("Accept", "text/html")
            .form(&[("_token", form_token(&text).as_str())])
            .send()
            .await;
        assert!(
            matches!(signed_out.status(), 302 | 303),
            "{} {}",
            signed_out.status(),
            signed_out.body_text()
        );
        assert_eq!(path_of(&location(&signed_out)), "/login");
        assert_consent_headers(&signed_out, "the sign-out");

        let again = visitor.get(&authorize).send().await;
        assert_eq!(again.status(), 302, "{}", again.body_text());
        let sign_in = path_of(&location(&again));
        assert!(sign_in.starts_with("/login?"), "{sign_in}");
        let guest_page = visitor
            .get("/email/verify?token=not-hex")
            .header("Accept", "text/html")
            .send()
            .await;
        assert_eq!(guest_page.status(), 404, "{}", guest_page.body_text());
        assert!(
            !guest_page.body_text().contains("/logout"),
            "a guest is offered no sign-out: {}",
            guest_page.body_text()
        );
    });
}

/// AGENT-017: one address registers at most six accounts a minute. The
/// seventh request gets `429` and makes no account; another address is not
/// held back.
#[test]
fn agent_017_api_registration_is_limited_per_address() {
    run(async {
        let client = browser();
        let register_as = |email: &str, password: &str| {
            client
                .post("/api/auth/register")
                .header("Accept", "application/json")
                .json(&json!({ "email": email, "password": password }))
                .send()
        };
        let first = register_as("limited-0@example.test", PASSWORD).await;
        assert_eq!(first.status(), 202, "{}", first.body_text());
        // Refused forms count too, and cost no password hash.
        for attempt in 1..6 {
            let refused = register_as(&format!("limited-{attempt}@example.test"), "short").await;
            assert_eq!(
                refused.status(),
                422,
                "attempt {attempt}: {}",
                refused.body_text()
            );
        }
        let seventh = register_as("limited-6@example.test", PASSWORD).await;
        assert_eq!(seventh.status(), 429, "{}", seventh.body_text());
        assert!(
            User::find_by_email("limited-6@example.test")
                .await
                .expect("read the account")
                .is_none(),
            "the seventh request made no account"
        );
        register("limited-7@example.test").await;
    });
}

/// AGENT-017: the session's allow-list matches whole paths. A path that
/// only looks like a consent page gets no session, and its answer is the
/// one the routes give with no middleware at all.
#[test]
fn agent_017_api_session_allow_list_matches_whole_paths() {
    run(async {
        use __PACKAGE__::agents::ConsentSession;

        let lookalikes = [
            "/login/",
            "/LOGIN",
            "/oauthx/authorize",
            "/oauth/connectedx",
            "/oauth/authorizations",
        ];
        for path in lookalikes {
            assert!(!ConsentSession::covers(path), "{path}");
        }
        for path in [
            "/login",
            "/logout",
            "/email/verify",
            "/oauth/approve",
            "/oauth/connected",
            "/oauth/connected/clients/1/revoke",
            "/agent/claim",
            "/oauth/authorize",
            "/oauth/authorizations/1",
        ] {
            assert!(ConsentSession::covers(path), "{path}");
        }

        let bare = TestClient::new(
            __PACKAGE__::agents::serve(__PACKAGE__::routes::register())
                .expect("the routes with agents"),
            MiddlewareRegistry::new(),
        )
        .timeout(Duration::from_secs(60));
        let before = session_rows().await;
        for path in lookalikes {
            for post in [false, true] {
                let ask = |client: &TestClient| {
                    let request = if post {
                        client.post(path).form(&[("decision", "approve")])
                    } else {
                        client.get(path)
                    };
                    request.header("Accept", "application/json").send()
                };
                let served = ask(&browser()).await;
                let plain = ask(&bare).await;
                let method = if post { "POST" } else { "GET" };
                assert!(
                    served.header("set-cookie").is_none(),
                    "{method} {path} set a cookie"
                );
                assert_eq!(
                    (served.status(), served.body_text()),
                    (plain.status(), plain.body_text()),
                    "{method} {path}"
                );
            }
        }
        assert_eq!(session_rows().await, before, "no request stored a session");
    });
}

/// AGENT-017: the approval, claim, revoke and sign-out forms are refused
/// with `419` when posted without the session's CSRF token, and change
/// nothing.
#[test]
fn agent_017_api_consent_forms_need_the_csrf_token() {
    run(async {
        let email = "forms@example.test";
        register(email).await;
        verify(email).await;
        let visitor = browser();
        let (_, access) = approve_through_sign_in(&visitor, email).await;

        // A second client's approval, in the same signed-in browser.
        let client_id = register_client(&visitor).await;
        let (_, challenge) = pkce();
        let to_approval = visitor
            .get(authorize_path(&client_id, &challenge))
            .send()
            .await;
        assert_eq!(to_approval.status(), 302, "{}", to_approval.body_text());
        let approval = path_of(&location(&to_approval));
        let page = visitor.get(&approval).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        let id = query_param(&approval, "authorization").expect("the authorization");
        let decision = format!("/oauth/authorizations/{id}");
        let refused = visitor
            .post(&decision)
            .form(&[("decision", "approve")])
            .send()
            .await;
        assert_eq!(refused.status(), 419, "approval: {}", refused.body_text());
        let decided = visitor
            .post(&decision)
            .form(&[
                ("_token", form_token(&page.body_text()).as_str()),
                ("decision", "approve"),
            ])
            .send()
            .await;
        assert!(
            matches!(decided.status(), 302 | 303),
            "the approval was still open: {} {}",
            decided.status(),
            decided.body_text()
        );

        // The revoke form of the client whose code was exchanged, the only
        // one with a grant.
        let connected = visitor.get(HOME).send().await;
        assert_eq!(connected.status(), 200, "{}", connected.body_text());
        let revoke = form_action(&connected.body_text(), "/revoke");
        let refused = visitor.post(&revoke).send().await;
        assert_eq!(refused.status(), 419, "revoke: {}", refused.body_text());
        assert_eq!(mcp_status(&access).await, 200, "nothing was revoked");

        // The claim form.
        let registration = register_agent(email).await;
        let claim = path_of(
            registration["claim"]["verification_uri"]
                .as_str()
                .expect("a verification URI"),
        );
        let page = visitor.get(&claim).send().await;
        assert_eq!(page.status(), 200, "{}", page.body_text());
        let attempt = field(&page.body_text(), "claim_attempt_token");
        let code = registration["claim"]["user_code"]
            .as_str()
            .expect("a user code");
        let refused = visitor
            .post("/agent/claim")
            .form(&[
                ("claim_attempt_token", attempt.as_str()),
                ("user_code", code),
            ])
            .send()
            .await;
        assert_eq!(refused.status(), 419, "claim: {}", refused.body_text());
        let claimed = visitor
            .post("/agent/claim")
            .form(&[
                ("_token", form_token(&page.body_text()).as_str()),
                ("claim_attempt_token", attempt.as_str()),
                ("user_code", code),
            ])
            .send()
            .await;
        assert_eq!(
            claimed.status(),
            200,
            "the claim was still open: {}",
            claimed.body_text()
        );

        // The sign-out form.
        let refused = visitor.post("/logout").send().await;
        assert_eq!(refused.status(), 419, "sign-out: {}", refused.body_text());
        let still = visitor.get(HOME).send().await;
        assert_eq!(still.status(), 200, "still signed in");
    });
}

/// AGENT-017: the route that mails a new link answers at most six times a
/// minute for one user. A verified user's request mails nothing (`204`), so
/// the seventh `429` is the route's limit and not Magnetar's per-address
/// mail budget.
#[test]
fn agent_017_api_verification_resend_is_limited() {
    run(async {
        let email = "resend-limit@example.test";
        register(email).await;
        verify(email).await;
        let client = api_client(email).await;
        for attempt in 1..=6 {
            let answer = client
                .post("/api/email/verification-notification")
                .send()
                .await;
            assert_eq!(
                answer.status(),
                204,
                "attempt {attempt}: {}",
                answer.body_text()
            );
        }
        let seventh = client
            .post("/api/email/verification-notification")
            .send()
            .await;
        assert_eq!(seventh.status(), 429, "{}", seventh.body_text());
    });
}

/// AGENT-017: registration answers `202` and keeps the account when the
/// verification mail cannot go, here because the SMTP server is down. The
/// account asks for a new link later.
#[test]
fn agent_017_api_registration_answers_202_when_the_mail_cannot_go() {
    run(async {
        // A port nothing listens on: bound, then released.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("a free port")
            .local_addr()
            .expect("its address")
            .port();
        // `Mail::fake` keeps the transport it replaces and puts it back when
        // the guard drops, so the run's capture comes back after this case.
        let restore = suprnova::Mail::fake();
        suprnova::Mail::set_transport(std::sync::Arc::new(
            suprnova::mail::smtp::SmtpMailTransport::unencrypted("127.0.0.1", port)
                .expect("an SMTP transport"),
        ))
        .expect("send mail to the dead port");
        let email = "dead-smtp@example.test";
        let response = browser()
            .post("/api/auth/register")
            .header("Accept", "application/json")
            .json(&json!({ "email": email, "password": PASSWORD }))
            .send()
            .await;
        drop(restore);
        assert_eq!(response.status(), 202, "{}", response.body_text());
        assert!(
            User::find_by_email(email)
                .await
                .expect("read the account")
                .is_some(),
            "the account was kept"
        );
        assert_eq!(mails_to(email), 0, "no link was captured");
    });
}
