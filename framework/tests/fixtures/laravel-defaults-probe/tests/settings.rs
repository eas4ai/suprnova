//! PAR-045 through a package that carries both settings: the casts
//! `#[suprnova::model]` chose from `datetime_cast = "native"`, and the
//! columns this package's binaries create with `unsigned_ids = true`.
//!
//! Each backend test runs the three binaries in turn against one database:
//! `plain-probe` (no setting), `documented-call-probe` (the documented
//! call) and `laravel-defaults-probe` (`#[suprnova::main]`). With the
//! setting, MySQL gets `BIGINT UNSIGNED` keys; Postgres and SQLite get the
//! columns `plain-probe` created. The models then write and read the
//! native columns. The Postgres and MySQL tests need `PG_TEST_URL` and
//! `MYSQL_TEST_URL`; `scripts/check-laravel-defaults.sh` creates both, and
//! a run without them fails rather than passing silently.

use std::path::Path;
use std::process::Command;

use laravel_defaults_probe::{ProbePost, ProbeUser, probe_post, probe_user};
use suprnova::chrono::{DateTime, Duration, TimeZone, Utc};
use suprnova::sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use suprnova::testing::{TestClock, TestContainer};
use suprnova::{DbConnection, Model, Touchable, attrs};

const MAIN: &str = env!("CARGO_BIN_EXE_laravel-defaults-probe");
const DOCUMENTED: &str = env!("CARGO_BIN_EXE_documented-call-probe");
const PLAIN: &str = env!("CARGO_BIN_EXE_plain-probe");

/// The key columns `unsigned_ids` decides: both tables' `id` and the
/// foreign key between them.
const KEYS: &[(&str, &str)] = &[
    ("probe_users", "id"),
    ("probe_posts", "id"),
    ("probe_posts", "probe_user_id"),
];

/// Compiles only while `datetime_cast = "native"` gives every date-time
/// field of `ProbePost` without a cast of its own `AsNativeDateTime` or
/// `AsOptionalNativeDateTime`, whose storage is the moment itself; the
/// text casts store a `String`. `archived_at` names the text cast, and it
/// wins.
fn post_storage(
    row: probe_post::Model,
) -> (
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
) {
    (
        row.created_at,
        row.updated_at,
        row.deleted_at,
        row.archived_at,
    )
}

/// The same for `ProbeUser`'s managed timestamps.
fn user_storage(row: probe_user::Model) -> (DateTime<Utc>, DateTime<Utc>) {
    (row.created_at, row.updated_at)
}

/// Runs one of the package's binaries against `url`, which migrates it.
fn migrate_with(binary: &str, url: &str) {
    let output = Command::new(binary)
        .env("PROBE_DATABASE_URL", url)
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .output()
        .expect("run the probe binary");
    assert!(
        output.status.success(),
        "{binary} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn execute(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql).await.expect(sql);
}

/// Drops what a migration made, so the next binary starts from nothing.
async fn reset(conn: &DatabaseConnection) {
    for table in ["probe_posts", "probe_users", "seaql_migrations"] {
        execute(conn, &format!("DROP TABLE IF EXISTS {table}")).await;
    }
}

/// MariaDB, and MySQL before 8.0.19, write an integer column's display
/// width into `column_type` (`bigint(20) unsigned`); MySQL 8.4 leaves it
/// out (`bigint unsigned`). The width changes nothing the column stores, so
/// it is dropped and both engines report the same type.
fn without_display_width(declared: &str) -> String {
    for integer in ["tinyint", "smallint", "mediumint", "bigint", "int"] {
        let Some(rest) = declared.strip_prefix(integer) else {
            continue;
        };
        if let Some((width, tail)) = rest.strip_prefix('(').and_then(|r| r.split_once(')'))
            && !width.is_empty()
            && width.bytes().all(|b| b.is_ascii_digit())
        {
            return format!("{integer}{tail}");
        }
        break;
    }
    declared.to_owned()
}

/// The declared type of each of [`KEYS`], as the catalog reports it,
/// lower-cased and without an integer's display width.
async fn key_types(conn: &DatabaseConnection) -> Vec<String> {
    let backend = conn.get_database_backend();
    let mut types = Vec::new();
    for (table, column) in KEYS {
        let declared = match backend {
            DbBackend::Sqlite => {
                let rows = conn
                    .query_all_raw(Statement::from_string(
                        backend,
                        format!("PRAGMA table_info('{table}')"),
                    ))
                    .await
                    .expect("PRAGMA table_info");
                rows.iter()
                    .find(|row| row.try_get::<String>("", "name").ok().as_deref() == Some(column))
                    .map(|row| {
                        row.try_get::<String>("", "type")
                            .expect("the declared type")
                    })
                    .unwrap_or_else(|| panic!("{table}.{column} exists"))
            }
            DbBackend::MySql | DbBackend::Postgres => {
                let (type_column, schema) = if backend == DbBackend::MySql {
                    ("CAST(column_type AS CHAR)", "DATABASE()")
                } else {
                    ("data_type::text", "current_schema()")
                };
                let row = conn
                    .query_one_raw(Statement::from_string(
                        backend,
                        format!(
                            "SELECT {type_column} AS t FROM information_schema.columns \
                             WHERE table_schema = {schema} AND table_name = '{table}' \
                             AND column_name = '{column}'"
                        ),
                    ))
                    .await
                    .expect("information_schema.columns")
                    .unwrap_or_else(|| panic!("{table}.{column} exists"));
                row.try_get("", "t").expect("the declared type")
            }
            other => panic!("no catalog query for {other:?}"),
        };
        types.push(without_display_width(&declared.to_lowercase()));
    }
    types
}

/// The models write and read the native columns the migration made,
/// through create, a touch of the owner, `touch`, a soft delete, a restore
/// and a comparison, at whole seconds so MySQL's `TIMESTAMP` holds each
/// moment exactly. Postgres refuses the text the default cast would bind
/// for a `timestamp with time zone` column.
async fn models_round_trip(conn: &DatabaseConnection) {
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let at = |hour: i64| {
        Utc.with_ymd_and_hms(2031, 3, 14, 0, 0, 0)
            .single()
            .expect("a valid moment")
            + Duration::hours(hour)
    };
    let clock = TestClock::travel_to(at(9));

    let ada = ProbeUser::create(attrs! { name: "ada" })
        .await
        .expect("create the user");
    clock.set(at(10));
    let archived = at(3);
    let post = ProbePost::create(attrs! {
        probe_user_id: ada.id,
        title: "first",
        archived_at: archived,
    })
    .await
    .expect("create the post");

    let read = ProbePost::find(post.id)
        .await
        .expect("find the post")
        .expect("the post");
    assert_eq!(
        (read.created_at, read.updated_at, read.archived_at),
        (at(10), at(10), Some(archived))
    );
    let (created_at, updated_at, deleted_at, archived_text) = post_storage(read.clone().into());
    assert_eq!((created_at, updated_at, deleted_at), (at(10), at(10), None));
    assert_eq!(archived_text, Some(archived.to_rfc3339()));

    let owner = ProbeUser::find(ada.id)
        .await
        .expect("find the user")
        .expect("the user");
    assert_eq!(
        user_storage(owner.into()),
        (at(9), at(10)),
        "the post touched its user"
    );

    clock.set(at(11));
    read.touch().await.expect("touch");
    clock.set(at(12));
    let touched = ProbePost::find(post.id)
        .await
        .expect("find after touch")
        .expect("the post");
    assert_eq!(touched.updated_at, at(11));
    touched.delete().await.expect("soft delete");
    let trashed = ProbePost::with_trashed()
        .filter("id", post.id)
        .first()
        .await
        .expect("with_trashed")
        .expect("the soft deleted post");
    assert_eq!(trashed.deleted_at, Some(at(12)));
    trashed.restore().await.expect("restore");

    assert_eq!(
        ProbePost::query()
            .filter_op("created_at", ">", "2031-03-14T09:30:00Z")
            .count()
            .await
            .expect("compare a native column"),
        1
    );
}

async fn settings_reach_models_and_migrations(url: &str) {
    let conn = Database::connect(url)
        .await
        .expect("the test database is reachable");
    let backend = conn.get_database_backend();

    reset(&conn).await;
    migrate_with(PLAIN, url);
    let plain = key_types(&conn).await;
    reset(&conn).await;
    migrate_with(DOCUMENTED, url);
    let documented = key_types(&conn).await;
    reset(&conn).await;
    migrate_with(MAIN, url);
    let main = key_types(&conn).await;

    if backend == DbBackend::MySql {
        assert!(
            plain.iter().all(|declared| declared == "bigint"),
            "without the setting the keys are signed: {plain:?}"
        );
        assert!(
            documented
                .iter()
                .all(|declared| declared == "bigint unsigned"),
            "the documented call makes them unsigned: {documented:?}"
        );
        assert!(
            main.iter().all(|declared| declared == "bigint unsigned"),
            "#[suprnova::main] makes them unsigned: {main:?}"
        );
    } else {
        assert_eq!(documented, plain, "the documented call changes no column");
        assert_eq!(main, plain, "#[suprnova::main] changes no column");
    }

    models_round_trip(&conn).await;
    reset(&conn).await;
}

#[tokio::test]
async fn sqlite_settings_reach_models_and_migrations() {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("laravel-defaults-probe.sqlite");
    // A file left by an earlier run would still hold its tables.
    if path.exists() {
        std::fs::remove_file(&path).expect("remove the previous database");
    }
    settings_reach_models_and_migrations(&format!("sqlite://{}?mode=rwc", path.display())).await;
}

#[tokio::test]
async fn postgres_settings_reach_models_and_migrations() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres; scripts/check-laravel-defaults.sh does");
    settings_reach_models_and_migrations(&url).await;
}

#[tokio::test]
async fn mysql_settings_reach_models_and_migrations() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL; scripts/check-laravel-defaults.sh does");
    settings_reach_models_and_migrations(&url).await;
}
