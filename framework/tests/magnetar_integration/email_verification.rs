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
    Crypt::init(EncryptionKey::generate());
    App::bind::<dyn RateLimiterDriver>(Arc::new(InMemoryRateLimiter::new()));
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
    let connection = Database::connect(options).await.expect("connect SQLite");
    init_magnetar(MagnetarConfig::from_sea_orm(connection.clone()))
        .await
        .expect("install the default Magnetar engine");
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
    App::singleton(DbConnection::from_raw(connection.clone()));
    connection
}

/// Register `label`'s account and sign it in through the JSON-style
/// password sign-in: the account and its bearer token.
async fn account(label: &str) -> (User, String) {
    let email = format!("{label}-{}@example.test", std::process::id());
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
    let email = format!("new-{}@example.test", std::process::id());
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
