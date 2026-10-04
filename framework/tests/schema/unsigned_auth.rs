//! A user keyed the way Laravel keys `users.id`, `BIGINT UNSIGNED`, signs
//! in on every database (PAR-044 follow-ups).
//!
//! The session and the remember-me token store the user's id as text, and
//! the user providers turn it back into a lookup by key. On MySQL the id
//! reaches `u64::MAX`; on Postgres and SQLite no row holds an id above
//! `i64::MAX`, and a lookup by one answers "no such user".
//!
//! Driven through `handle_request` on a loopback socket with the real
//! `SessionMiddleware`, a database session store and the remember-me table,
//! for both the Eloquent and the table-backed provider.

use std::any::Any;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::http::{HttpResponse, text};
use suprnova::schema::Schema;
use suprnova::session::{DatabaseSessionDriver, SessionConfig, SessionMiddleware};
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, Authenticatable, CanResetPassword, Credentials,
    DatabaseUserProvider, DbConnection, EloquentUserProvider, EncryptionKey, MiddlewareRegistry,
    Model, MustVerifyEmail, Request, Response, Router, UserProvider, attrs, handle_request,
    handler, model,
};

use super::cases::drop_tables;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

/// A users table as Laravel's `id()` makes it on MySQL.
#[model(table = "ua_users", fillable = ["email", "password"])]
pub struct UaUser {
    pub id: u64,
    pub email: String,
    pub password: String,
}

impl Authenticatable for UaUser {
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

impl MustVerifyEmail for UaUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        None
    }
    fn set_email_verified_at(&mut self, _verified_at: Option<DateTime<Utc>>) {}
}

impl CanResetPassword for UaUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_owned();
    }
}

const SESSION_COOKIE: &str = "ua_session";
const REMEMBER_COOKIE: &str = "remember_me";
const TABLES: &[&str] = &["ua_users", "ua_sessions", "remember_tokens"];

/// Signs in with the `email` and `password` query parameters, asking to be
/// remembered, and answers the user's id.
#[handler]
pub async fn login(req: Request) -> Response {
    let params = req.query_params();
    let email = params.get("email").cloned().unwrap_or_default();
    let password = params.get("password").cloned().unwrap_or_default();
    match Auth::attempt(&Credentials::password(email, password), true).await? {
        Some(user) => text(user.get_auth_identifier()),
        None => Ok(HttpResponse::text("bad credentials").status(401)),
    }
}

/// Signs in the user with the id in the path, or answers "no such user".
#[handler]
pub async fn login_as(id: String) -> Response {
    match Auth::login_using_id(&id, false).await? {
        Some(user) => text(user.get_auth_identifier()),
        None => Ok(HttpResponse::text("no such user").status(404)),
    }
}

/// The signed-in user's id, restored from the session or the remember-me
/// cookie.
#[handler]
pub async fn me() -> Response {
    match Auth::user().await? {
        Some(user) => text(user.get_auth_identifier()),
        None => Ok(HttpResponse::text("guest").status(401)),
    }
}

/// Creates the users table, the session table and the remember-me table.
///
/// The session and remember-me stores read their times as a naive
/// date-time, which the MySQL driver reads from `DATETIME` only, so the
/// time columns are date-times rather than `TIMESTAMP`s.
async fn create_tables(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, TABLES).await;
    Schema::create(&manager, "ua_users", |t| {
        t.unsigned_id();
        t.string("email");
        t.string("password");
    })
    .await
    .expect("create ua_users");
    manager
        .create_table(
            Table::create()
                .table(Alias::new("ua_sessions"))
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alias::new("user_id")).string().null())
                .col(ColumnDef::new(Alias::new("payload")).text().not_null())
                .col(ColumnDef::new(Alias::new("csrf_token")).string().not_null())
                .col(
                    ColumnDef::new(Alias::new("last_activity"))
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .to_owned(),
        )
        .await
        .expect("create ua_sessions");
    manager
        .create_table(
            Table::create()
                .table(Alias::new("remember_tokens"))
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alias::new("user_id")).string().not_null())
                .col(ColumnDef::new(Alias::new("selector")).string().not_null())
                .col(ColumnDef::new(Alias::new("token_hash")).string().not_null())
                .col(
                    ColumnDef::new(Alias::new("expires_at"))
                        .date_time()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Alias::new("created_at"))
                        .date_time()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Alias::new("last_used_at"))
                        .date_time()
                        .null(),
                )
                .to_owned(),
        )
        .await
        .expect("create remember_tokens");
}

/// Serves `router` behind the real session middleware, on a loopback socket.
/// Each connection runs on this test's thread, where the test container
/// holds the connection and the auth manager.
async fn serve(router: Router) -> SocketAddr {
    let store = DatabaseSessionDriver::with_table(Duration::from_secs(3600), "ua_sessions")
        .expect("ua_sessions is a valid table name");
    // The store names the session table; the config's table only reaches
    // a store the middleware builds itself.
    let config = SessionConfig::new().cookie_name(SESSION_COOKIE);
    let session = SessionMiddleware::with_store(config, Arc::new(store));
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new().append(session));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("the listener's address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// A response: the status, the body, and each cookie it sets as
/// `(name, value)`.
struct Reply {
    status: u16,
    body: String,
    cookies: Vec<(String, String)>,
}

impl Reply {
    /// The value of the cookie `name` sets, if it sets one with a value.
    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies
            .iter()
            .find(|(cookie, value)| cookie == name && !value.is_empty())
            .map(|(_, value)| value.clone())
    }
}

/// Sends `GET path` with the given cookies.
async fn get(addr: SocketAddr, path: &str, cookies: &[(&str, &str)]) -> Reply {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to the test server");
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost");
    if !cookies.is_empty() {
        let header = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        request = request.header("Cookie", header);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        sender.send_request(request.body(Full::new(Bytes::new())).expect("a request")),
    )
    .await
    .expect("the server answers in time")
    .expect("a response");
    let status = response.status().as_u16();
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| {
            let pair = value.split(';').next()?;
            let (name, value) = pair.split_once('=')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    Reply {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
        cookies,
    }
}

/// Which user provider the flow runs against.
#[derive(Clone, Copy, Debug)]
enum Provider {
    Eloquent,
    Table,
}

impl Provider {
    fn build(self) -> Arc<dyn UserProvider> {
        match self {
            Self::Eloquent => Arc::new(EloquentUserProvider::<UaUser>::new()),
            Self::Table => Arc::new(DatabaseUserProvider::new("ua_users")),
        }
    }
}

/// A user signs in, is restored from the session and then from the
/// remember-me cookie alone, with each provider. On MySQL the user's id is
/// `u64::MAX`, and the id one below it is no user; on Postgres and SQLite
/// the id is small and an id above `i64::MAX` is "no such user", from the
/// provider and through the route that signs in by id. It fails while the
/// providers bind an id above `i64::MAX` as text, which Postgres refuses to
/// compare with the key.
pub async fn a_u64_keyed_user_signs_in_on_every_database(conn: &DatabaseConnection) {
    let _first =
        suprnova::testing::install_test_encryption_keyring(EncryptionKey::generate(), Vec::new());
    let backend = conn.get_database_backend();
    for provider in [Provider::Eloquent, Provider::Table] {
        create_tables(conn).await;
        let _guard = TestContainer::fake();
        TestContainer::singleton(DbConnection::from_raw(conn.clone()));
        TestContainer::singleton(AuthManager::new(AuthConfig::default()));
        Auth::register_provider("users", provider.build()).expect("register the provider");

        let hash = suprnova::hashing::hash_async("secret")
            .await
            .expect("hash the password");
        let id = if backend == DbBackend::MySql {
            conn.execute_raw(Statement::from_sql_and_values(
                backend,
                "INSERT INTO ua_users (id, email, password) VALUES (?, ?, ?)",
                [
                    sea_orm::Value::BigUnsigned(Some(u64::MAX)),
                    "ada".into(),
                    hash.clone().into(),
                ],
            ))
            .await
            .expect("insert a user keyed at u64::MAX");
            u64::MAX
        } else {
            UaUser::create(attrs! { email: "ada", password: hash })
                .await
                .expect("create a user")
                .id
        };
        let id = id.to_string();

        let router: Router = Router::new()
            .get("/login", login)
            .get("/login-as/{id}", login_as)
            .get("/me", me)
            .into();
        let addr = serve(router).await;

        let signed_in = get(addr, "/login?email=ada&password=secret", &[]).await;
        assert_eq!(
            (signed_in.status, signed_in.body.as_str()),
            (200, id.as_str()),
            "{provider:?}: sign in"
        );
        let session = signed_in
            .cookie(SESSION_COOKIE)
            .expect("the session cookie");
        let remember = signed_in
            .cookie(REMEMBER_COOKIE)
            .expect("the remember-me cookie");

        let restored = get(addr, "/me", &[(SESSION_COOKIE, &session)]).await;
        assert_eq!(
            (restored.status, restored.body.as_str()),
            (200, id.as_str()),
            "{provider:?}: restored from the session"
        );
        let remembered = get(addr, "/me", &[(REMEMBER_COOKIE, &remember)]).await;
        assert_eq!(
            (remembered.status, remembered.body.as_str()),
            (200, id.as_str()),
            "{provider:?}: restored from the remember-me cookie"
        );
        assert_eq!(
            get(addr, "/me", &[]).await.status,
            401,
            "{provider:?}: a guest"
        );

        let beyond = (i64::MAX as u64 + 1).to_string();
        let (present, absent) = if backend == DbBackend::MySql {
            (Some(u64::MAX.to_string()), (u64::MAX - 1).to_string())
        } else {
            (None, beyond.clone())
        };
        if let Some(present) = present {
            let found = get(addr, &format!("/login-as/{present}"), &[]).await;
            assert_eq!(
                (found.status, found.body.as_str()),
                (200, present.as_str()),
                "{provider:?}: sign in by an id above i64::MAX"
            );
        }
        let missing = get(addr, &format!("/login-as/{absent}"), &[]).await;
        assert_eq!(
            (missing.status, missing.body.as_str()),
            (404, "no such user"),
            "{provider:?}: an id no user has"
        );
        assert!(
            provider
                .build()
                .retrieve_by_id(&absent)
                .await
                .expect("a lookup by an id no user has is not an error")
                .is_none(),
            "{provider:?}: no such user"
        );
        if backend != DbBackend::MySql {
            assert!(
                provider
                    .build()
                    .retrieve_by_id(&beyond)
                    .await
                    .expect("an id above i64::MAX is not an error")
                    .is_none(),
                "{provider:?}: no row holds an id above i64::MAX"
            );
        }
    }

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
#[serial]
async fn sqlite_a_u64_keyed_user_signs_in_on_every_database() {
    a_u64_keyed_user_signs_in_on_every_database(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_a_u64_keyed_user_signs_in_on_every_database() {
    a_u64_keyed_user_signs_in_on_every_database(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_a_u64_keyed_user_signs_in_on_every_database() {
    a_u64_keyed_user_signs_in_on_every_database(&connect_mysql().await).await;
}
