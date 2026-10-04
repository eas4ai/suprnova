//! A login on the tables `suprnova new` creates stays signed in, on every
//! database engine, for the tables today's scaffold creates and for the ones
//! older scaffolds created.
//!
//! Older scaffolds created `last_activity`, `expires_at`, `created_at` and
//! `last_used_at` with `.timestamp()`: `TIMESTAMP` on MySQL and MariaDB,
//! `timestamp` on Postgres, text on SQLite. The session driver and the
//! remember-me store read them as `NaiveDateTime`, which the MySQL driver
//! decodes only from `DATETIME`, so on MySQL and MariaDB every session read
//! failed and nobody stayed signed in. Today's scaffold creates them with
//! `.date_time()` (`DATETIME` on MySQL), and its `User` model names the casts
//! for its `.date_time()` columns. Migrations that already ran are not run
//! again, so both schemas must work.
//!
//! The current schema comes from the scaffold's own migration and model
//! templates, so the test cannot drift from what `suprnova new` writes. The
//! test then registers a user through the scaffold's `User::create`, and
//! drives a real login, a session restore and a remember-me restore through
//! `handle_request` and `SessionMiddleware` over a loopback socket.
//!
//! ```bash
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth -- --ignored scaffold_tables::mysql_
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test auth -- --ignored scaffold_tables::postgres_
//! ```

// `#[rustfmt::skip]`: the templates are scaffold output, not workspace
// source, so `cargo fmt` must not rewrite them.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_remember_tokens_table.rs.tpl"]
mod scaffold_remember_tokens;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_sessions_table.rs.tpl"]
mod scaffold_sessions;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/user.rs.tpl"]
mod scaffold_user;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_users_table.rs.tpl"]
mod scaffold_users;

/// The tables older scaffolds created: the same columns, with every time
/// column `.timestamp()`.
mod legacy {
    use sea_orm_migration::prelude::*;

    #[derive(DeriveIden)]
    enum Users {
        Table,
        Id,
        Name,
        Email,
        Password,
        RememberToken,
        EmailVerifiedAt,
        CreatedAt,
        UpdatedAt,
    }

    #[derive(DeriveIden)]
    enum Sessions {
        Table,
        Id,
        UserId,
        Payload,
        CsrfToken,
        LastActivity,
    }

    #[derive(DeriveIden)]
    enum RememberTokens {
        Table,
        Id,
        UserId,
        Selector,
        TokenHash,
        ExpiresAt,
        CreatedAt,
        LastUsedAt,
    }

    pub async fn create(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .col(
                        ColumnDef::new(Users::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Users::Name).string().not_null())
                    .col(
                        ColumnDef::new(Users::Email)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Users::Password).string().not_null())
                    .col(ColumnDef::new(Users::RememberToken).string().null())
                    .col(ColumnDef::new(Users::EmailVerifiedAt).timestamp().null())
                    .col(
                        ColumnDef::new(Users::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Users::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .col(
                        ColumnDef::new(Sessions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Sessions::UserId).string().null())
                    .col(ColumnDef::new(Sessions::Payload).text().not_null())
                    .col(ColumnDef::new(Sessions::CsrfToken).string().not_null())
                    .col(
                        ColumnDef::new(Sessions::LastActivity)
                            .timestamp()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(RememberTokens::Table)
                    .col(
                        ColumnDef::new(RememberTokens::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RememberTokens::UserId).string().not_null())
                    .col(ColumnDef::new(RememberTokens::Selector).string().not_null())
                    .col(
                        ColumnDef::new(RememberTokens::TokenHash)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::LastUsedAt)
                            .timestamp()
                            .null(),
                    )
                    .to_owned(),
            )
            .await
    }
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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Set,
};
use sea_orm_migration::{MigrationTrait, SchemaManager};

use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::http::text;
use suprnova::session::SessionConfig;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, Credentials, DatabaseUserProvider, EloquentUserProvider,
    MiddlewareRegistry, Router, SessionMiddleware, handle_request,
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

/// Which tables a test creates.
#[derive(Clone, Copy, Debug)]
enum Schema {
    /// The scaffold's migrations as `suprnova new` writes them today. The
    /// scaffold's own `User` model is the user provider.
    Current,
    /// The `.timestamp()` tables older scaffolds created. Their `User`
    /// model is the application's own code, so the table-backed
    /// `DatabaseUserProvider` stands in for it.
    Legacy,
}

/// Create the tables exactly as a scaffolded app's migrations do.
async fn create_tables(database: &DbConnection, schema: Schema) {
    let manager = SchemaManager::new(database.inner());
    match schema {
        Schema::Current => {
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
        Schema::Legacy => legacy::create(&manager)
            .await
            .expect("legacy scaffold tables"),
    }
}

/// Register the user the way each schema's application does, and return
/// its id.
async fn register(database: &DbConnection, schema: Schema) -> String {
    match schema {
        Schema::Current => {
            let user = scaffold_user::User::create("Scaffold User", EMAIL, PASSWORD)
                .await
                .expect("register through the scaffold's User model");
            // The rest of the model's surface reads and writes the same
            // row: its own helpers, which save through the casts, and the
            // SeaORM entity it re-exports.
            let found = scaffold_user::User::find_by_email(EMAIL)
                .await
                .expect("find the user by email")
                .expect("the registered user");
            assert!(
                found
                    .verify_password(PASSWORD)
                    .expect("verify the password")
            );
            found
                .update_remember_token(Some("scaffold-token".into()))
                .await
                .expect("save the user through the model");
            let stored = scaffold_user::Entity::find()
                .filter(scaffold_user::Column::Email.eq(EMAIL))
                .one(database.inner())
                .await
                .expect("read the user through the entity")
                .expect("the user row");
            assert_eq!(stored.remember_token.as_deref(), Some("scaffold-token"));
            let mut active: scaffold_user::ActiveModel = stored.into();
            active.remember_token = Set(None);
            active
                .update(database.inner())
                .await
                .expect("update the user through the entity");
            Auth::register_provider(
                "users",
                Arc::new(EloquentUserProvider::<scaffold_user::User>::new()),
            )
            .expect("register users provider");
            user.id.to_string()
        }
        Schema::Legacy => {
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
            Auth::register_provider("users", Arc::new(DatabaseUserProvider::new("users")))
                .expect("register users provider");
            let id: i64 = row.try_get("", "id").expect("user id");
            id.to_string()
        }
    }
}

async fn login_session_and_remember_restore(url: &str, schema: Schema) {
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    let database = connect(url).await;
    drop_scaffold_tables(&database).await;
    create_tables(&database, schema).await;

    let _container = TestContainer::fake();
    TestContainer::singleton(database.clone());
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    let user_id = register(&database, schema).await;

    let addr = serve(4).await;
    let signed_in = format!("user:{user_id}");

    // A real login: credentials checked against `users`, the session row
    // written, a remember-me token issued.
    let login = get(addr, "/login", &[]).await;
    assert_eq!(
        (login.status, login.body.as_str()),
        (200, format!("signed-in:{user_id}").as_str()),
        "{schema:?} login"
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
        "{schema:?}: [session restore, remember-me restore, session after remember-me]"
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

    // Whole rows read through the public entities, whatever type the
    // migration gave the time columns.
    let sessions = suprnova::session::driver::database::sessions::Entity::find()
        .all(database.inner())
        .await;
    let tokens = suprnova::auth::remember::entity::Entity::find()
        .all(database.inner())
        .await;
    assert_eq!(
        format!(
            "{:?} {:?}",
            sessions.as_ref().map(|rows| !rows.is_empty()),
            tokens.as_ref().map(|rows| !rows.is_empty()),
        ),
        "Ok(true) Ok(true)",
        "{schema:?}: whole rows through sessions::Entity and remember::entity::Entity"
    );

    drop_scaffold_tables(&database).await;
    database.inner().clone().close().await.unwrap();
}

/// What each engine names the time columns `schema` creates, so a pass
/// proves the column type the test claims to cover.
async fn assert_time_column_type(url: &str, schema: Schema, expected: &str) {
    let database = connect(url).await;
    drop_scaffold_tables(&database).await;
    create_tables(&database, schema).await;
    let backend = database.inner().get_database_backend();
    for (table, column) in [
        ("users", "created_at"),
        ("sessions", "last_activity"),
        ("remember_tokens", "expires_at"),
    ] {
        let sql = match backend {
            DatabaseBackend::MySql => format!(
                "SELECT DATA_TYPE AS kind FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}' \
                 AND COLUMN_NAME = '{column}'"
            ),
            DatabaseBackend::Postgres => format!(
                "SELECT data_type AS kind FROM information_schema.columns \
                 WHERE table_name = '{table}' AND column_name = '{column}'"
            ),
            _ => unreachable!("only server engines report a column type here"),
        };
        let row = database
            .inner()
            .query_one_raw(sea_orm::Statement::from_string(backend, sql))
            .await
            .expect("read column type")
            .expect("column exists");
        let kind: String = row.try_get("", "kind").expect("column type");
        assert_eq!(
            kind.to_ascii_lowercase(),
            expected,
            "{schema:?} {table}.{column}"
        );
    }
    drop_scaffold_tables(&database).await;
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
async fn sqlite_scaffold_tables_keep_a_login_signed_in() {
    login_session_and_remember_restore("sqlite::memory:", Schema::Current).await;
    login_session_and_remember_restore("sqlite::memory:", Schema::Legacy).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_scaffold_tables_keep_a_login_signed_in() {
    let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
    assert_time_column_type(&url, Schema::Current, "datetime").await;
    login_session_and_remember_restore(&url, Schema::Current).await;
    assert_time_column_type(&url, Schema::Legacy, "timestamp").await;
    login_session_and_remember_restore(&url, Schema::Legacy).await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_scaffold_tables_keep_a_login_signed_in() {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
    for schema in [Schema::Current, Schema::Legacy] {
        assert_time_column_type(&url, schema, "timestamp without time zone").await;
        login_session_and_remember_restore(&url, schema).await;
    }
}
