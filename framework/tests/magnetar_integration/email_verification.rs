//! `Auth::email_verification()`: a link that proves the signed-in user
//! holds their address, through the installed Magnetar engine.
//!
//! The link proves the mailbox only to the account's own signed-in user.
//! A link that verified whoever opened it would let someone who registered
//! another person's address, with a password of their own, get it verified
//! by that person's click. So `verify` asks for the signed-in user, by
//! bearer token or by session, and leaves every credential and session as
//! it was, unlike Magnetar's first-proof sign-ins.
//!
//! Each test runs alone in its own process: it installs the process-wide
//! Magnetar engine over a database of its own.
//!
//! The race of one link presented many times at once also runs on live
//! engines, where each request holds a pool connection of its own. Those
//! twins are ignored by default and share the database they are given, so
//! every account has an address no other run used:
//!
//! ```bash
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test magnetar_integration -- --ignored _on_postgres
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test magnetar_integration -- --ignored _on_mysql
//! ```

#![cfg(all(feature = "testing", feature = "database-sqlite"))]

use std::sync::Arc;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use serde_json::{Value, json};
use suprnova::database::DbConnection;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::session::SessionConfig;
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    App, Auth, BearerTokenMiddleware, Crypt, EmailVerificationIssue, EmailVerificationOutcome,
    EncryptionKey, HttpResponse, MagnetarConfig, MiddlewareRegistry, RateLimiterDriver, Request,
    Router, SessionMiddleware, User, init_magnetar,
};

const PASSWORD: &str = "correct-horse-battery";

/// Install the default Magnetar engine over a fresh SQLite database, the
/// framework session table and a rate limiter that counts; the database.
async fn setup() -> DatabaseConnection {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "magnetar-email-verification-{}.sqlite",
        std::process::id()
    ));
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.clone().into_os_string();
        file.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(file));
    }
    let mut options = ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
    options.max_connections(1).min_connections(1);
    let connection = install(options).await;
    connection
        .execute_unprepared(
            "CREATE TABLE sessions (\
                id TEXT PRIMARY KEY, \
                user_id INTEGER NULL, \
                ip_address TEXT NULL, \
                user_agent TEXT NULL, \
                payload TEXT NOT NULL, \
                last_activity INTEGER NOT NULL\
             )",
        )
        .await
        .expect("the sessions table");
    connection
}

/// Install the default Magnetar engine over the database `options` open,
/// with a rate limiter that counts; the connection. Magnetar creates or
/// upgrades its own tables, so a live database shared with other tests is
/// fine: every account here has an address of its own.
async fn install(options: ConnectOptions) -> DatabaseConnection {
    Crypt::init(EncryptionKey::generate());
    App::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
    let connection = Database::connect(options).await.expect("connect");
    init_magnetar(MagnetarConfig::from_sea_orm(connection.clone()))
        .await
        .expect("install the default Magnetar engine");
    App::singleton(DbConnection::from_raw(connection.clone()));
    connection
}

/// An address no other run has used, so repeated runs against one live
/// database never meet an account of an earlier run.
fn unique(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    format!("{label}-{}-{nanos}@example.test", std::process::id())
}

/// Register `label`'s account and sign it in through the JSON-style
/// password sign-in: the account and its bearer token.
async fn account(label: &str) -> (User, String) {
    let email = unique(label);
    let user = Auth::password()
        .register(&email, PASSWORD)
        .await
        .expect("register")
        .created()
        .expect("a new account");
    let (_, session) = Auth::password()
        .authenticate(&email, PASSWORD, None, None)
        .await
        .expect("sign in");
    let token = session
        .token
        .expect("a bearer token")
        .expose_secret()
        .to_owned();
    (user, token)
}

/// The name a test reads an outcome by.
fn outcome_name(outcome: &EmailVerificationOutcome) -> &'static str {
    match outcome {
        EmailVerificationOutcome::Verified => "verified",
        EmailVerificationOutcome::AlreadyVerified => "already_verified",
        EmailVerificationOutcome::OtherAccount => "other_account",
        EmailVerificationOutcome::AddressChanged => "address_changed",
        EmailVerificationOutcome::Expired => "expired",
        EmailVerificationOutcome::Unknown => "unknown",
        EmailVerificationOutcome::SignIn => "sign_in",
        _ => "unexpected",
    }
}

/// The routes: issue a link, verify one, and a sign-in that starts a
/// browser session.
fn client() -> TestClient {
    let router: Router = Router::new()
        .get("/issue", |_request: Request| async move {
            let issued = Auth::email_verification()
                .issue()
                .await
                .map_err(HttpResponse::from)?;
            let body = match issued {
                EmailVerificationIssue::Issued(link) => {
                    json!({ "outcome": "issued", "email": link.email, "token": link.token })
                }
                EmailVerificationIssue::AlreadyVerified => json!({ "outcome": "already_verified" }),
                EmailVerificationIssue::SignIn => json!({ "outcome": "sign_in" }),
                _ => json!({ "outcome": "unexpected" }),
            };
            Ok(HttpResponse::json(body))
        })
        .get("/verify", |request: Request| async move {
            let token = request.query_param("token").unwrap_or_default();
            let outcome = Auth::email_verification()
                .verify(&token)
                .await
                .map_err(HttpResponse::from)?;
            Ok(HttpResponse::json(
                json!({ "outcome": outcome_name(&outcome) }),
            ))
        })
        .post("/sign-in", |request: Request| async move {
            let email = request.query_param("email").unwrap_or_default();
            Auth::password()
                .authenticate_outcome(&email, PASSWORD, None, None)
                .await
                .map_err(HttpResponse::from)?;
            Ok(HttpResponse::text("signed in"))
        })
        .into();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::new(config))
        .append(BearerTokenMiddleware);
    TestClient::new(router, registry).timeout(std::time::Duration::from_secs(60))
}

/// A client that sends `token` as its bearer token.
fn bearer(token: &str) -> TestClient {
    client().with_header("Authorization", format!("Bearer {token}"))
}

/// A client of a JSON API that sends `token` as its bearer token. The
/// session middleware records no previous page for its requests, so it
/// needs no session table: the live-database tests create none.
fn api_client(token: &str) -> TestClient {
    bearer(token).with_header("Accept", "application/json")
}

/// The JSON answer of `response`, which must be a 200.
fn answer(response: &TestResponse) -> Value {
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

/// Issue a link for the client's user: its token.
async fn issue(client: &TestClient) -> String {
    let issued = answer(&client.get("/issue").send().await);
    assert_eq!(issued["outcome"], "issued", "{issued}");
    issued["token"].as_str().expect("a token").to_owned()
}

/// Present `token` as the client's user: the outcome's name.
async fn verify(client: &TestClient, token: &str) -> String {
    let path = format!(
        "/verify?token={}",
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
    );
    let verified = answer(&client.get(path).send().await);
    verified["outcome"].as_str().expect("an outcome").to_owned()
}

/// One column of the account's `app_users` row, as text.
async fn user_column(db: &DatabaseConnection, user: &User, column: &str) -> Option<String> {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        format!("SELECT {column} FROM app_users WHERE id = {}", user.id),
    ))
    .await
    .expect("read the account")
    .expect("the account")
    .try_get_by_index::<Option<String>>(0)
    .expect("a text column")
}

/// The rows of `table` that belong to the account.
async fn rows(db: &DatabaseConnection, table: &str, user: &User) -> i64 {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        format!("SELECT COUNT(*) FROM {table} WHERE user_id = '{}'", user.id),
    ))
    .await
    .expect("count")
    .expect("a row")
    .try_get_by_index::<i64>(0)
    .expect("a number")
}

#[tokio::test]
async fn email_verification_stamps_the_address_and_changes_no_credential() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_stamps_the_address_and_changes_no_credential",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("stamp").await;
    // What a first-proof sign-in would remove: a passkey, a linked
    // provider account and a second factor.
    for statement in [
        format!(
            "INSERT INTO auth_methods (user_id, credential_id, public_key, created_at) \
             VALUES ({}, 'credential', 'public-key', CURRENT_TIMESTAMP)",
            user.id
        ),
        format!(
            "INSERT INTO auth_linked_accounts \
             (user_id, provider, provider_account_id, created_at, updated_at) \
             VALUES ({}, 'github', 'octocat', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
            user.id
        ),
        format!(
            "INSERT INTO auth_two_factor \
             (user_id, secret, enrollment_auth_epoch, rotation_pending, confirmed_at) \
             VALUES ('{}', X'00', 0, 0, CURRENT_TIMESTAMP)",
            user.id
        ),
    ] {
        db.execute_unprepared(&statement).await.expect("seed");
    }
    let password_hash = user_column(&db, &user, "password_hash").await;
    assert!(password_hash.is_some());
    let sessions = suprnova::magnetar_integration::list_sessions(&user.id.to_string())
        .await
        .expect("list sessions");
    assert_eq!(sessions.len(), 1, "the bearer sign-in");

    let client = bearer(&token);
    let link = client.get("/issue").send().await;
    let issued = answer(&link);
    assert_eq!(issued["outcome"], "issued", "{issued}");
    assert_eq!(issued["email"], user.email.as_str());
    let verified = client
        .get(format!(
            "/verify?token={}",
            url::form_urlencoded::byte_serialize(
                issued["token"].as_str().expect("a token").as_bytes()
            )
            .collect::<String>()
        ))
        // A JSON request, which the session middleware records as no
        // previous page: any cookie would be a session the verification
        // started.
        .header("Accept", "application/json")
        .send()
        .await;
    assert_eq!(answer(&verified)["outcome"], "verified");
    assert!(
        verified.header("set-cookie").is_none(),
        "verification starts no session"
    );

    assert!(user_column(&db, &user, "email_verified_at").await.is_some());
    assert_eq!(
        user_column(&db, &user, "password_hash").await,
        password_hash
    );
    for table in ["auth_methods", "auth_linked_accounts", "auth_two_factor"] {
        assert_eq!(rows(&db, table, &user).await, 1, "{table} is kept");
    }
    let after = suprnova::magnetar_integration::list_sessions(&user.id.to_string())
        .await
        .expect("list sessions");
    assert_eq!(
        after.iter().map(|s| &s.session_id).collect::<Vec<_>>(),
        sessions.iter().map(|s| &s.session_id).collect::<Vec<_>>(),
        "no session issued or revoked"
    );
    // The bearer token still signs the user in, and the address reads as
    // verified.
    assert_eq!(
        answer(&client.get("/issue").send().await)["outcome"],
        "already_verified"
    );
}

#[tokio::test]
async fn email_verification_refuses_another_users_token_and_keeps_it() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_refuses_another_users_token_and_keeps_it",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (owner, owner_token) = account("owner").await;
    let (_, other_token) = account("other").await;
    let link = issue(&bearer(&owner_token)).await;
    assert_eq!(verify(&bearer(&other_token), &link).await, "other_account");
    assert!(
        user_column(&db, &owner, "email_verified_at")
            .await
            .is_none()
    );
    assert_eq!(verify(&bearer(&owner_token), &link).await, "verified");
}

#[tokio::test]
async fn email_verification_tokens_are_single_use() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_tokens_are_single_use",
    )
    .await
    {
        return;
    }
    setup().await;
    let (_, token) = account("single").await;
    let client = bearer(&token);
    let link = issue(&client).await;
    assert_eq!(verify(&client, &link).await, "verified");
    assert_eq!(verify(&client, &link).await, "unknown");
    assert_eq!(verify(&client, "not-a-token").await, "unknown");
}

#[tokio::test]
async fn email_verification_refuses_an_expired_token() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_refuses_an_expired_token",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("expired").await;
    let client = bearer(&token);
    let link = issue(&client).await;
    db.execute_unprepared(&format!(
        "UPDATE auth_tokens SET expires_at = datetime('now', '-1 day') \
         WHERE purpose = 'email-verification' AND user_id = {}",
        user.id
    ))
    .await
    .expect("expire the token");
    assert_eq!(verify(&client, &link).await, "expired");
    assert!(user_column(&db, &user, "email_verified_at").await.is_none());
}

#[tokio::test]
async fn email_verification_asks_a_guest_to_sign_in() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_asks_a_guest_to_sign_in",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("guest").await;
    let link = issue(&bearer(&token)).await;
    let guest = client();
    assert_eq!(
        answer(&guest.get("/issue").send().await)["outcome"],
        "sign_in"
    );
    assert_eq!(verify(&guest, &link).await, "sign_in");
    assert!(user_column(&db, &user, "email_verified_at").await.is_none());
    // The link is still the owner's to use.
    assert_eq!(verify(&bearer(&token), &link).await, "verified");
}

#[tokio::test]
async fn email_verification_works_for_a_session_caller() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_works_for_a_session_caller",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, _) = account("session").await;
    let browser = client();
    let signed_in = browser
        .post(format!("/sign-in?email={}", user.email))
        .send()
        .await;
    assert_eq!(signed_in.status(), 200, "{}", signed_in.body_text());
    let link = issue(&browser).await;
    assert_eq!(verify(&browser, &link).await, "verified");
    assert!(user_column(&db, &user, "email_verified_at").await.is_some());
}

#[tokio::test]
async fn email_verification_for_a_new_account_needs_no_sign_in() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_for_a_new_account_needs_no_sign_in",
    )
    .await
    {
        return;
    }
    setup().await;
    let email = unique("new");
    let user = Auth::password()
        .register(&email, PASSWORD)
        .await
        .expect("register")
        .created()
        .expect("a new account");
    let EmailVerificationIssue::Issued(link) = Auth::email_verification()
        .issue_for(&user)
        .await
        .expect("issue")
    else {
        panic!("a link for the new account");
    };
    assert_eq!(link.email, email);
    let (_, session) = Auth::password()
        .authenticate(&email, PASSWORD, None, None)
        .await
        .expect("sign in");
    let token = session.token.expect("a token").expose_secret().to_owned();
    assert_eq!(verify(&bearer(&token), &link.token).await, "verified");
}

#[tokio::test]
async fn email_verification_issuing_is_limited_per_address() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_issuing_is_limited_per_address",
    )
    .await
    {
        return;
    }
    setup().await;
    let (_, token) = account("limited").await;
    let client = bearer(&token);
    for _ in 0..3 {
        issue(&client).await;
    }
    let refused = client.get("/issue").send().await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
}

/// One column of the account's newest email-verification token, as text.
async fn token_column(db: &DatabaseConnection, user: &User, column: &str) -> Option<String> {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        format!(
            "SELECT {column} FROM auth_tokens WHERE user_id = {} ORDER BY id DESC LIMIT 1",
            user.id
        ),
    ))
    .await
    .expect("read the token")
    .expect("the token")
    .try_get_by_index::<Option<String>>(0)
    .expect("a text column")
}

/// A link mailed to an address proves that address. When the account's
/// address changes after the link went out, the link no longer proves the
/// address the account holds, so it verifies nothing.
#[tokio::test]
async fn email_verification_refuses_a_token_after_the_address_changed() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_refuses_a_token_after_the_address_changed",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("moved").await;
    let client = bearer(&token);
    let link = issue(&client).await;
    let moved = unique("moved-elsewhere");
    db.execute_unprepared(&format!(
        "UPDATE app_users SET email = '{moved}' WHERE id = {}",
        user.id
    ))
    .await
    .expect("change the address");
    assert_eq!(verify(&client, &link).await, "address_changed");
    assert!(
        user_column(&db, &user, "email_verified_at").await.is_none(),
        "nothing is stamped"
    );
    // A link for the address the account holds now verifies it.
    let fresh = issue(&client).await;
    assert_eq!(verify(&client, &fresh).await, "verified");
    assert!(user_column(&db, &user, "email_verified_at").await.is_some());
}

/// Verifying with one link spends every other live link of the account,
/// so an older mail cannot be used later.
#[tokio::test]
async fn email_verification_with_one_link_voids_the_others() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_with_one_link_voids_the_others",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("siblings").await;
    let client = bearer(&token);
    let first = issue(&client).await;
    let second = issue(&client).await;
    assert_eq!(verify(&client, &second).await, "verified");
    assert_eq!(verify(&client, &first).await, "unknown");
    let unused: i64 = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            format!(
                "SELECT COUNT(*) FROM auth_tokens WHERE user_id = {} AND used_at IS NULL",
                user.id
            ),
        ))
        .await
        .expect("count")
        .expect("a row")
        .try_get_by_index(0)
        .expect("a number");
    assert_eq!(unused, 0, "no live link is left");
}

/// A link for an address that was verified in the meantime is spent and
/// answers `AlreadyVerified`; the first verification time stays.
#[tokio::test]
async fn email_verification_of_a_verified_address_keeps_its_first_time() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_of_a_verified_address_keeps_its_first_time",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("verified-before").await;
    let client = bearer(&token);
    let link = issue(&client).await;
    db.execute_unprepared(&format!(
        "UPDATE app_users SET email_verified_at = '2020-01-02T03:04:05+00:00' WHERE id = {}",
        user.id
    ))
    .await
    .expect("verify the address another way");
    let first_time = user_column(&db, &user, "email_verified_at").await;
    assert_eq!(verify(&client, &link).await, "already_verified");
    assert_eq!(
        user_column(&db, &user, "email_verified_at").await,
        first_time
    );
    assert_eq!(verify(&client, &link).await, "unknown", "the link is spent");
}

/// A token minted for another purpose, such as a password reset, verifies
/// nothing and is left unused.
#[tokio::test]
async fn email_verification_refuses_a_token_of_another_purpose() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_refuses_a_token_of_another_purpose",
    )
    .await
    {
        return;
    }
    let db = setup().await;
    let (user, token) = account("purpose").await;
    let client = bearer(&token);
    let link = issue(&client).await;
    db.execute_unprepared(&format!(
        "UPDATE auth_tokens SET purpose = 'password-reset' \
         WHERE purpose = 'email-verification' AND user_id = {}",
        user.id
    ))
    .await
    .expect("make the token a reset token");
    assert_eq!(verify(&client, &link).await, "unknown");
    assert!(user_column(&db, &user, "email_verified_at").await.is_none());
    assert!(
        token_column(&db, &user, "used_at").await.is_none(),
        "the other purpose's token is not spent"
    );
}

/// Present one link from `count` requests at once: each outcome's name.
async fn verify_at_once(client: &TestClient, token: &str, count: usize) -> Vec<String> {
    let path = format!(
        "/verify?token={}",
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
    );
    let answers =
        futures::future::join_all((0..count).map(|_| client.get(path.as_str()).send())).await;
    answers
        .iter()
        .map(|response| {
            answer(response)["outcome"]
                .as_str()
                .expect("an outcome")
                .to_owned()
        })
        .collect()
}

/// Eight requests present one link at once: one verifies and the others
/// find it spent. The address then reads as verified.
async fn concurrent_verifies_spend_the_token_once() {
    let (_, token) = account("race").await;
    let client = api_client(&token);
    let issued = answer(&client.get("/issue").send().await);
    assert_eq!(issued["outcome"], "issued", "{issued}");
    let link = issued["token"].as_str().expect("a token").to_owned();
    let outcomes = verify_at_once(&client, &link, 8).await;
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| *outcome == "verified")
            .count(),
        1,
        "{outcomes:?}"
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome == "verified" || outcome == "unknown"),
        "{outcomes:?}"
    );
    assert_eq!(
        answer(&client.get("/issue").send().await)["outcome"],
        "already_verified"
    );
}

#[tokio::test]
async fn email_verification_concurrent_verifies_spend_the_token_once() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_concurrent_verifies_spend_the_token_once",
    )
    .await
    {
        return;
    }
    setup().await;
    concurrent_verifies_spend_the_token_once().await;
}

/// The race on a live engine, where each request takes a pool connection
/// of its own and the single spend rests on the database's conditional
/// update.
async fn concurrent_verifies_on(url: &str) {
    let mut options = ConnectOptions::new(url.to_owned());
    options.max_connections(8).min_connections(1);
    install(options).await;
    concurrent_verifies_spend_the_token_once().await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn email_verification_concurrent_verifies_spend_the_token_once_on_postgres() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_concurrent_verifies_spend_the_token_once_on_postgres",
    )
    .await
    {
        return;
    }
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    concurrent_verifies_on(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn email_verification_concurrent_verifies_spend_the_token_once_on_mysql() {
    if crate::own_process_async::delegate(
        module_path!(),
        "email_verification_concurrent_verifies_spend_the_token_once_on_mysql",
    )
    .await
    {
        return;
    }
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    concurrent_verifies_on(&url).await;
}
