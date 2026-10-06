//! BIND-002 and BIND-005: what a binding matches and what a miss answers.
//! The lookup `#[model]` generates goes through the model's query and its
//! connection, parses the value as the column's type (and a `unique_id`
//! key's format), and answers a value that does not parse exactly as it
//! answers a value that matches no row: 404, naming the model, never
//! repeating the value. `#[model(route_key = "...")]` sets the column.

use suprnova::database::{ConnectionRegistry, EntityExt};
use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{DbConnection, Model, Response, RouteParam, Router, attrs, handler, model};

use super::{get, message, run_sql, serve};

#[model(table = "ky_posts", fillable = ["slug", "title"])]
pub struct KyPost {
    pub id: i64,
    pub slug: String,
    pub title: String,
}

impl EntityExt for ky_post::Entity {}

#[model(table = "ky_pages", route_key = "slug", fillable = ["slug", "title"])]
pub struct KyPage {
    pub id: i64,
    pub slug: String,
    pub title: String,
}

#[model(table = "ky_tokens", unique_id = "uuid", fillable = ["label"])]
pub struct KyToken {
    pub id: String,
    pub label: String,
}

#[model(table = "ky_archives", connection = "ky_archive", fillable = ["title"])]
pub struct KyArchive {
    pub id: i64,
    pub title: String,
}

#[handler]
pub async fn show_post(post: KyPost) -> Response {
    text(post.title)
}

#[handler]
pub async fn show_wrapped(post: RouteParam<KyPost>) -> Response {
    text(post.title.clone())
}

#[handler]
pub async fn show_bare(post: ky_post::Model) -> Response {
    text(post.title)
}

#[handler]
pub async fn show_page(page: KyPage) -> Response {
    text(page.title)
}

#[handler]
pub async fn show_token(token: KyToken) -> Response {
    text(token.label)
}

#[handler]
pub async fn show_archive(archive: KyArchive) -> Response {
    text(archive.title)
}

/// A JSON error body without its `request_id`, which differs per request.
fn without_request_id(body: &str) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(body).expect("a JSON body");
    if let Some(object) = value.as_object_mut() {
        object.remove("request_id");
    }
    value
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE ky_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL, \
                title TEXT NOT NULL)",
            "CREATE TABLE ky_pages (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL, \
                title TEXT NOT NULL)",
            "CREATE TABLE ky_tokens (id TEXT PRIMARY KEY, label TEXT NOT NULL)",
            "INSERT INTO ky_posts (id, slug, title) VALUES (7, 'seven', 'Seven')",
            "INSERT INTO ky_pages (id, slug, title) VALUES (1, 'about', 'About us'), \
                (2, '1', 'Slug one')",
        ],
    )
    .await;
    db
}

fn router() -> Router {
    Router::new()
        .get("/posts/{post}", show_post)
        .get("/wrapped/{post}", show_wrapped)
        .get("/bare/{post}", show_bare)
        .get("/pages/{page}", show_page)
        .get("/tokens/{token}", show_token)
        .into()
}

#[tokio::test]
async fn bind_002_a_malformed_value_answers_the_404_a_missing_row_does() {
    let _db = fixture().await;
    let addr = serve(router()).await;

    assert_eq!(get(addr, "/posts/7").await, (200, "Seven".to_owned()));
    // `7abc` is no `i64`: it must not reach the query, where a lenient
    // database would read it as 7.
    let (malformed_status, malformed) = get(addr, "/posts/7abc").await;
    let (missing_status, missing) = get(addr, "/posts/8").await;
    assert_eq!((malformed_status, missing_status), (404, 404));
    assert_eq!(
        without_request_id(&malformed),
        without_request_id(&missing),
        "the two 404 bodies must be the same"
    );
    assert_eq!(message(&missing), "KyPost not found");
    assert!(
        !malformed.contains("7abc"),
        "the body repeated the value: {malformed}"
    );

    // Every binding form answers a malformed value with a 404.
    for (path, named) in [
        ("/wrapped/7abc", "KyPost not found"),
        ("/bare/7abc", "ky_post not found"),
    ] {
        let (status, body) = get(addr, path).await;
        assert_eq!((status, message(&body).as_str()), (404, named), "{path}");
        assert!(!body.contains("7abc"), "{path}: {body}");
    }
    let (status, body) = get(addr, "/bare/8").await;
    assert_eq!(
        (status, message(&body).as_str()),
        (404, "ky_post not found")
    );
}

#[tokio::test]
async fn bind_002_a_malformed_unique_id_key_answers_404() {
    let _db = fixture().await;
    let token = KyToken::create(attrs! { label: "first" }).await.unwrap();
    let addr = serve(router()).await;

    assert_eq!(
        get(addr, &format!("/tokens/{}", token.id)).await,
        (200, "first".to_owned())
    );
    let (status, body) = get(addr, "/tokens/not-a-uuid").await;
    assert_eq!(
        (status, message(&body).as_str()),
        (404, "KyToken not found")
    );
    assert!(!body.contains("not-a-uuid"), "{body}");
}

#[tokio::test]
async fn bind_002_a_model_binds_through_its_own_connection() {
    // The default connection has no `ky_archives` table; only the model's
    // own connection has the row.
    let _db = fixture().await;
    let archive_conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("archive connection");
    let archive = DbConnection::from_raw(archive_conn);
    {
        use sea_orm::ConnectionTrait;
        archive
            .inner()
            .execute_unprepared(
                "CREATE TABLE ky_archives (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                    title TEXT NOT NULL); \
                 INSERT INTO ky_archives (id, title) VALUES (3, 'archived')",
            )
            .await
            .expect("seed the archive");
    }
    ConnectionRegistry::register_existing("ky_archive", archive)
        .await
        .expect("register the archive connection");
    let addr = serve(
        Router::new()
            .get("/archives/{archive}", show_archive)
            .into(),
    )
    .await;
    assert_eq!(get(addr, "/archives/3").await, (200, "archived".to_owned()));
}

#[tokio::test]
async fn bind_005_a_route_key_binds_by_its_column() {
    let _db = fixture().await;
    let addr = serve(router()).await;
    assert_eq!(
        get(addr, "/pages/about").await,
        (200, "About us".to_owned())
    );
    // `1` is page 2's slug, and page 1's id: the slug wins.
    assert_eq!(get(addr, "/pages/1").await, (200, "Slug one".to_owned()));
    assert_eq!(get(addr, "/pages/2").await.0, 404, "an id is not a slug");
}

/// BIND-002 on Postgres: a malformed value for a `unique_id` key over a
/// native `uuid` column must not reach the database, where Postgres would
/// raise an error and the request would answer 500.
#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn bind_002_postgres_a_malformed_unique_id_key_answers_404() {
    let url = std::env::var("PG_TEST_URL").expect("PG_TEST_URL names a disposable database");
    let conn = sea_orm::Database::connect(&url)
        .await
        .expect("connect to Postgres");
    let db = DbConnection::from_raw(conn);
    {
        use sea_orm::ConnectionTrait;
        db.inner()
            .execute_unprepared(
                "DROP TABLE IF EXISTS ky_tokens; \
                 CREATE TABLE ky_tokens (id uuid PRIMARY KEY, label TEXT NOT NULL); \
                 DROP TABLE IF EXISTS ky_posts; \
                 CREATE TABLE ky_posts (id BIGSERIAL PRIMARY KEY, slug TEXT NOT NULL, \
                    title TEXT NOT NULL); \
                 INSERT INTO ky_posts (id, slug, title) VALUES (7, 'seven', 'Seven')",
            )
            .await
            .expect("create the table");
    }
    let _guard = suprnova::testing::TestContainer::fake();
    suprnova::testing::TestContainer::singleton(db.clone());
    let addr = serve(router()).await;
    // Compared with a `uuid` column, `not-a-uuid` is an error in Postgres;
    // the format check keeps it from reaching the query.
    let (status, body) = get(addr, "/tokens/not-a-uuid").await;
    assert_eq!(
        (status, message(&body).as_str()),
        (404, "KyToken not found"),
        "a malformed key must answer 404, not reach Postgres"
    );
    // The same for an integer key: `7abc` compared with a `bigint` column
    // is an error in Postgres.
    assert_eq!(get(addr, "/posts/7").await, (200, "Seven".to_owned()));
    let (status, body) = get(addr, "/posts/7abc").await;
    assert_eq!((status, message(&body).as_str()), (404, "KyPost not found"));
    {
        use sea_orm::ConnectionTrait;
        db.inner()
            .execute_unprepared("DROP TABLE IF EXISTS ky_tokens; DROP TABLE IF EXISTS ky_posts")
            .await
            .expect("drop the tables");
    }
}
