//! A login on the tables `suprnova new` creates stays signed in, on every
//! database engine.
//!
//! The scaffold's migrations create `last_activity`, `expires_at`,
//! `created_at` and `last_used_at` with `.timestamp()`: `TIMESTAMP` on MySQL
//! and MariaDB, `timestamp` on Postgres, text on SQLite. The session driver
//! and the remember-me store once read them as `NaiveDateTime`, which the
//! MySQL driver decodes only from `DATETIME`, so on MySQL and MariaDB every
//! session read failed and nobody stayed signed in.
//!
//! These tests run the scaffold's own migration bodies (included from the
//! CLI's templates, so they cannot drift from what `suprnova new` writes),
//! then drive a real login, a session restore and a remember-me restore
//! through `handle_request` and `SessionMiddleware` over a loopback socket.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth -- --ignored scaffold_tables::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test auth -- --ignored scaffold_tables::postgres_
//! ```

// `include!` rather than `#[path]`: rustfmt follows `#[path]` modules and
// would reformat the templates, which are scaffold output, not workspace
// source.
mod scaffold_remember_tokens {
    include!(
        "../../../suprnova-cli/src/templates/files/backend/migrations/create_remember_tokens_table.rs.tpl"
    );
}
mod scaffold_sessions {
    include!(
        "../../../suprnova-cli/src/templates/files/backend/migrations/create_sessions_table.rs.tpl"
    );
}
mod scaffold_users {
    include!(
        "../../../suprnova-cli/src/templates/files/backend/migrations/create_users_table.rs.tpl"
    );
}

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::sea_query::{Alias, Query};
use sea_orm::{ConnectionTrait, DatabaseBackend};
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::http::text;
use suprnova::session::SessionConfig;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, Credentials, DatabaseUserProvider, MiddlewareRegistry, Router,
    SessionMiddleware, handle_request,
};

const EMAIL: &str = "scaffold@example.test";
const PASSWORD: &str = "correct horse battery staple";

/// What one request returned: the status, the body, and every
/// `name=value` pair the response set (a cookie cleared with `Max-Age=0`
/// is left out, as a browser would drop it).
struct Reply {
    status: u16,
    body: String,
    cookies: Vec<(String, String)>,
}

impl Reply {
    fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(cookie, _)| cookie == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Send a `GET` carrying `cookies` and collect every `Set-Cookie`.
async fn get(addr: SocketAddr, path: &str, cookies: &[(&str, &str)]) -> Reply {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut builder = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost");
    if !cookies.is_empty() {
        let header = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        builder = builder.header("Cookie", header);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        sender.send_request(builder.body(Full::new(Bytes::new())).unwrap()),
    )
    .await
    .expect("request timed out")
    .expect("send request");
    let (parts, body) = response.into_parts();
    let cookies = parts
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|header| !header.contains("Max-Age=0"))
        .filter_map(|header| {
            let pair = header.split(';').next()?;
            let (name, value) = pair.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect();
    let body = body.collect().await.unwrap().to_bytes();
    Reply {
        status: parts.status.as_u16(),
        body: String::from_utf8_lossy(&body).into_owned(),
        cookies,
    }
}

/// `/login` signs the scaffold's user in with "remember me"; `/me` says
/// who the request belongs to.
fn router() -> Router {
    Router::new()
        .get("/login", |_req| async {
            let outcome = match Auth::attempt(&Credentials::password(EMAIL, PASSWORD), true).await {
                Ok(Some(user)) => format!("signed-in:{}", user.get_auth_identifier()),
                Ok(None) => "rejected".to_string(),
                Err(error) => format!("error:{error}"),
            };
            text(outcome)
        })
        .get("/me", |_req| async {
            let outcome = match Auth::user().await {
                Ok(Some(user)) => format!("user:{}", user.get_auth_identifier()),
                Ok(None) => "guest".to_string(),
                Err(error) => format!("error:{error}"),
            };
            text(outcome)
        })
        .into()
}

/// Serve [`router`] behind the session middleware on an ephemeral port,
/// for `accepts` connections. The tasks run on the test's own thread, so
/// they see its container overrides.
async fn serve(accepts: usize) -> SocketAddr {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let router = Arc::new(router());
    let middleware = Arc::new(MiddlewareRegistry::new().append(SessionMiddleware::new(config)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

async fn connect(url: &str) -> DbConnection {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    DbConnection::connect(&config)
        .await
        .expect("connect test database")
}

async fn drop_scaffold_tables(database: &DbConnection) {
    for table in ["remember_tokens", "sessions", "users"] {
        database
            .inner()
            .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
            .await
            .expect("drop scaffold table");
    }
}

/// Create the tables exactly as a scaffolded app's migrations do.
async fn run_scaffold_migrations(database: &DbConnection) {
    let manager = SchemaManager::new(database.inner());
    scaffold_users::Migration
        .up(&manager)
        .await
        .expect("scaffold users migration");
    scaffold_sessions::Migration
        .up(&manager)
        .await
        .expect("scaffold sessions migration");
    scaffold_remember_tokens::Migration
        .up(&manager)
        .await
        .expect("scaffold remember_tokens migration");
}

/// The scaffold's `users` row, written with only the columns a
/// registration sets: the timestamps take the migration's defaults.
async fn insert_user(database: &DbConnection) -> String {
    let hash = suprnova::hashing::hash(PASSWORD).expect("hash password");
    let insert = Query::insert()
        .into_table(Alias::new("users"))
        .columns([
            Alias::new("name"),
            Alias::new("email"),
            Alias::new("password"),
        ])
        .values_panic(["Scaffold User".into(), EMAIL.into(), hash.into()])
        .to_owned();
    database
        .inner()
        .execute(&insert)
        .await
        .expect("insert user");
    let select = Query::select()
        .column(Alias::new("id"))
        .from(Alias::new("users"))
        .to_owned();
    let row = database
        .inner()
        .query_one(&select)
        .await
        .expect("read user id")
        .expect("user row");
    let id: i64 = row.try_get("", "id").expect("user id");
    id.to_string()
}

async fn login_session_and_remember_restore(url: &str) {
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    let database = connect(url).await;
    drop_scaffold_tables(&database).await;
    run_scaffold_migrations(&database).await;
    let user_id = insert_user(&database).await;

    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(DatabaseUserProvider::new("users")))
        .expect("register users provider");

    let addr = serve(4).await;
    let signed_in = format!("user:{user_id}");

    // A real login: credentials checked against `users`, the session row
    // written, a remember-me token issued.
    let login = get(addr, "/login", &[]).await;
    assert_eq!(
        (login.status, login.body.as_str()),
        (200, format!("signed-in:{user_id}").as_str())
    );
    let session = login
        .cookie("suprnova_session")
        .expect("login sets the session cookie")
        .to_string();
    let remember = login
        .cookie("remember_me")
        .expect("login sets the remember-me cookie")
        .to_string();

    // The next request restores the session from `sessions`.
    let restored = get(addr, "/me", &[("suprnova_session", &session)]).await;

    // With the session gone, the remember-me cookie signs the user back in
    // from `remember_tokens` and rotates the token. That restore starts a
    // new session, which is itself restorable.
    let remembered = get(addr, "/me", &[("remember_me", &remember)]).await;
    let after = match remembered.cookie("suprnova_session") {
        Some(rotated) => {
            get(addr, "/me", &[("suprnova_session", rotated)])
                .await
                .body
        }
        None => "no session cookie after the remember-me restore".to_string(),
    };

    // One assertion over all three, so a failure shows every restore.
    assert_eq!(
        [
            restored.body.as_str(),
            remembered.body.as_str(),
            after.as_str()
        ],
        [signed_in.as_str(); 3],
        "[session restore, remember-me restore, session after remember-me]"
    );

    // A remember-me lifetime that reaches past 2038-01-19, the end of
    // MySQL's `TIMESTAMP` range, still issues and verifies a token.
    let twenty_years = 20 * 365 * 24 * 60;
    let token = suprnova::auth::remember::issue(&user_id, twenty_years)
        .await
        .expect("issue a remember-me token that outlives 2038");
    let verified = suprnova::auth::remember::verify_and_rotate(&token, twenty_years)
        .await
        .expect("verify the long-lived token");
    assert_eq!(verified.map(|(owner, _)| owner), Some(user_id.clone()));

    drop_scaffold_tables(&database).await;
    database.inner().clone().close().await.unwrap();
}

/// What each engine names the scaffold's time columns, so a pass proves
/// the column type the test claims to cover.
async fn assert_time_column_type(url: &str, expected: &str) {
    let database = connect(url).await;
    drop_scaffold_tables(&database).await;
    run_scaffold_migrations(&database).await;
    let sql = match database.inner().get_database_backend() {
        DatabaseBackend::MySql => {
            "SELECT DATA_TYPE AS kind FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'sessions' \
             AND COLUMN_NAME = 'last_activity'"
        }
        DatabaseBackend::Postgres => {
            "SELECT data_type AS kind FROM information_schema.columns \
             WHERE table_name = 'sessions' AND column_name = 'last_activity'"
        }
        _ => unreachable!("only server engines report a column type here"),
    };
    let row = database
        .inner()
        .query_one_raw(sea_orm::Statement::from_string(
            database.inner().get_database_backend(),
            sql,
        ))
        .await
        .expect("read column type")
        .expect("column exists");
    let kind: String = row.try_get("", "kind").expect("column type");
    assert_eq!(kind.to_ascii_lowercase(), expected);
    drop_scaffold_tables(&database).await;
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_scaffold_tables_keep_a_login_signed_in() {
    login_session_and_remember_restore("sqlite::memory:").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_scaffold_tables_keep_a_login_signed_in() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    assert_time_column_type(&url, "timestamp").await;
    login_session_and_remember_restore(&url).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_scaffold_tables_keep_a_login_signed_in() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    assert_time_column_type(&url, "timestamp without time zone").await;
    login_session_and_remember_restore(&url).await;
}
