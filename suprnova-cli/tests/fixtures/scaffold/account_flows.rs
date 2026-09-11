//! Behavioural proof for a generated application's account flows,
//! validation-error display, and static-file serving.
//!
//! Written into a freshly scaffolded project by
//! `suprnova-cli/tests/scaffold_account_flows.rs`, which replaces
//! `__PACKAGE__` with the generated crate's name. Drives the generated
//! router in-process over a real Hyper connection, with the session,
//! CSRF, Inertia and error-page middleware all in place and mail captured
//! in memory.

use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use suprnova::serde_json::{self, Value, json};
use suprnova::{MiddlewareRegistry, Router, handle_request};

use __PACKAGE__::models::user::User;

/// Boot the generated application the way `cmd/main.rs` does, minus the
/// listener, and capture outgoing mail.
async fn setup() -> suprnova::mail::MailFake {
    use sea_orm_migration::MigratorTrait;
    assert_eq!(
        std::env::var("APP_ENV").as_deref(),
        Ok("test"),
        "the harness runs this binary with APP_ENV=test and never loads .env"
    );
    __PACKAGE__::config::register_all();
    suprnova::Crypt::init(suprnova::EncryptionKey::from_env().expect("APP_KEY is set"));
    __PACKAGE__::bootstrap::register().await;
    __PACKAGE__::migrations::Migrator::up(suprnova::DB::connection().unwrap().inner(), None)
        .await
        .expect("migrations apply");
    suprnova::rate_limit::bootstrap_default().await;
    __PACKAGE__::bootstrap::register_http_stack();
    suprnova::Mail::fake()
}

/// The token in the most recent captured mail whose link lands on `path`.
fn mail_token(mail: &suprnova::mail::MailFake, path: &str) -> String {
    let prefix = format!("http://app.test/{path}?token=");
    mail.captured()
        .iter()
        .rev()
        .filter_map(|message| message.text.as_deref())
        .flat_map(str::lines)
        .find_map(|line| {
            line.split_once(&prefix)
                .map(|(_, token)| token.trim().to_owned())
        })
        .unwrap_or_else(|| panic!("no captured mail carries a `{prefix}` link"))
}

fn page(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("Inertia page object: {e}\n{body}"))
}

#[derive(Clone)]
struct Client {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    cookies: HashMap<String, String>,
}

struct Response {
    status: u16,
    location: Option<String>,
    content_type: Option<String>,
    body: String,
}

impl Client {
    fn new() -> Self {
        Self {
            router: Arc::new(__PACKAGE__::routes::register()),
            middleware: Arc::new(MiddlewareRegistry::from_global()),
            cookies: HashMap::new(),
        }
    }

    async fn get(&mut self, path: &str) -> Response {
        self.exchange("GET", path, None, false).await
    }

    async fn head(&mut self, path: &str) -> Response {
        self.exchange("HEAD", path, None, false).await
    }

    async fn post(&mut self, path: &str, body: Value) -> Response {
        self.exchange("POST", path, Some(body), false).await
    }

    async fn inertia_get(&mut self, path: &str) -> Response {
        self.exchange("GET", path, None, true).await
    }

    async fn inertia_post(&mut self, path: &str, body: Value) -> Response {
        self.exchange("POST", path, Some(body), true).await
    }

    async fn exchange(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        inertia: bool,
    ) -> Response {
        // One real Hyper connection per exchange; both tasks are aborted on
        // timeout or panic as well as on normal completion.
        let (client, server) = tokio::io::duplex(64 * 1024);
        let router = self.router.clone();
        let middleware = self.middleware.clone();
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let service = service_fn(move |req: hyper::Request<Incoming>| {
                let router = router.clone();
                let middleware = middleware.clone();
                async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server), service)
                .await
        });
        let payload = body.map(|value| value.to_string()).unwrap_or_default();
        let response = tokio::time::timeout(Duration::from_secs(15), async {
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client))
                    .await
                    .expect("HTTP handshake");
            tasks.spawn(connection);
            let mut request = hyper::Request::builder()
                .method(method)
                .uri(path)
                .header("Host", "app.test")
                .header("Accept", "text/html");
            if !payload.is_empty() {
                request = request.header("Content-Type", "application/json");
            }
            if inertia {
                request = request
                    .header("X-Inertia", "true")
                    .header(
                        "X-Inertia-Version",
                        suprnova::InertiaConfig::new().version.resolve(),
                    )
                    .header("Referer", format!("http://app.test{path}"));
            }
            if !self.cookies.is_empty() {
                request = request.header(
                    "Cookie",
                    self.cookies
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
            if method != "GET"
                && method != "HEAD"
                && let Some(token) = self.cookies.get("XSRF-TOKEN")
            {
                request = request.header("X-XSRF-TOKEN", token);
            }
            let response = sender
                .send_request(
                    request
                        .body(Full::new(Bytes::from(payload)))
                        .expect("request"),
                )
                .await
                .expect("HTTP response");
            let (parts, body) = response.into_parts();
            for cookie in parts.headers.get_all("set-cookie") {
                let cookie = cookie.to_str().expect("cookie header");
                let mut fields = cookie.split(';');
                let (name, value) = fields
                    .next()
                    .unwrap()
                    .split_once('=')
                    .expect("cookie pair");
                if value.is_empty()
                    || fields.any(|field| field.trim().eq_ignore_ascii_case("max-age=0"))
                {
                    self.cookies.remove(name);
                } else {
                    self.cookies.insert(name.to_owned(), value.to_owned());
                }
            }
            let header = |name: &str| {
                parts
                    .headers
                    .get(name)
                    .map(|value| value.to_str().expect("ASCII header").to_owned())
            };
            let location = header("location");
            let content_type = header("content-type");
            let body_bytes = body
                .collect()
                .await
                .expect("response body")
                .to_bytes()
                .to_vec();
            Response {
                status: parts.status.as_u16(),
                location,
                content_type,
                body: String::from_utf8_lossy(&body_bytes).into_owned(),
            }
        })
        .await
        .expect("HTTP exchange exceeded 15 seconds");
        tasks.abort_all();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Err(error) if error.is_cancelled() => {}
                Ok(Err(error)) => panic!("HTTP connection failed: {error}"),
                Err(error) => panic!("HTTP task failed: {error}"),
            }
        }
        response
    }
}

fn assert_redirects_to(response: &Response, path: &str) {
    let location = response.location.as_deref().unwrap_or("");
    assert!(
        (response.status == 302 || response.status == 303) && location.ends_with(path),
        "expected a redirect to {path}, got {} to {location:?}: {}",
        response.status,
        response.body
    );
}

#[tokio::test]
async fn account_flows_validation_errors_and_static_files() {
    let mail = setup().await;
    let mut client = Client::new();

    // A browser loads the HTML shell before its first Inertia visit, and
    // that first load is what establishes the session every later visit
    // carries (a fresh session is stored only once a request writes to
    // it). Do the same before driving the Inertia protocol directly.
    assert_eq!(client.get("/login").await.status, 200);

    // ---- Flashed validation errors reach the auth pages (#6) ----
    let clean = client.inertia_get("/login").await;
    assert_eq!(clean.status, 200, "{}", clean.body);
    let props = &page(&clean.body)["props"];
    assert_eq!(
        props["errors"],
        json!({}),
        "a clean GET carries the framework's empty errors bag, not null: {props}"
    );

    let denied = client
        .inertia_post(
            "/login",
            json!({"email": "nobody@example.test", "password": "wrong"}),
        )
        .await;
    assert_redirects_to(&denied, "/login");
    let after = client.inertia_get("/login").await;
    let errors = &page(&after.body)["props"]["errors"];
    assert!(
        errors["email"]
            .as_str()
            .is_some_and(|message| message.contains("credentials")),
        "the login page must show the flashed credential error; got {errors}"
    );

    let denied = client
        .inertia_post(
            "/register",
            json!({"name": "A", "email": "invalid", "password": "short", "password_confirmation": "different"}),
        )
        .await;
    assert_redirects_to(&denied, "/register");
    let after = client.inertia_get("/register").await;
    let errors = &page(&after.body)["props"]["errors"];
    for field in ["name", "email", "password"] {
        assert!(
            errors[field].is_string(),
            "the register page must show the flashed `{field}` error; got {errors}"
        );
    }
    assert_eq!(
        client
            .post(
                "/register",
                json!({"name": "A", "email": "invalid", "password": "short", "password_confirmation": "different"}),
            )
            .await
            .status,
        422,
        "a plain client still gets the 422 envelope"
    );

    // ---- Registration mails a verification link (#8) ----
    let registered = client
        .post(
            "/register",
            json!({
                "name": "Ada Example", "email": "ada@example.test",
                "password": "first-password-123", "password_confirmation": "first-password-123"
            }),
        )
        .await;
    assert_redirects_to(&registered, "/verify-email");
    assert_eq!(mail.count(), 1, "registration sends exactly one mail");
    assert!(
        mail.captured()[0].has_to("ada@example.test"),
        "the verification mail goes to the registered address"
    );

    let notice = client.inertia_get("/verify-email").await;
    assert_eq!(notice.status, 200, "{}", notice.body);
    let notice = page(&notice.body);
    assert_eq!(notice["component"], json!("auth/VerifyEmail"));
    assert_eq!(notice["props"]["email"], json!("ada@example.test"));

    let resent = client.post("/email/verification-notification", json!({})).await;
    assert_redirects_to(&resent, "/verify-email");
    assert_eq!(mail.count(), 2, "resend delivers another link");
    let token = mail_token(&mail, "verify-email/verify");

    // Another signed-in account cannot consume the link.
    let other = User::create("Other Account", "other@example.test", "other-password-123")
        .await
        .expect("create the other user");
    let mut stranger = Client::new();
    stranger.get("/login").await;
    assert_redirects_to(
        &stranger
            .post(
                "/login",
                json!({"email": "other@example.test", "password": "other-password-123"}),
            )
            .await,
        "/dashboard",
    );
    assert_eq!(
        stranger
            .get(&format!("/verify-email/verify?token={token}"))
            .await
            .status,
        400,
        "a foreign token is refused"
    );
    assert!(
        User::find_by_email(&other.email)
            .await
            .unwrap()
            .unwrap()
            .email_verified_at
            .is_none(),
        "the stranger stays unverified"
    );

    // The owner consumes it once.
    let verified = client
        .get(&format!("/verify-email/verify?token={token}"))
        .await;
    assert_redirects_to(&verified, "/dashboard");
    let ada = User::find_by_email("ada@example.test")
        .await
        .unwrap()
        .unwrap();
    assert!(ada.email_verified_at.is_some(), "the owner is verified");
    for reused_or_bad in [token.as_str(), "invalid"] {
        assert_eq!(
            client
                .get(&format!("/verify-email/verify?token={reused_or_bad}"))
                .await
                .status,
            400,
            "a reused or invalid token is refused"
        );
    }
    let expired = suprnova::auth_flows::token_store::TokenStore::issue(
        &ada.id.to_string(),
        suprnova::auth_flows::token_store::TokenPurpose::EmailVerification,
        chrono::Duration::seconds(-1),
    )
    .await
    .expect("issue an already expired token");
    assert_eq!(
        client
            .get(&format!("/verify-email/verify?token={expired}"))
            .await
            .status,
        400,
        "an expired token is refused"
    );

    // A verified account has nothing to do on the notice page and gets no
    // further mail.
    assert_redirects_to(&client.get("/verify-email").await, "/dashboard");
    let before = mail.count();
    assert_redirects_to(
        &client.post("/email/verification-notification", json!({})).await,
        "/verify-email",
    );
    assert_eq!(mail.count(), before, "no mail for a verified account");

    // ---- Password reset (#8) ----
    let mut recovery = Client::new();
    assert_eq!(recovery.get("/forgot-password").await.status, 200);
    let before = mail.count();
    let unverified = recovery
        .post("/forgot-password", json!({"email": "other@example.test"}))
        .await;
    let unknown = recovery
        .post("/forgot-password", json!({"email": "unknown@example.test"}))
        .await;
    assert_eq!(
        mail.count(),
        before,
        "neither an unverified nor an unknown address receives a link"
    );
    assert_eq!(
        (unverified.status, &unverified.location, &unverified.body),
        (unknown.status, &unknown.location, &unknown.body),
        "the two answers are indistinguishable"
    );
    let sent = recovery
        .post("/forgot-password", json!({"email": "ada@example.test"}))
        .await;
    assert_redirects_to(&sent, "/forgot-password");
    assert_eq!(mail.count(), before + 1, "a verified address gets a link");
    let reset_token = mail_token(&mail, "reset-password");

    let form = recovery
        .inertia_get(&format!("/reset-password?token={reset_token}"))
        .await;
    assert_eq!(form.status, 200, "{}", form.body);
    let form = page(&form.body);
    assert_eq!(form["component"], json!("auth/ResetPassword"));
    assert_eq!(form["props"]["token"], json!(reset_token));

    let bad_token = recovery
        .inertia_post(
            "/reset-password",
            json!({"token": "invalid", "password": "next-password-456", "password_confirmation": "next-password-456"}),
        )
        .await;
    assert_redirects_to(&bad_token, "/reset-password");
    let after = recovery.inertia_get("/reset-password?token=invalid").await;
    let errors = &page(&after.body)["props"]["errors"];
    assert!(
        errors["token"].is_string(),
        "an invalid token is a field error on the form; got {errors}"
    );
    let mismatch = recovery
        .inertia_post(
            "/reset-password",
            json!({"token": reset_token, "password": "next-password-456", "password_confirmation": "other"}),
        )
        .await;
    assert_redirects_to(&mismatch, "/reset-password");
    assert_eq!(
        mail.count(),
        before + 1,
        "a rejected submission changes nothing and sends nothing"
    );

    let done = recovery
        .post(
            "/reset-password",
            json!({"token": reset_token, "password": "next-password-456", "password_confirmation": "next-password-456"}),
        )
        .await;
    assert_redirects_to(&done, "/login");
    assert_eq!(
        mail.count(),
        before + 2,
        "completing the reset sends the password-changed notice"
    );
    assert_eq!(
        recovery
            .post(
                "/reset-password",
                json!({"token": reset_token, "password": "again-password-789", "password_confirmation": "again-password-789"}),
            )
            .await
            .status,
        422,
        "a consumed token is refused"
    );
    assert_redirects_to(
        &client.get("/dashboard").await,
        "/login",
    );

    let mut fresh = Client::new();
    fresh.get("/login").await;
    assert_eq!(
        fresh
            .post(
                "/login",
                json!({"email": "ada@example.test", "password": "first-password-123"}),
            )
            .await
            .status,
        422,
        "the old password no longer works"
    );
    assert_redirects_to(
        &fresh
            .post(
                "/login",
                json!({"email": "ada@example.test", "password": "next-password-456"}),
            )
            .await,
        "/dashboard",
    );
    assert_eq!(fresh.get("/dashboard").await.status, 200);

    // ---- Static files through the fallback (#7) ----
    let asset = fresh.get("/assets/app-1234.js").await;
    assert_eq!(asset.status, 200, "{}", asset.body);
    assert!(
        asset
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("text/javascript")),
        "content type: {:?}",
        asset.content_type
    );
    assert_eq!(asset.body, "console.log('built')\n");
    assert_eq!(fresh.head("/assets/app-1234.js").await.status, 200);
    assert_eq!(
        fresh.get("/assets/.vite/manifest.json").await.status,
        404,
        "dotfiles under public/ stay private"
    );
    assert_eq!(
        fresh.get("/assets/../Cargo.toml").await.status,
        404,
        "traversal out of public/ is refused"
    );
    assert_eq!(fresh.get("/Cargo.toml").await.status, 404);
    let missing = fresh.inertia_get("/nowhere").await;
    assert_eq!(missing.status, 404, "{}", missing.body);
    assert_eq!(
        page(&missing.body)["component"],
        json!("Error"),
        "an unknown URL still renders the Inertia error page"
    );
}
