//! Laravel 13.35.0 auth gaps in the auth flows: the verified-email
//! middleware's per-request answer (PAR-124), the verification hook and the
//! answers of `verify` (PAR-125), the reset hook and the reset timebox
//! (PAR-126), and the password rehash moved from `validate_credentials` to
//! the sign-in (PAR-132). The `par-laravel-gaps-auth` mechanism selects this
//! module by name.

use std::any::Any;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serial_test::serial;

use crate::env_snapshot::{EnvSnapshot, set_env};
use suprnova::auth::AuthConfig;
use suprnova::auth_flows::events::EmailVerified;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::auth_flows::{EmailVerification, PasswordReset, VerifyEmailNotification};
use suprnova::container::testing::TestContainer;
use suprnova::notifications::{Notify, NotifyFakeGuard};
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, AuthFlowUser, AuthManager, Authenticatable, CanResetPassword, Credentials,
    DatabaseUserProvider, EloquentUserProvider, EnsureEmailVerifiedMiddleware, EventFacade,
    FrameworkError, HttpResponse, Mail, Middleware, MustVerifyEmail, Next, Request, UserProvider,
    model,
};

// ── Models ──────────────────────────────────────────────────────────────

/// A user that keeps every default hook. `email_for_reset` is
/// `reset_email` when one is set, so a test can tell the reset address from
/// the sign-in address.
#[model(table = "users", fillable = ["email", "password"])]
pub struct GapUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub reset_email: Option<String>,
}

/// The same table, with both hooks overridden to record the link instead
/// of sending anything.
#[model(table = "users", fillable = ["email", "password"])]
pub struct HookedUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub reset_email: Option<String>,
}

macro_rules! auth_user {
    ($model:ty) => {
        impl Authenticatable for $model {
            fn get_auth_identifier(&self) -> String {
                self.id.to_string()
            }
            fn get_auth_password(&self) -> Option<&str> {
                Some(&self.password)
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
                self
            }
        }
    };
}

auth_user!(GapUser);
auth_user!(HookedUser);

impl MustVerifyEmail for GapUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}

impl CanResetPassword for GapUser {
    fn email_for_reset(&self) -> &str {
        self.reset_email.as_deref().unwrap_or(&self.email)
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_owned();
    }
}

/// One link a [`HookedUser`] hook received.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hooked {
    hook: &'static str,
    address: String,
    link: String,
}

static HOOKED: Mutex<Vec<Hooked>> = Mutex::new(Vec::new());

fn record_hook(hook: &'static str, address: &str, link: &str) {
    HOOKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(Hooked {
            hook,
            address: address.to_owned(),
            link: link.to_owned(),
        });
}

/// The links the hook `hook` received for `address`, oldest first.
fn hooked(hook: &str, address: &str) -> Vec<String> {
    HOOKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|entry| entry.hook == hook && entry.address == address)
        .map(|entry| entry.link.clone())
        .collect()
}

impl MustVerifyEmail for HookedUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
    async fn send_email_verification_notification(
        &self,
        verification_link: &str,
    ) -> Result<(), FrameworkError> {
        record_hook("verify", &self.email, verification_link);
        Ok(())
    }
}

impl CanResetPassword for HookedUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_owned();
    }
    async fn send_password_reset_notification(
        &self,
        reset_link: &str,
    ) -> Result<(), FrameworkError> {
        record_hook("reset", &self.email, reset_link);
        Ok(())
    }
}

// ── Providers without a model ───────────────────────────────────────────

/// A provider whose users live in code: `grace@example.com` (id 9) can
/// verify; `orphan@example.com` (id 10) has no verification address.
struct VerificationDirectory;

#[async_trait::async_trait]
impl UserProvider for VerificationDirectory {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }

    async fn retrieve_by_email(&self, email: &str) -> Result<Option<AuthFlowUser>, FrameworkError> {
        let id = match email {
            "grace@example.com" => "9",
            "orphan@example.com" => "10",
            _ => return Ok(None),
        };
        Ok(Some(AuthFlowUser {
            id: id.to_owned(),
            email: email.to_owned(),
            name: None,
        }))
    }

    async fn flow_user_by_id(&self, id: &str) -> Result<Option<AuthFlowUser>, FrameworkError> {
        Ok((id == "9").then(|| AuthFlowUser {
            id: "9".to_owned(),
            email: "grace@example.com".to_owned(),
            name: Some("Grace".to_owned()),
        }))
    }
}

/// A reset-capable provider whose users live in code: `known@example.com`
/// (id 5, named Kim) and `nameless@example.com` (id 6), which
/// `flow_user_by_id` does not know. Every other address has no account.
struct ResetDirectory;

#[async_trait::async_trait]
impl UserProvider for ResetDirectory {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }

    fn supports_password_reset(&self) -> bool {
        true
    }

    async fn retrieve_verified_user_for_password_reset(
        &self,
        email: &str,
    ) -> Result<Option<AuthFlowUser>, FrameworkError> {
        let id = match email {
            "known@example.com" => "5",
            "nameless@example.com" => "6",
            _ => return Ok(None),
        };
        Ok(Some(AuthFlowUser {
            id: id.to_owned(),
            email: email.to_owned(),
            name: None,
        }))
    }

    async fn flow_user_by_id(&self, id: &str) -> Result<Option<AuthFlowUser>, FrameworkError> {
        Ok((id == "5").then(|| AuthFlowUser {
            id: "5".to_owned(),
            email: "known@example.com".to_owned(),
            name: Some("Kim".to_owned()),
        }))
    }
}

// ── Harness ─────────────────────────────────────────────────────────────

/// The environment keys these tests change; the snapshot restores them.
const ENV_KEYS: &[&str] = &["MAIL_FROM", "PASSWORD_RESET_TIMEBOX_MS", "HASH_ROUNDS"];

/// An in-memory database with the `users` and `auth_flow_tokens` tables,
/// `provider` registered as the `users` provider of the default guard, the
/// default rate limiter, and `MAIL_FROM` set. The caller holds the env lock.
struct Harness {
    db: TestDatabase,
    _env: EnvSnapshot,
}

async fn harness(provider: Arc<dyn UserProvider>) -> Harness {
    let env = EnvSnapshot::capture(ENV_KEYS);
    set_env("MAIL_FROM", Some("test-mailer@example.test"));
    set_env("PASSWORD_RESET_TIMEBOX_MS", None);
    let db = TestDatabase::sqlite_memory().await.expect("sqlite_memory");
    db.conn()
        .execute_unprepared(
            "CREATE TABLE users (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                email TEXT NOT NULL, \
                password TEXT NOT NULL, \
                email_verified_at TEXT, \
                reset_email TEXT\
             )",
        )
        .await
        .expect("create users table");
    db.conn()
        .execute(&create_auth_flow_tokens_table())
        .await
        .expect("create auth_flow_tokens table");
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", provider).expect("register provider");
    suprnova::rate_limit::bootstrap_default().await;
    Harness { db, _env: env }
}

impl Harness {
    /// Insert user `id` with `email`, the password hash `hash`, an optional
    /// verification time (RFC 3339) and an optional reset address.
    async fn seed(
        &self,
        id: i64,
        email: &str,
        hash: &str,
        verified_at: Option<&str>,
        reset_email: Option<&str>,
    ) {
        self.db
            .conn()
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "INSERT INTO users (id, email, password, email_verified_at, reset_email) \
                 VALUES (?, ?, ?, ?, ?)",
                [
                    id.into(),
                    email.into(),
                    hash.into(),
                    verified_at.map(str::to_owned).into(),
                    reset_email.map(str::to_owned).into(),
                ],
            ))
            .await
            .expect("seed user");
    }

    /// The stored column `column` of user `id`, as text.
    async fn column(&self, id: i64, column: &str) -> Option<String> {
        let row = self
            .db
            .fetch_one(
                &format!("SELECT {column} AS value FROM users WHERE id = ?"),
                vec![id.into()],
            )
            .await
            .expect("read user");
        row.try_get::<Option<String>>("", "value").expect("value")
    }

    /// How many verification and reset tokens are stored.
    async fn token_rows(&self) -> i64 {
        let row = self
            .db
            .fetch_one("SELECT COUNT(*) AS n FROM auth_flow_tokens", vec![])
            .await
            .expect("count tokens");
        row.try_get::<i64>("", "n").expect("count")
    }
}

/// Load user `id` as the model `M` through its Eloquent provider.
async fn load<M>(id: &str) -> M
where
    M: Clone + 'static,
    EloquentUserProvider<M>: UserProvider,
{
    EloquentUserProvider::<M>::new()
        .retrieve_by_id(id)
        .await
        .expect("lookup")
        .expect("user exists")
        .as_any()
        .downcast_ref::<M>()
        .expect("the model type")
        .clone()
}

/// The token a link carries.
fn token_of(link: &str) -> String {
    link.rsplit("token=")
        .next()
        .expect("token")
        .trim()
        .to_owned()
}

/// The one verification link the notification fake recorded for `address`.
fn notified_link(notify: &NotifyFakeGuard, address: &str) -> String {
    let sent = notify
        .sent::<VerifyEmailNotification>(address, |_| true)
        .expect("decode recorded notifications");
    assert_eq!(sent.len(), 1, "one verification notification to {address}");
    sent[0].mail.verification_link.clone()
}

/// Run `verify(token)` as the signed-in user `user_id`.
async fn verify_as(user_id: &str, token: &str) -> Result<String, FrameworkError> {
    let slot = suprnova::session::new_session_slot_for_test();
    let user_id = user_id.to_owned();
    suprnova::session::session_scope_for_test(slot, async move {
        suprnova::session::set_auth_user(user_id);
        EmailVerification::verify(token).await
    })
    .await
}

/// Run `fut` inside the session, pending-cookie and auth request scopes a
/// request has.
async fn in_request<F: Future>(fut: F) -> F::Output {
    let session = suprnova::session::new_session_slot_for_test();
    let cookies = suprnova::session::new_pending_cookies_slot_for_test();
    suprnova::session::session_scope_for_test(
        session,
        suprnova::session::pending_cookies_scope_for_test(
            cookies,
            suprnova::auth::request_state::request_state_scope_for_test(fut),
        ),
    )
    .await
}

// ── PAR-124: the verified-email middleware answers each request ─────────

/// A provider that resolves nobody and reports the listed ids verified.
struct VerifiedIds(&'static [&'static str]);

#[async_trait::async_trait]
impl UserProvider for VerifiedIds {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }

    async fn is_email_verified(&self, id: &str) -> Result<bool, FrameworkError> {
        Ok(self.0.contains(&id))
    }
}

/// A signed-in user carrying only its id.
struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// What one request through the gate produced.
struct Gated {
    response: HttpResponse,
    reached: bool,
    intended: Option<String>,
}

/// Send `request` through `gate` with user `user_id` signed in (user 8 is
/// verified, every other id is not) and the session's previous URL set to
/// `previous`.
async fn through_gate(
    gate: &EnsureEmailVerifiedMiddleware,
    request: Request,
    user_id: Option<&str>,
    previous: Option<&str>,
) -> Gated {
    TestContainer::scope(async {
        TestContainer::singleton(AuthManager::new(AuthConfig::default()));
        Auth::register_provider("users", Arc::new(VerifiedIds(&["8"]))).expect("provider");
        let slot = suprnova::session::new_session_slot_for_test();
        if let Some(previous) = previous {
            slot.lock()
                .expect("session slot")
                .as_mut()
                .expect("session")
                .set_previous_url(previous);
        }
        let reached = Arc::new(Mutex::new(false));
        let seen = Arc::clone(&reached);
        let next: Next = Arc::new(move |_request| {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                *seen.lock().expect("reached flag") = true;
                Ok(HttpResponse::text("reached"))
            })
        });
        let user_id = user_id.map(str::to_owned);
        let response = suprnova::session::session_scope_for_test(
            Arc::clone(&slot),
            suprnova::auth::request_state::request_state_scope_for_test(async move {
                if let Some(id) = user_id {
                    Auth::set_user(Arc::new(UserById(id)));
                }
                gate.handle(request, next).await
            }),
        )
        .await;
        let response = response.unwrap_or_else(|response| response);
        let intended = slot
            .lock()
            .expect("session slot")
            .as_ref()
            .and_then(|session| session.get::<String>("url.intended"));
        let reached = *reached.lock().expect("reached flag");
        Gated {
            response,
            reached,
            intended,
        }
    })
    .await
}

fn browser(method: &str, uri: &str) -> Request {
    Request::for_test_with_headers(method, uri, [("accept", "text/html,application/xhtml+xml")])
}

fn json_message(response: &HttpResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(response.body()).expect("a JSON body");
    body["message"].as_str().expect("a message").to_owned()
}

#[tokio::test]
async fn a_request_that_expects_json_gets_403_even_when_a_redirect_is_named() {
    let gate = EnsureEmailVerifiedMiddleware::redirect_to("/email/verify");
    for request in [
        Request::for_test_with_headers("GET", "/billing", [("accept", "application/json")]),
        // An XHR that accepts anything expects JSON too, as Laravel's
        // `expectsJson` reads it.
        Request::for_test_with_headers("GET", "/billing", [("x-requested-with", "XMLHttpRequest")]),
    ] {
        let gated = through_gate(&gate, request, Some("1"), None).await;
        assert_eq!(gated.response.status_code(), 403);
        assert_eq!(
            json_message(&gated.response),
            "Your email address is not verified."
        );
        assert_eq!(gated.response.header_value("location"), None);
        assert!(!gated.reached);
        assert_eq!(gated.intended, None, "a JSON caller stores no intended URL");
    }
}

#[tokio::test]
async fn a_browser_get_is_redirected_and_its_path_and_query_become_the_intended_url() {
    let gate = EnsureEmailVerifiedMiddleware::redirect_to("/email/verify");
    let gated = through_gate(&gate, browser("GET", "/billing?tab=2"), Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 302);
    assert_eq!(
        gated.response.header_value("location"),
        Some("/email/verify")
    );
    assert_eq!(gated.intended.as_deref(), Some("/billing?tab=2"));
    assert!(!gated.reached);
}

#[tokio::test]
async fn a_post_stores_the_previous_page_never_its_own_path() {
    let gate = EnsureEmailVerifiedMiddleware::redirect_to("/email/verify");
    let gated = through_gate(
        &gate,
        browser("POST", "/billing"),
        Some("1"),
        Some("/plans"),
    )
    .await;
    assert_eq!(gated.response.status_code(), 302);
    assert_eq!(
        gated.response.header_value("location"),
        Some("/email/verify")
    );
    assert_eq!(gated.intended.as_deref(), Some("/plans"));

    let gated = through_gate(&gate, browser("POST", "/billing"), Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 302);
    assert_eq!(
        gated.intended, None,
        "with no previous page, a POST stores nothing, never /billing"
    );
}

#[tokio::test]
async fn an_inertia_visit_gets_409_and_stores_the_intended_url() {
    let gate = EnsureEmailVerifiedMiddleware::redirect_to("/email/verify");
    let request = Request::for_test_with_headers(
        "GET",
        "/billing?tab=2",
        [
            ("x-inertia", "true"),
            ("x-requested-with", "XMLHttpRequest"),
            ("accept", "text/html, application/xhtml+xml"),
        ],
    );
    let gated = through_gate(&gate, request, Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 409);
    assert_eq!(
        gated.response.header_value("x-inertia-location"),
        Some("/email/verify")
    );
    assert_eq!(gated.intended.as_deref(), Some("/billing?tab=2"));
}

#[tokio::test]
async fn redirect_to_route_resolves_the_route_name_on_each_request() {
    suprnova::routing::register_route_name("verification.notice", "/email/verify");
    let gate = EnsureEmailVerifiedMiddleware::redirect_to_route("verification.notice");
    let gated = through_gate(&gate, browser("GET", "/billing?tab=2"), Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 302);
    assert_eq!(
        gated.response.header_value("location"),
        Some("/email/verify")
    );
    assert_eq!(gated.intended.as_deref(), Some("/billing?tab=2"));

    // A route named after the middleware was built is found by the next
    // request: the name is resolved per request, not at construction.
    let late = EnsureEmailVerifiedMiddleware::redirect_to_route("verification.late-notice");
    let before = through_gate(&late, browser("GET", "/billing"), Some("1"), None).await;
    assert_eq!(before.response.status_code(), 500);
    suprnova::routing::register_route_name("verification.late-notice", "/verify-later");
    let after = through_gate(&late, browser("GET", "/billing"), Some("1"), None).await;
    assert_eq!(after.response.status_code(), 302);
    assert_eq!(
        after.response.header_value("location"),
        Some("/verify-later")
    );
}

#[tokio::test]
async fn redirect_to_an_unknown_route_fails_the_request_with_a_500_naming_it() {
    let gate = EnsureEmailVerifiedMiddleware::redirect_to_route("verification.nowhere");
    let gated = through_gate(&gate, browser("GET", "/billing"), Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 500);
    assert_eq!(gated.response.header_value("location"), None);
    let report = gated
        .response
        .error_report()
        .expect("the 500 carries its error");
    assert!(
        report
            .chain()
            .iter()
            .any(|message| message.contains("verification.nowhere")),
        "the error names the route: {:?}",
        report.chain()
    );
    assert!(!gated.reached);

    // A JSON caller never needs the redirect, so it still gets the 403.
    let json = Request::for_test_with_headers("GET", "/billing", [("accept", "application/json")]);
    let gated = through_gate(&gate, json, Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 403);

    // A route whose path needs a parameter fails the same way, rather than
    // sending a `Location` with a raw placeholder.
    suprnova::routing::register_route_name("verification.per-user", "/verify/{id}");
    let gate = EnsureEmailVerifiedMiddleware::redirect_to_route("verification.per-user");
    let gated = through_gate(&gate, browser("GET", "/billing"), Some("1"), None).await;
    assert_eq!(gated.response.status_code(), 500);
    assert_eq!(gated.response.header_value("location"), None);
}

#[tokio::test]
async fn new_answers_403_to_a_browser_and_a_verified_user_passes_every_form() {
    let gated = through_gate(
        &EnsureEmailVerifiedMiddleware::new(),
        browser("GET", "/billing"),
        Some("1"),
        None,
    )
    .await;
    assert_eq!(gated.response.status_code(), 403);
    assert_eq!(
        json_message(&gated.response),
        "Your email address is not verified."
    );
    assert_eq!(gated.intended, None);

    let guest = through_gate(
        &EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"),
        browser("GET", "/billing"),
        None,
        None,
    )
    .await;
    assert_eq!(
        guest.response.status_code(),
        302,
        "a guest falls into the unverified branch"
    );

    for gate in [
        EnsureEmailVerifiedMiddleware::new(),
        EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"),
        // A verified user never needs the redirect, so an unknown route
        // name does not fail the request.
        EnsureEmailVerifiedMiddleware::redirect_to_route("verification.unused"),
    ] {
        let gated = through_gate(&gate, browser("GET", "/billing"), Some("8"), None).await;
        assert!(gated.reached, "the verified user reaches the handler");
        assert_eq!(gated.response.status_code(), 200);
    }
}

// ── PAR-125: the verification hook and the answers of `verify` ──────────

#[tokio::test]
#[serial]
async fn send_link_with_the_default_hook_notifies_the_users_address() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(1, "ada@example.com", "x", None, None).await;
    let user = load::<GapUser>("1").await;

    let mail = Mail::fake();
    let notify = Notify::fake();
    EmailVerification::send_link(&user, "https://app.test/verify")
        .await
        .expect("send_link");

    let link = notified_link(&notify, "ada@example.com");
    assert!(link.starts_with("https://app.test/verify?token="), "{link}");
    suprnova::notifications::assert_sent_to_on(
        "ada@example.com",
        "mail",
        "suprnova.auth.verify_email",
    );
    assert_eq!(mail.count(), 0, "the fake records the notification instead");
    let data = &suprnova::notifications::recorded_notifications()[0].data;
    assert!(
        !data.to_string().contains(&token_of(&link)),
        "the public payload never carries the token: {data}"
    );
    drop(notify);

    let _events = EventFacade::fake();
    assert_eq!(verify_as("1", &token_of(&link)).await.expect("verify"), "1");
    assert!(load::<GapUser>("1").await.is_email_verified());
    suprnova::events::testing::assert_dispatched::<EmailVerified>(|event| event.user_id == "1");
}

#[tokio::test]
#[serial]
async fn without_a_bound_dispatcher_the_default_hook_still_mails_through_the_mail_channel() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(1, "ada@example.com", "x", None, None).await;

    let mail = Mail::fake();
    EmailVerification::send_link(&load::<GapUser>("1").await, "https://app.test/verify")
        .await
        .expect("send_link");
    let captured = mail.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].to[0].email, "ada@example.com");
    assert_eq!(captured[0].from.email, "test-mailer@example.test");
    assert!(captured[0].subject.starts_with("Verify your email for "));
    let text = captured[0].text.as_deref().expect("a text body");
    let link = text
        .lines()
        .find(|line| line.contains("token="))
        .expect("the link");
    verify_as("1", &token_of(link))
        .await
        .expect("the mailed link verifies");
}

#[tokio::test]
#[serial]
async fn a_model_that_overrides_the_verification_hook_sends_only_its_own_message() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<HookedUser>::new())).await;
    h.seed(2, "hook-verify@example.com", "x", None, None).await;
    let user = load::<HookedUser>("2").await;

    let mail = Mail::fake();
    let notify = Notify::fake();
    EmailVerification::send_link(&user, "https://app.test/verify")
        .await
        .expect("send_link");
    EmailVerification::resend("hook-verify@example.com", "https://app.test/verify")
        .await
        .expect("resend");
    suprnova::notifications::assert_nothing_sent();
    notify.assert_not_sent_to::<VerifyEmailNotification>("hook-verify@example.com");
    assert_eq!(mail.count(), 0, "the framework sends nothing of its own");

    let links = hooked("verify", "hook-verify@example.com");
    assert_eq!(links.len(), 2, "send_link and resend each reach the hook");
    assert!(
        links
            .iter()
            .all(|link| link.starts_with("https://app.test/verify?token="))
    );
    drop(notify);

    assert_eq!(
        verify_as("2", &token_of(&links[1]))
            .await
            .expect("the resend link verifies"),
        "2"
    );
    assert!(load::<HookedUser>("2").await.is_email_verified());
    let replaced = verify_as("2", &token_of(&links[0]))
        .await
        .expect_err("the older link was spent with its sibling");
    assert_eq!(replaced.status_code(), 400);
}

#[tokio::test]
#[serial]
async fn resend_through_the_eloquent_provider_honours_the_default_hook() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(3, "resend@example.com", "x", None, None).await;

    let notify = Notify::fake();
    EmailVerification::resend("resend@example.com", "https://app.test/verify")
        .await
        .expect("resend");
    let link = notified_link(&notify, "resend@example.com");
    EmailVerification::resend("nobody@example.com", "https://app.test/verify")
        .await
        .expect("an unknown address answers Ok");
    suprnova::notifications::assert_count(1);
    drop(notify);
    verify_as("3", &token_of(&link)).await.expect("verify");
}

#[tokio::test]
#[serial]
async fn resend_through_a_provider_without_a_model_notifies_its_verification_address() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _h = harness(Arc::new(VerificationDirectory)).await;

    let notify = Notify::fake();
    EmailVerification::resend("grace@example.com", "https://app.test/verify")
        .await
        .expect("resend");
    let sent = notify
        .sent::<VerifyEmailNotification>("grace@example.com", |_| true)
        .expect("decode");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].mail.user_name.as_deref(),
        Some("Grace"),
        "the default greets the name of flow_user_by_id"
    );

    // A provider that names no verification address cannot verify the link
    // either; the default says so rather than dropping it.
    let error = EmailVerification::resend("orphan@example.com", "https://app.test/verify")
        .await
        .expect_err("no address to send to");
    assert!(
        error.to_string().contains("verification address"),
        "{error}"
    );
    suprnova::notifications::assert_count(1);
}

#[tokio::test]
#[serial]
async fn verify_answers_403_for_a_live_token_of_another_account() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(1, "one@example.com", "x", None, None).await;
    h.seed(2, "two@example.com", "x", None, None).await;

    let notify = Notify::fake();
    EmailVerification::send_link(&load::<GapUser>("2").await, "https://app.test/verify")
        .await
        .expect("send_link");
    let token = token_of(&notified_link(&notify, "two@example.com"));
    drop(notify);

    let error = verify_as("1", &token)
        .await
        .expect_err("user 1 cannot use user 2's link");
    assert_eq!(error.status_code(), 403);
    assert_eq!(error.to_string(), "This action is unauthorized.");
    assert!(
        EmailVerification::check(&token).await.expect("check"),
        "the refused token stays live"
    );
    assert!(!load::<GapUser>("2").await.is_email_verified());
    assert_eq!(verify_as("2", &token).await.expect("owner verifies"), "2");
}

#[tokio::test]
#[serial]
async fn verify_answers_403_for_a_link_mailed_to_an_address_the_account_left() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(1, "old@example.com", "x", None, None).await;

    let notify = Notify::fake();
    EmailVerification::send_link(&load::<GapUser>("1").await, "https://app.test/verify")
        .await
        .expect("send_link");
    let token = token_of(&notified_link(&notify, "old@example.com"));
    drop(notify);
    h.db.conn()
        .execute_unprepared("UPDATE users SET email = 'new@example.com' WHERE id = 1")
        .await
        .expect("change the address");

    let error = verify_as("1", &token)
        .await
        .expect_err("the link proves the old mailbox only");
    assert_eq!(error.status_code(), 403);
    assert_eq!(error.to_string(), "This action is unauthorized.");
    assert!(EmailVerification::check(&token).await.expect("check"));
    assert!(!load::<GapUser>("1").await.is_email_verified());
}

#[tokio::test]
#[serial]
async fn verify_keeps_400_for_an_unknown_or_consumed_token() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(1, "ada@example.com", "x", None, None).await;

    let error = verify_as("1", "not-a-real-token")
        .await
        .expect_err("unknown token");
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), "invalid or expired verification token");

    let notify = Notify::fake();
    EmailVerification::send_link(&load::<GapUser>("1").await, "https://app.test/verify")
        .await
        .expect("send_link");
    let token = token_of(&notified_link(&notify, "ada@example.com"));
    drop(notify);
    verify_as("1", &token).await.expect("first use");
    let error = verify_as("1", &token).await.expect_err("consumed token");
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), "invalid or expired verification token");
}

#[tokio::test]
#[serial]
async fn verify_of_a_verified_account_consumes_the_token_and_keeps_its_timestamp() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    h.seed(
        1,
        "done@example.com",
        "x",
        Some("2026-10-01T10:00:00Z"),
        None,
    )
    .await;
    let verified_at: DateTime<Utc> = "2026-10-01T10:00:00Z".parse().expect("timestamp");
    assert_eq!(
        load::<GapUser>("1").await.email_verified_at,
        Some(verified_at)
    );

    let notify = Notify::fake();
    EmailVerification::send_link(&load::<GapUser>("1").await, "https://app.test/verify")
        .await
        .expect("send_link");
    let token = token_of(&notified_link(&notify, "done@example.com"));
    drop(notify);

    let _events = EventFacade::fake();
    assert_eq!(verify_as("1", &token).await.expect("verify"), "1");
    assert_eq!(
        h.column(1, "email_verified_at").await.as_deref(),
        Some("2026-10-01T10:00:00Z"),
        "the stored timestamp is not rewritten"
    );
    assert_eq!(
        load::<GapUser>("1").await.email_verified_at,
        Some(verified_at)
    );
    suprnova::events::testing::assert_not_dispatched::<EmailVerified>(|_| true);
    assert!(
        !EmailVerification::check(&token).await.expect("check"),
        "the token is consumed"
    );
}

// ── PAR-126: the reset hook and the timebox ──────────────────────────────

/// The reset link in the one mail `mail` captured.
fn mailed_link(mail: &suprnova::MailFake) -> String {
    let captured = mail.captured();
    assert_eq!(captured.len(), 1, "one reset mail");
    let text = captured[0].text.as_deref().expect("a text body");
    text.lines()
        .find(|line| line.contains("token="))
        .expect("the link")
        .trim()
        .to_owned()
}

#[tokio::test]
#[serial]
async fn a_model_that_overrides_the_reset_hook_sends_only_its_own_message() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<HookedUser>::new())).await;
    let hash = suprnova::hash("old-password").expect("hash");
    h.seed(
        4,
        "hook-reset@example.com",
        &hash,
        Some("2026-10-01T10:00:00Z"),
        None,
    )
    .await;

    let mail = Mail::fake();
    PasswordReset::send_link("hook-reset@example.com", "https://app.test/reset")
        .await
        .expect("send_link");
    assert_eq!(
        mail.count(),
        0,
        "the framework sends no reset mail of its own"
    );
    let links = hooked("reset", "hook-reset@example.com");
    assert_eq!(links.len(), 1, "the hook received the link");
    assert!(links[0].starts_with("https://app.test/reset?token="));

    assert_eq!(
        PasswordReset::complete(&token_of(&links[0]), "new-password")
            .await
            .expect("the hook's link completes the reset"),
        "4"
    );
    let stored = h.column(4, "password").await.expect("a hash");
    assert!(suprnova::hashing::verify("new-password", &stored).expect("verify"));
}

#[tokio::test]
#[serial]
async fn the_default_reset_hook_mails_email_for_reset() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    let hash = suprnova::hash("old-password").expect("hash");
    h.seed(
        5,
        "sign-in@example.com",
        &hash,
        Some("2026-10-01T10:00:00Z"),
        Some("reset-inbox@example.com"),
    )
    .await;

    let mail = Mail::fake();
    PasswordReset::send_link("sign-in@example.com", "https://app.test/reset")
        .await
        .expect("send_link");
    let captured = mail.captured();
    assert_eq!(captured.len(), 1);
    let recipients: Vec<_> = captured[0].to.iter().map(|to| to.email.clone()).collect();
    assert_eq!(recipients, ["reset-inbox@example.com"]);
    assert!(captured[0].subject.starts_with("Reset your "));
    let link = mailed_link(&mail);
    PasswordReset::complete(&token_of(&link), "new-password")
        .await
        .expect("the mailed link completes the reset");
}

#[tokio::test]
#[serial]
async fn the_provider_default_reset_hook_mails_the_address_flow_user_by_id_returns() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(ResetDirectory)).await;

    let mail = Mail::fake();
    PasswordReset::send_link("known@example.com", "https://app.test/reset")
        .await
        .expect("send_link");
    let captured = mail.captured();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].to[0].email, "known@example.com");
    assert!(
        captured[0]
            .text
            .as_deref()
            .is_some_and(|text| text.starts_with("Hi Kim,")),
        "the default greets the provider's name"
    );

    // A reset-capable provider that cannot name the address fails the send
    // rather than dropping the link.
    let error = PasswordReset::send_link("nameless@example.com", "https://app.test/reset")
        .await
        .expect_err("no address to send to");
    assert!(error.to_string().contains("no address"), "{error}");
    assert_eq!(mail.count(), 1);
    assert_eq!(h.token_rows().await, 2, "both links were minted");
}

/// A container scope with `ResetDirectory` as the `users` provider and the
/// in-memory rate limiter: no database, so a paused clock never races I/O.
async fn in_reset_directory<F: Future>(fut: F) -> F::Output {
    TestContainer::scope(async {
        TestContainer::singleton(AuthManager::new(AuthConfig::default()));
        Auth::register_provider("users", Arc::new(ResetDirectory)).expect("provider");
        suprnova::rate_limit::bootstrap_default().await;
        fut.await
    })
    .await
}

#[tokio::test(start_paused = true)]
#[serial]
async fn an_unknown_address_answers_no_sooner_than_the_timebox() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _env = EnvSnapshot::capture(ENV_KEYS);
    set_env("MAIL_FROM", Some("test-mailer@example.test"));
    in_reset_directory(async {
        set_env("PASSWORD_RESET_TIMEBOX_MS", Some("300"));
        let started = tokio::time::Instant::now();
        PasswordReset::send_link("nobody@example.com", "https://app.test/reset")
            .await
            .expect("an unknown address answers Ok");
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "answered after {:?}",
            started.elapsed()
        );

        set_env("PASSWORD_RESET_TIMEBOX_MS", None);
        let started = tokio::time::Instant::now();
        PasswordReset::send_link("nobody-else@example.com", "https://app.test/reset")
            .await
            .expect("an unknown address answers Ok");
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "the default timebox is 200 ms; answered after {:?}",
            started.elapsed()
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
#[serial]
async fn the_abuse_refusal_and_an_error_are_held_for_the_timebox_too() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _env = EnvSnapshot::capture(ENV_KEYS);
    set_env("PASSWORD_RESET_TIMEBOX_MS", Some("300"));
    in_reset_directory(async {
        // An error: `MAIL_FROM` is unset.
        set_env("MAIL_FROM", None);
        let started = tokio::time::Instant::now();
        PasswordReset::send_link("known@example.com", "https://app.test/reset")
            .await
            .expect_err("MAIL_FROM is required");
        assert!(started.elapsed() >= Duration::from_millis(300));

        // The abuse refusal: the address is past its three sends an hour.
        set_env("MAIL_FROM", Some("test-mailer@example.test"));
        for _ in 0..3 {
            PasswordReset::send_link("flood@example.com", "https://app.test/reset")
                .await
                .expect("within the limit");
        }
        let started = tokio::time::Instant::now();
        PasswordReset::send_link("flood@example.com", "https://app.test/reset")
            .await
            .expect_err("over the limit");
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "the refusal answered after {:?}",
            started.elapsed()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_known_address_answers_no_sooner_than_the_timebox() {
    // Real time: the known address mints a token in SQLite, and a paused
    // clock would jump the pool's timers while the query runs.
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    let hash = suprnova::hash("old-password").expect("hash");
    h.seed(
        6,
        "timed@example.com",
        &hash,
        Some("2026-10-01T10:00:00Z"),
        None,
    )
    .await;
    set_env("PASSWORD_RESET_TIMEBOX_MS", Some("300"));

    let mail = Mail::fake();
    let started = std::time::Instant::now();
    PasswordReset::send_link("timed@example.com", "https://app.test/reset")
        .await
        .expect("send_link");
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "answered after {:?}",
        started.elapsed()
    );
    assert_eq!(mail.count(), 1);
}

#[tokio::test]
#[serial]
async fn a_timebox_that_is_not_a_whole_number_fails_before_any_mail() {
    let _lock = crate::env_lock::lock_env_async().await;
    let h = harness(Arc::new(EloquentUserProvider::<GapUser>::new())).await;
    let hash = suprnova::hash("old-password").expect("hash");
    h.seed(
        7,
        "abc@example.com",
        &hash,
        Some("2026-10-01T10:00:00Z"),
        None,
    )
    .await;
    set_env("PASSWORD_RESET_TIMEBOX_MS", Some("abc"));

    let mail = Mail::fake();
    let error = PasswordReset::send_link("abc@example.com", "https://app.test/reset")
        .await
        .expect_err("the configuration is refused");
    assert!(
        matches!(error, FrameworkError::ParamError { .. }),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("PASSWORD_RESET_TIMEBOX_MS"),
        "{error}"
    );
    assert_eq!(mail.count(), 0, "no mail is sent");
    assert_eq!(h.token_rows().await, 0, "no token is minted");
}

// ── PAR-132: validation answers, the sign-in rewrites ───────────────────

/// Turns the shared-Laravel-database setting on or off for one test and
/// hands it back to the environment after.
struct SharedSetting;

impl SharedSetting {
    fn set(shared: bool) -> Self {
        suprnova::LaravelDatabase::share(shared);
        Self
    }
}

impl Drop for SharedSetting {
    fn drop(&mut self) {
        suprnova::LaravelDatabase::follow_environment();
    }
}

/// A `$2b$` hash of `password` at cost 4, a hash Laravel's hasher refuses.
fn hash_2b(password: &str) -> String {
    let hash = bcrypt::hash(password, 4).expect("bcrypt");
    assert!(hash.starts_with("$2b$"), "{hash}");
    hash
}

/// Seed user `id` with a `$2b$` hash of `secret`, and set the bcrypt cost
/// the Laravel rewrite uses to 4 so the test stays fast.
async fn rehash_harness(
    provider: Arc<dyn UserProvider>,
    id: i64,
    email: &str,
) -> (Harness, String) {
    let h = harness(provider).await;
    set_env("HASH_ROUNDS", Some("4"));
    let hash = hash_2b("secret");
    h.seed(id, email, &hash, None, None).await;
    (h, hash)
}

#[tokio::test]
#[serial]
async fn with_the_setting_on_validate_writes_nothing_and_attempt_rewrites_the_hash() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(true);
    for (id, provider) in [
        (
            11,
            Arc::new(EloquentUserProvider::<GapUser>::new()) as Arc<dyn UserProvider>,
        ),
        (12, Arc::new(DatabaseUserProvider::new("users"))),
    ] {
        let email = format!("rehash-{id}@example.com");
        let (h, hash) = rehash_harness(provider, id, &email).await;
        let credentials = Credentials::password(email.as_str(), "secret");

        let valid = in_request(async {
            let facade = Auth::validate(&credentials).await.expect("validate");
            let guard = Auth::guard("web")
                .expect("web guard")
                .validate(&credentials)
                .await
                .expect("guard validate");
            facade && guard
        })
        .await;
        assert!(valid, "{id}: the password is right");
        assert_eq!(
            h.column(id, "password").await.as_deref(),
            Some(hash.as_str()),
            "{id}: validation leaves the stored hash as it is"
        );

        let (user, signed_in) = in_request(async {
            let user = Auth::attempt(&credentials, false).await.expect("attempt");
            (user, Auth::check())
        })
        .await;
        assert!(user.is_some() && signed_in, "{id}: the sign-in succeeds");
        let stored = h.column(id, "password").await.expect("a hash");
        assert!(stored.starts_with("$2y$"), "{id}: rewritten as {stored}");
        assert!(suprnova::hashing::verify("secret", &stored).expect("verify"));
    }
}

#[tokio::test]
#[serial]
async fn with_the_setting_on_once_rewrites_the_hash_before_it_signs_in() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(true);
    let (h, _) = rehash_harness(
        Arc::new(EloquentUserProvider::<GapUser>::new()),
        13,
        "once@example.com",
    )
    .await;
    let credentials = Credentials::password("once@example.com", "secret");
    assert!(in_request(Auth::once(&credentials)).await.expect("once"));
    let stored = h.column(13, "password").await.expect("a hash");
    assert!(stored.starts_with("$2y$"), "rewritten as {stored}");
}

#[tokio::test]
#[serial]
async fn a_rewrite_that_cannot_be_stored_fails_the_sign_in_and_keeps_the_hash() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(true);
    let (h, hash) = rehash_harness(
        Arc::new(EloquentUserProvider::<GapUser>::new()),
        14,
        "refused@example.com",
    )
    .await;
    h.db.conn()
        .execute_unprepared(
            "CREATE TRIGGER refuse_password BEFORE UPDATE OF password ON users \
             BEGIN SELECT RAISE(ABORT, 'the test refuses this password write'); END",
        )
        .await
        .expect("refuse password writes");
    let credentials = Credentials::password("refused@example.com", "secret");

    let (attempt, signed_in) = in_request(async {
        let attempt = Auth::attempt(&credentials, false).await;
        (attempt, Auth::check())
    })
    .await;
    let Err(error) = attempt else {
        panic!("the refused rewrite must fail the sign-in");
    };
    assert!(
        error.to_string().contains("the test refuses"),
        "the database's error: {error}"
    );
    assert!(!signed_in, "nobody is signed in");
    let once = in_request(Auth::once(&credentials)).await;
    assert!(once.is_err(), "once fails the same way");
    assert_eq!(
        h.column(14, "password").await.as_deref(),
        Some(hash.as_str()),
        "the stored hash is as it was"
    );
}

#[tokio::test]
#[serial]
async fn a_rewrite_that_changes_no_row_fails_the_sign_in_and_keeps_the_hash() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(true);
    for (id, provider) in [
        (
            16,
            Arc::new(EloquentUserProvider::<GapUser>::new()) as Arc<dyn UserProvider>,
        ),
        (17, Arc::new(DatabaseUserProvider::new("users"))),
    ] {
        let email = format!("silent-{id}@example.com");
        let (h, hash) = rehash_harness(provider, id, &email).await;
        // SQLite's RAISE(IGNORE) abandons the UPDATE without an error, so
        // the statement reports zero changed rows and the hash stays `$2b$`.
        h.db.conn()
            .execute_unprepared(
                "CREATE TRIGGER keep_hash BEFORE UPDATE OF password ON users \
                 BEGIN SELECT RAISE(IGNORE); END",
            )
            .await
            .expect("ignore password writes");
        let credentials = Credentials::password(email.as_str(), "secret");

        let (attempt, signed_in) = in_request(async {
            let attempt = Auth::attempt(&credentials, false).await;
            (attempt, Auth::check())
        })
        .await;
        assert!(
            attempt.is_err(),
            "{id}: a rewrite that changed no row fails"
        );
        assert!(!signed_in, "{id}: nobody is signed in");
        let once = in_request(Auth::once(&credentials)).await;
        assert!(once.is_err(), "{id}: once fails the same way");
        assert_eq!(
            h.column(id, "password").await.as_deref(),
            Some(hash.as_str()),
            "{id}: the stored hash is as it was"
        );
    }
}

#[tokio::test]
#[serial]
async fn with_the_setting_off_no_check_or_sign_in_writes_a_hash() {
    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(false);
    let (h, hash) = rehash_harness(
        Arc::new(EloquentUserProvider::<GapUser>::new()),
        15,
        "off@example.com",
    )
    .await;
    let credentials = Credentials::password("off@example.com", "secret");
    let (valid, user, once) = in_request(async {
        let valid = Auth::validate(&credentials).await.expect("validate");
        let user = Auth::attempt(&credentials, false).await.expect("attempt");
        let once = Auth::once(&credentials).await.expect("once");
        (valid, user, once)
    })
    .await;
    assert!(valid && user.is_some() && once);
    assert_eq!(
        h.column(15, "password").await.as_deref(),
        Some(hash.as_str())
    );
}

#[tokio::test]
#[serial]
async fn logout_other_devices_keeps_its_rewrite() {
    use sea_orm_migration::SchemaManager;
    use suprnova::session::{DatabaseSessionDriver, SessionData, SessionStore};

    let _lock = crate::env_lock::lock_env_async().await;
    let _shared = SharedSetting::set(true);
    let h = harness(Arc::new(DatabaseUserProvider::new("users"))).await;
    set_env("HASH_ROUNDS", Some("4"));
    let hash = hash_2b("secret");
    h.seed(7, "devices@example.com", &hash, None, None).await;
    let conn = h.db.conn();
    suprnova::session::migrations::create_sessions_table(
        &SchemaManager::new(conn),
        "gap_sessions",
        suprnova::session::migrations::SessionUserKey::Integer,
    )
    .await
    .expect("sessions table");
    conn.execute_unprepared(
        "CREATE TABLE remember_tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         user_id TEXT NOT NULL, selector TEXT NOT NULL, token_hash TEXT NOT NULL, \
         expires_at DATETIME NOT NULL, created_at DATETIME NOT NULL, last_used_at DATETIME)",
    )
    .await
    .expect("remember table");
    let store = Arc::new(
        DatabaseSessionDriver::with_table(Duration::from_secs(3600), "gap_sessions")
            .expect("session table"),
    );
    TestContainer::bind::<dyn SessionStore>(store.clone());
    let mut current = SessionData::new("current-session".into(), "csrf".into());
    current.user_id = Some("7".into());
    current.set_auth_guard_for_test("web", "7", None);
    store.write(&current).await.expect("current stored");

    let slot = Arc::new(Mutex::new(Some(current)));
    suprnova::session::session_scope_for_test(
        slot,
        suprnova::auth::request_state::request_state_scope_for_test(async {
            Auth::logout_other_devices("secret")
                .await
                .expect("logout other devices");
        }),
    )
    .await;
    let stored = h.column(7, "password").await.expect("a hash");
    assert!(stored.starts_with("$2y$"), "rewritten as {stored}");
}
