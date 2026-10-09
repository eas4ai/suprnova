//! `EmailVerification` facade integration tests - provider-backed.
//!
//! Exercises the facade end-to-end against a real `#[suprnova::model]` user in
//! in-memory SQLite + the framework's own `auth_flow_tokens` table, with the
//! configured [`EloquentUserProvider`] as the active "users" provider. No
//! `init_magnetar`: the facade mints tokens through the provider-agnostic
//! `TokenStore` and marks users verified through the provider.
//!
//! # Serial execution
//!
//! `Mail::fake()` swaps the process-global mail transport, so two parallel
//! tests installing fakes would cross-capture each other's messages. The DB is
//! thread-local (per `TestDatabase`), so the mail fake is the only remaining
//! global - `#[serial]` serializes against it.

use std::any::Any;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serial_test::serial;

use suprnova::auth::AuthConfig;
use suprnova::auth_flows::EmailVerification;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::container::testing::TestContainer;
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, AuthManager, Authenticatable, CanResetPassword, EloquentUserProvider, MustVerifyEmail,
    UserProvider, model,
};

// The app's `User` shape: a typed model that is also Authenticatable +
// MustVerifyEmail. `email_verified_at` is a nullable datetime; the model macro
// auto-injects `AsOptionalDateTime` on `Option<DateTime<Utc>>` fields.
#[model(table = "users", fillable = ["email", "password"])]
pub struct TestUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl Authenticatable for TestUser {
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

impl MustVerifyEmail for TestUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
        self.email_verified_at = v;
    }
    fn name(&self) -> Option<&str> {
        None
    }
}

impl CanResetPassword for TestUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

/// Held-for-the-test guard: the `TestDatabase` carries the thread-local
/// container scope (it installs one via `TestContainer::fake` internally and
/// registers the `DbConnection` in it). We register the `AuthManager` +
/// provider into that SAME container - without replacing it - so the facade's
/// `active_user_provider()` and `DB::connection()` both resolve.
struct Harness {
    _db: TestDatabase,
}

/// Fresh in-memory DB with a `users` table, the `auth_flow_tokens` table, and
/// an `EloquentUserProvider::<TestUser>` registered as the active "users"
/// provider. Seeds one user (`ada@x.com`, not yet verified). Also sets
/// `MAIL_FROM` (the facade fails closed without it).
async fn setup() -> Harness {
    use sea_orm::ConnectionTrait;

    // SAFETY: every test in this file is `#[serial]`; no parallel observer.
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.com");
    }

    let db = TestDatabase::sqlite_memory().await.expect("sqlite_memory");
    let conn = db.conn();
    conn.execute_unprepared(
        "CREATE TABLE users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            email_verified_at TEXT\
         )",
    )
    .await
    .expect("create users table");

    let create = create_auth_flow_tokens_table();
    conn.execute(&create)
        .await
        .expect("create auth_flow_tokens table");

    let hash = suprnova::hash("secret").expect("hash");
    conn.execute_unprepared(&format!(
        "INSERT INTO users (email, password) VALUES ('ada@x.com', '{hash}')"
    ))
    .await
    .expect("seed user");

    // Register the Eloquent provider as the active "users" provider into the
    // SAME thread-local container `TestDatabase` already installed (do NOT call
    // `TestContainer::fake()` again - that would replace the container and drop
    // the DB binding). `TestContainer::singleton` writes into the active scope.
    // `AuthConfig::default()`'s "web" guard points at the "users" provider.
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<TestUser>::new()))
        .expect("register provider");
    suprnova::rate_limit::bootstrap_default().await;

    Harness { _db: db }
}

/// Reload the seeded user from the DB by email - to assert the verification
/// stamp persisted through the provider.
async fn reload_ada() -> TestUser {
    let p = EloquentUserProvider::<TestUser>::new();
    let flow = p
        .retrieve_by_email("ada@x.com")
        .await
        .expect("lookup")
        .expect("ada exists");
    let user = p
        .retrieve_by_id(&flow.id)
        .await
        .expect("by id")
        .expect("ada exists");
    user.as_any()
        .downcast_ref::<TestUser>()
        .expect("TestUser")
        .clone()
}

#[tokio::test]
#[serial]
async fn send_link_then_verify_marks_verified_single_use() {
    let _env = crate::env_lock::lock_env_async().await;
    let _h = setup().await;

    // Look the user up so `send_link` has a `MustVerifyEmail` to mint against.
    let p = EloquentUserProvider::<TestUser>::new();
    let user = p
        .retrieve_by_id("1")
        .await
        .expect("by id")
        .expect("ada exists");
    let user = user
        .as_any()
        .downcast_ref::<TestUser>()
        .expect("TestUser")
        .clone();

    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(&user, "https://app.test/verify-email/verify")
        .await
        .expect("send_link");
    fake.assert_sent_to("ada@x.com");

    // Pull the plaintext token out of the captured mail's rendered link. The
    // text body renders the URL verbatim (the HTML body HTML-escapes slashes).
    let captured = fake.captured();
    assert_eq!(captured.len(), 1, "exactly one verification mail");
    let text = captured[0]
        .text
        .as_deref()
        .expect("verification mail has a text body");
    let link = text
        .lines()
        .find(|l| l.contains("token="))
        .expect("a line with the token link");
    let token = link
        .rsplit("token=")
        .next()
        .expect("token after marker")
        .trim();

    // Not yet verified.
    assert!(!reload_ada().await.is_email_verified());

    // A signed link alone is not enough: logged-out verification fails without
    // consuming the token.
    assert!(EmailVerification::verify(token).await.is_err());
    assert!(EmailVerification::check(token).await.unwrap());

    let slot = suprnova::session::new_session_slot_for_test();
    let (id, replay) = suprnova::session::session_scope_for_test(slot, async {
        suprnova::session::set_auth_user("1");
        let id = EmailVerification::verify(token).await;
        let replay = EmailVerification::verify(token).await;
        (id, replay)
    })
    .await;
    assert_eq!(
        id.expect("authenticated verify"),
        user.get_auth_identifier()
    );
    assert!(
        reload_ada().await.is_email_verified(),
        "verify() must persist email_verified_at through the provider"
    );
    assert!(replay.is_err(), "a consumed token must not verify again");
}

#[tokio::test]
#[serial]
async fn check_reports_validity_without_consuming() {
    let _env = crate::env_lock::lock_env_async().await;
    let _h = setup().await;

    let p = EloquentUserProvider::<TestUser>::new();
    let user = p
        .retrieve_by_id("1")
        .await
        .expect("by id")
        .expect("ada exists");
    let user = user
        .as_any()
        .downcast_ref::<TestUser>()
        .expect("TestUser")
        .clone();

    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(&user, "https://app.test/verify")
        .await
        .expect("send_link");

    let captured = fake.captured();
    let text = captured[0].text.as_deref().expect("text body");
    let link = text
        .lines()
        .find(|l| l.contains("token="))
        .expect("token link");
    let token = link.rsplit("token=").next().expect("token").trim();

    // check() is non-consuming: true before, still true after, and verify
    // still works afterwards.
    assert!(EmailVerification::check(token).await.expect("check"));
    assert!(EmailVerification::check(token).await.expect("check again"));
    let slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(slot, async {
        suprnova::session::set_auth_user("1");
        EmailVerification::verify(token).await.expect("verify");
    })
    .await;
    // Spent now.
    assert!(!EmailVerification::check(token).await.expect("check spent"));
}

#[tokio::test]
#[serial]
async fn resend_sends_for_known_email_and_is_silent_for_unknown() {
    let _env = crate::env_lock::lock_env_async().await;
    let _h = setup().await;

    // Known email → a mail is sent.
    {
        let fake = suprnova::mail::Mail::fake();
        EmailVerification::resend("ada@x.com", "https://app.test/verify")
            .await
            .expect("resend known");
        assert_eq!(fake.count(), 1, "known email must trigger a mail");
        fake.assert_sent_to("ada@x.com");
    }

    // Unknown email → anti-enumeration: nothing sent, still Ok.
    {
        let fake = suprnova::mail::Mail::fake();
        EmailVerification::resend("nobody@x.com", "https://app.test/verify")
            .await
            .expect("resend unknown returns Ok (no leak)");
        assert_eq!(
            fake.count(),
            0,
            "unknown email must not send any mail (anti-enumeration)"
        );
    }
}

#[tokio::test]
#[serial]
async fn verify_rejects_garbage_token() {
    let _env = crate::env_lock::lock_env_async().await;
    let _h = setup().await;
    assert!(
        EmailVerification::verify("not-a-real-token").await.is_err(),
        "an unknown token must be rejected"
    );
}

/// Reload user 1 by id: the tests below change its address, so a lookup by
/// email would miss it.
async fn reload_user_one() -> TestUser {
    let user = EloquentUserProvider::<TestUser>::new()
        .retrieve_by_id("1")
        .await
        .expect("by id")
        .expect("user 1 exists");
    user.as_any()
        .downcast_ref::<TestUser>()
        .expect("TestUser")
        .clone()
}

/// Verify `token` as signed-in user 1.
async fn verify_as_user_one(token: &str) -> Result<String, suprnova::FrameworkError> {
    let slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(slot, async {
        suprnova::session::set_auth_user("1");
        EmailVerification::verify(token).await
    })
    .await
}

/// IDENTITY-023: a verification link proves ownership of the mailbox it was
/// sent to, and of no other. The account's address changes to an unproven
/// mailbox while the link is live; redeeming the link must not mark the new
/// mailbox verified. Restoring the address shows the link itself is intact.
#[tokio::test]
#[serial]
async fn a_link_sent_to_one_mailbox_never_verifies_another() {
    use sea_orm::ConnectionTrait;

    let _env = crate::env_lock::lock_env_async().await;
    let h = setup().await;
    let user = reload_user_one().await;

    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(&user, "https://app.test/verify")
        .await
        .expect("send_link");
    fake.assert_sent_to("ada@x.com");
    let captured = fake.captured();
    let text = captured[0].text.as_deref().expect("text body");
    let link = text
        .lines()
        .find(|l| l.contains("token="))
        .expect("token link");
    let token = link.rsplit("token=").next().expect("token").trim();

    h._db
        .conn()
        .execute_unprepared("UPDATE users SET email = 'unproven@x.com' WHERE id = 1")
        .await
        .expect("change the address");
    assert!(
        verify_as_user_one(token).await.is_err(),
        "a link sent to ada@x.com must not verify unproven@x.com"
    );
    assert!(!reload_user_one().await.is_email_verified());

    h._db
        .conn()
        .execute_unprepared("UPDATE users SET email = 'ada@x.com' WHERE id = 1")
        .await
        .expect("restore the address");
    verify_as_user_one(token)
        .await
        .expect("the link verifies the mailbox it was sent to");
    assert!(reload_user_one().await.is_email_verified());
}

/// IDENTITY-023: the address can change after `verify` read the mailbox and
/// before it stamps the verification. A trigger on the token's consumption
/// lands that change exactly there. The link proved `grace@x.com` only, so
/// the account, now at `unproven@x.com`, must stay unverified.
///
/// A user of its own, because `send_link` allows three mails an hour per
/// address and the tests above spend ada's.
#[tokio::test]
#[serial]
async fn an_address_changed_while_verify_runs_is_never_marked_verified() {
    use sea_orm::ConnectionTrait;

    let _env = crate::env_lock::lock_env_async().await;
    let h = setup().await;
    let hash = suprnova::hash("secret").expect("hash");
    h._db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO users (id, email, password) VALUES (2, 'grace@x.com', '{hash}')"
        ))
        .await
        .expect("seed grace");
    let reload_grace = || async {
        let user = EloquentUserProvider::<TestUser>::new()
            .retrieve_by_id("2")
            .await
            .expect("by id")
            .expect("grace exists");
        user.as_any()
            .downcast_ref::<TestUser>()
            .expect("TestUser")
            .clone()
    };

    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(&reload_grace().await, "https://app.test/verify")
        .await
        .expect("send_link");
    let captured = fake.captured();
    let text = captured[0].text.as_deref().expect("text body");
    let link = text
        .lines()
        .find(|l| l.contains("token="))
        .expect("token link");
    let token = link.rsplit("token=").next().expect("token").trim();

    h._db
        .conn()
        .execute_unprepared(
            "CREATE TRIGGER address_changes_mid_verify AFTER UPDATE ON auth_flow_tokens \
             BEGIN UPDATE users SET email = 'unproven@x.com' WHERE id = 2; END",
        )
        .await
        .expect("create the trigger");
    let slot = suprnova::session::new_session_slot_for_test();
    let outcome = suprnova::session::session_scope_for_test(slot, async {
        suprnova::session::set_auth_user("2");
        EmailVerification::verify(token).await
    })
    .await;

    let grace = reload_grace().await;
    assert_eq!(
        grace.email, "unproven@x.com",
        "the trigger changed the address while verify ran"
    );
    assert!(
        !grace.is_email_verified(),
        "a link sent to grace@x.com must not verify unproven@x.com, however the change \
         interleaves - verify returned {outcome:?}"
    );
    assert!(
        outcome.is_err(),
        "verify must report that nothing was verified"
    );
}

/// Verify `token` behind `AuthMiddleware::new().for_guard("admin")`, with
/// `web` signed in on the default guard and `admin` on the `admin` guard.
async fn verify_behind_admin_guard(
    token: &str,
    web: TestUser,
    admin: TestUser,
) -> Result<String, suprnova::FrameworkError> {
    use std::sync::Mutex;
    use suprnova::{AuthMiddleware, HttpResponse, Middleware, Next, Request};

    let outcome = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&outcome);
    let token = token.to_owned();
    let next: Next = Arc::new(move |_request| {
        let seen = Arc::clone(&seen);
        let token = token.clone();
        Box::pin(async move {
            let result = EmailVerification::verify(&token).await;
            *seen.lock().unwrap() = Some(result);
            Ok(HttpResponse::text("done"))
        })
    });
    let slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(
        slot,
        suprnova::auth::request_state::request_state_scope_for_test(async move {
            Auth::guard("web").unwrap().set_user(Arc::new(web)).await;
            Auth::guard("admin")
                .unwrap()
                .set_user(Arc::new(admin))
                .await;
            let _ = AuthMiddleware::new()
                .for_guard("admin")
                .handle(Request::for_test("GET", "/verify"), next)
                .await;
        }),
    )
    .await;
    let result = outcome.lock().unwrap().take();
    result.expect("the admin route reached its handler")
}

/// Load the user whose address is `email`.
async fn user_by_email(email: &str) -> TestUser {
    let provider = EloquentUserProvider::<TestUser>::new();
    let flow = provider
        .retrieve_by_email(email)
        .await
        .expect("lookup")
        .expect("user exists");
    provider
        .retrieve_by_id(&flow.id)
        .await
        .expect("by id")
        .expect("user exists")
        .as_any()
        .downcast_ref::<TestUser>()
        .expect("TestUser")
        .clone()
}

/// `verify` checks the token against the user of the route's guard, through
/// that guard's provider. Behind the `admin` guard, the default guard's user
/// in the same session never stands in for the admin: a link for the owner
/// does not verify while another user is the route's user, and it verifies
/// when the owner is the route's user.
#[tokio::test]
#[serial]
async fn verify_behind_a_second_guard_checks_that_guards_user() {
    use sea_orm::ConnectionTrait;

    let _env = crate::env_lock::lock_env_async().await;
    let h = setup().await;
    let hash = suprnova::hash("secret").expect("hash");
    for email in ["route-owner@x.com", "route-other@x.com"] {
        h._db
            .conn()
            .execute_unprepared(&format!(
                "INSERT INTO users (email, password) VALUES ('{email}', '{hash}')"
            ))
            .await
            .expect("seed user");
    }
    let config = AuthConfig::new("web").guard("admin", suprnova::GuardConfig::session("admins"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<TestUser>::new()))
        .expect("users provider");
    Auth::register_provider("admins", Arc::new(EloquentUserProvider::<TestUser>::new()))
        .expect("admins provider");
    let owner = user_by_email("route-owner@x.com").await;
    let other = user_by_email("route-other@x.com").await;

    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(&owner, "https://app.test/verify")
        .await
        .expect("send_link");
    let captured = fake.captured();
    let text = captured[0].text.as_deref().expect("text body");
    let link = text
        .lines()
        .find(|l| l.contains("token="))
        .expect("token link");
    let token = link.rsplit("token=").next().expect("token").trim();

    assert!(
        verify_behind_admin_guard(token, owner.clone(), other.clone())
            .await
            .is_err(),
        "the route's user is the other admin; the owner on the web guard must not verify \
         through it"
    );
    assert!(!user_by_email("route-owner@x.com").await.is_email_verified());

    verify_behind_admin_guard(token, other, owner)
        .await
        .expect("the route's user, the owner, verifies her own link");
    assert!(user_by_email("route-owner@x.com").await.is_email_verified());
}
