//! LDB-007: Suprnova leaves `users.remember_token` as Laravel left it. A
//! sign-in with and without remember-me, a sign-in through Suprnova's
//! remember-me cookie, a sign-out and a revocation of the user's sessions,
//! through the framework and the scaffold and through Magnetar, write
//! nothing to the column.
//!
//! The scaffold's login is `Auth::attempt` over its `User` model, which is
//! what the router below calls; with Magnetar's engine installed, the same
//! calls go through Magnetar's sessions and remember credentials.

use suprnova::{Auth, Credentials, FrameworkError, HttpResponse, Request, Response, Router};

use crate::browser::{self, Browser, SESSION_COOKIE};
use crate::on_every_engine;
use crate::support::{self, Engine};

const EMAIL: &str = "taylor@example.com";

fn failure(error: FrameworkError) -> Response {
    Err(HttpResponse::text(error.to_string()).status(error.status_code()))
}

fn router() -> Router {
    Router::new()
        .get("/login", |request: Request| async move {
            let credentials = Credentials::password(
                browser::header(&request, "x-email"),
                browser::header(&request, "x-password"),
            );
            let remember = browser::header(&request, "x-remember") == "1";
            match Auth::attempt(&credentials, remember).await {
                Ok(Some(user)) => Ok(HttpResponse::text(user.get_auth_identifier())),
                Ok(None) => Ok(HttpResponse::text("invalid credentials").status(401)),
                Err(error) => failure(error),
            }
        })
        .get("/logout", |_request: Request| async {
            match Auth::logout().await {
                Ok(()) => Ok(HttpResponse::text("signed out")),
                Err(error) => failure(error),
            }
        })
        .get("/logout-everywhere", |_request: Request| async {
            match Auth::logout_and_invalidate().await {
                Ok(()) => Ok(HttpResponse::text("signed out")),
                Err(error) => failure(error),
            }
        })
        .get("/whoami", |_request: Request| async {
            Ok(HttpResponse::text(
                Auth::id().unwrap_or_else(|| "guest".to_owned()),
            ))
        })
        .into()
}

/// The `remember_token` Laravel stored for taylor.
async fn remember_token(db: &support::Db) -> String {
    let rows = support::rows(
        &db.conn,
        &format!("SELECT remember_token FROM users WHERE email = '{EMAIL}'"),
    )
    .await;
    support::text(&rows[0], "remember_token")
}

async fn sign_in(browser: &mut Browser, password: &str, remember: bool) {
    let remember = if remember { "1" } else { "0" };
    let (status, body) = browser
        .get(
            "/login",
            &[
                ("x-email", EMAIL),
                ("x-password", password),
                ("x-remember", remember),
            ],
        )
        .await;
    assert_eq!((status, body.as_str()), (200, "1"), "the sign-in failed");
}

async fn whoami(browser: &mut Browser) -> String {
    let (status, body) = browser.get("/whoami", &[]).await;
    assert_eq!(status, 200, "{body}");
    body
}

/// Every flow LDB-007 names, with a check of the column after each.
async fn flows_leave_the_remember_token(db: &support::Db, password: &str, engine: Engine) {
    let laravel_token = remember_token(db).await;
    assert!(
        !laravel_token.is_empty(),
        "the fixture stores a remember_token"
    );
    let unchanged = |after: String, step: &str| {
        assert_eq!(
            after, laravel_token,
            "{engine:?}: {step} wrote users.remember_token"
        );
    };

    // A sign-in without remember-me, then a sign-out.
    let mut plain = Browser::serve(router()).await;
    sign_in(&mut plain, password, false).await;
    assert_eq!(whoami(&mut plain).await, "1");
    unchanged(remember_token(db).await, "a sign-in without remember-me");
    plain.get("/logout", &[]).await;
    assert_eq!(whoami(&mut plain).await, "guest");
    unchanged(remember_token(db).await, "a sign-out");

    // A sign-in with remember-me, then a sign-in through its cookie alone.
    let mut remembered = Browser::serve(router()).await;
    sign_in(&mut remembered, password, true).await;
    assert!(
        remembered.cookie("remember_me").is_some(),
        "{engine:?}: no remember-me cookie was issued"
    );
    unchanged(remember_token(db).await, "a sign-in with remember-me");
    remembered.forget(SESSION_COOKIE);
    assert_eq!(
        whoami(&mut remembered).await,
        "1",
        "{engine:?}: the remember-me cookie did not sign in"
    );
    unchanged(
        remember_token(db).await,
        "a sign-in through the remember-me cookie",
    );

    // A revocation of the user's sessions, then a sign-out everywhere.
    suprnova::session::destroy_all_for_user("1")
        .await
        .expect("revoke the user's sessions");
    unchanged(
        remember_token(db).await,
        "a revocation of the user's sessions",
    );
    let mut everywhere = Browser::serve(router()).await;
    sign_in(&mut everywhere, password, true).await;
    everywhere.get("/logout-everywhere", &[]).await;
    assert_eq!(whoami(&mut everywhere).await, "guest");
    unchanged(remember_token(db).await, "a sign-out everywhere");
}

/// The framework's own auth, as the scaffold wires it.
async fn framework_leaves_the_remember_token(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    support::install_scaffold_auth().await;
    flows_leave_the_remember_token(&db, &fixture.users[EMAIL].password, engine).await;
}

on_every_engine!(framework_leaves_the_remember_token =>
    ldb_007_framework_sign_ins_leave_remember_token_sqlite,
    ldb_007_framework_sign_ins_leave_remember_token_postgres,
    ldb_007_framework_sign_ins_leave_remember_token_mysql);

/// The same flows with Magnetar's engine installed: its sessions back
/// every sign-in, its remember credentials back remember-me, and the
/// Laravel users are its accounts under their Laravel ids.
async fn magnetar_leaves_the_remember_token(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    support::install_scaffold_auth().await;
    suprnova::init_magnetar(suprnova::MagnetarConfig::from_sea_orm(db.conn.clone()))
        .await
        .expect("init_magnetar");
    for (email, user) in &fixture.users {
        let hash = support::stored_hash(&db.conn, email).await;
        sea_orm::ConnectionTrait::execute_unprepared(
            &db.conn,
            &format!(
                "INSERT INTO app_users (id, email, name, password_hash, auth_epoch) \
                 VALUES ({}, '{email}', 'Imported', '{hash}', 0)",
                user.id
            ),
        )
        .await
        .expect("import a Laravel user into Magnetar");
    }
    flows_leave_the_remember_token(&db, &fixture.users[EMAIL].password, engine).await;
    // The flows ran through Magnetar: its sessions backed the sign-ins, and
    // the framework's own remember-me table was never written.
    assert!(support::count(&db.conn, "auth_sessions", "user_id = 1").await > 0);
    assert_eq!(support::count(&db.conn, "remember_tokens", "").await, 0);
}

#[test]
#[serial_test::serial]
fn ldb_007_magnetar_sign_ins_leave_remember_token_sqlite() {
    support::alone(
        "remember::ldb_007_magnetar_sign_ins_leave_remember_token_sqlite",
        || magnetar_leaves_the_remember_token(Engine::Sqlite),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway Postgres at PG_TEST_URL"]
fn ldb_007_magnetar_sign_ins_leave_remember_token_postgres() {
    support::alone(
        "remember::ldb_007_magnetar_sign_ins_leave_remember_token_postgres",
        || magnetar_leaves_the_remember_token(Engine::Postgres),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
fn ldb_007_magnetar_sign_ins_leave_remember_token_mysql() {
    support::alone(
        "remember::ldb_007_magnetar_sign_ins_leave_remember_token_mysql",
        || magnetar_leaves_the_remember_token(Engine::Mysql),
    );
}
