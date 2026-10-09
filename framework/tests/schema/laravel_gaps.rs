//! Laravel's column storage defaults and explicit precision and length.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;
use suprnova::{AsBool, Blueprint, Model, attrs, model};

#[test]
fn column_sql_matches_each_engine() {
    for backend in [DbBackend::MySql, DbBackend::Postgres, DbBackend::Sqlite] {
        let sql = Blueprint::create_sql("gap_types", backend, |t| {
            t.boolean("flag");
            t.float("ratio");
            t.float("small_ratio").precision(24);
            t.timestamp_tz("at");
            t.timestamp_tz("precise_at").precision(3);
            t.ulid("standard");
            t.ulid_with_length("short", 20);
            t.ulid("modified").length(19);
        })
        .unwrap()
        .join("; ");
        let (quote, flag, ratio, small_ratio, at, precise_at, ulid) = match backend {
            DbBackend::MySql => (
                '`',
                "tinyint(1)",
                "float(53)",
                "float(24)",
                "timestamp(0)",
                "timestamp(3)",
                "char",
            ),
            DbBackend::Postgres => (
                '"',
                "bool",
                "double precision",
                "float(24)",
                "timestamp(0) with time zone",
                "timestamp(3) with time zone",
                "char",
            ),
            _ => (
                '"',
                "tinyint(1)",
                "float",
                "float",
                "datetime",
                "datetime",
                "char",
            ),
        };
        for (name, ty) in [
            ("flag", flag),
            ("ratio", ratio),
            ("small_ratio", small_ratio),
            ("at", at),
            ("precise_at", precise_at),
        ] {
            assert!(
                sql.contains(&format!("{quote}{name}{quote} {ty} NOT NULL")),
                "{backend:?}: {sql}"
            );
        }
        for (name, length) in [("standard", 26), ("short", 20), ("modified", 19)] {
            assert!(
                sql.contains(&format!("{quote}{name}{quote} {ulid}({length}) NOT NULL")),
                "{backend:?}: {sql}"
            );
        }
    }
}

#[test]
fn precision_boundaries_and_invalid_ulid_length_are_checked_before_sql() {
    for digits in [1, 24, 25, 53] {
        let sql = Blueprint::create_sql("valid", DbBackend::Postgres, |t| {
            t.float("ratio").precision(digits);
            t.timestamp_tz("whole").precision(0);
            t.timestamp_tz("micro").precision(6);
        })
        .unwrap()
        .join("; ");
        assert!(sql.contains("timestamp(0) with time zone"));
        assert!(sql.contains("timestamp(6) with time zone"));
    }
    for digits in [0, 54, u32::MAX] {
        let error = Blueprint::create_sql("invalid", DbBackend::Postgres, |t| {
            t.float("ratio").precision(digits);
        })
        .unwrap_err();
        assert!(error.to_string().contains("from 1 to 53"));
    }
    let error = Blueprint::create_sql("invalid", DbBackend::Sqlite, |t| {
        t.ulid_with_length("code", 0);
    })
    .unwrap_err();
    assert!(error.to_string().contains("positive length"));
    let error = Blueprint::create_sql("invalid", DbBackend::Postgres, |t| {
        t.timestamp_tz("at").precision(7);
    })
    .unwrap_err();
    assert!(error.to_string().contains("from 0 to 6"));
}

#[model(table = "gap_flags", fillable = ["flag"], casts = { flag = AsBool })]
/// Reads SQLite boolean storage through the same cast your application uses.
pub struct Flag {
    /// Identifies each row so the test checks storage order.
    pub id: i64,
    /// Checks that the boolean cast preserves each stored value.
    pub flag: bool,
}

struct FlagMigration;
impl MigrationName for FlagMigration {
    fn name(&self) -> &str {
        "m20261009_gap_flags"
    }
}
#[async_trait::async_trait]
impl MigrationTrait for FlagMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::create(manager, "gap_flags", |t| {
            t.id();
            t.boolean("flag");
        })
        .await
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop(manager, "gap_flags").await
    }
}
struct FlagMigrator;
#[async_trait::async_trait]
impl MigratorTrait for FlagMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(FlagMigration)]
    }
}

#[suprnova::suprnova_test(migrator = FlagMigrator)]
async fn sqlite_boolean_declaration_and_zero_one_round_trip(db: TestDatabase) {
    let columns = db
        .conn()
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(gap_flags)",
        ))
        .await
        .unwrap();
    let column = columns
        .iter()
        .find(|row| row.try_get::<String>("", "name").unwrap() == "flag")
        .unwrap();
    assert_eq!(column.try_get::<String>("", "type").unwrap(), "tinyint(1)");
    db.execute_unprepared("INSERT INTO gap_flags (flag) VALUES (0), (1)")
        .await
        .unwrap();
    let flags = Flag::query().order_by_asc("id").get().await.unwrap();
    assert!(!flags[0].flag);
    assert!(flags[1].flag);
    Flag::create(attrs! { flag: false }).await.unwrap();
    Flag::create(attrs! { flag: true }).await.unwrap();
    let flags = Flag::query().order_by_asc("id").get().await.unwrap();
    assert_eq!(
        flags.iter().map(|flag| flag.flag).collect::<Vec<_>>(),
        vec![false, true, false, true]
    );
    let stored = db
        .fetch_all("SELECT flag FROM gap_flags ORDER BY id", vec![])
        .await
        .unwrap();
    assert_eq!(
        stored
            .iter()
            .map(|row| row.try_get::<i64>("", "flag").unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 1]
    );
}

#[tokio::test]
async fn sqlite_timestamp_tz_is_datetime_and_alter_uses_the_same_types() {
    let conn = super::sqlite::connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, "gap_times", |t| {
        t.timestamp_tz("at");
        t.timestamp_tz("precise_at").precision(6);
        t.float("ratio");
        t.ulid_with_length("code", 20);
    })
    .await
    .unwrap();
    Schema::table(&manager, "gap_times", |t| {
        t.boolean("flag").default(false);
        t.timestamp_tz("extra_at").nullable();
    })
    .await
    .unwrap();
    let columns = conn
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA table_info(gap_times)",
        ))
        .await
        .unwrap();
    for (name, ty) in [
        ("at", "datetime"),
        ("precise_at", "datetime"),
        ("extra_at", "datetime"),
        ("flag", "tinyint(1)"),
        ("code", "char(20)"),
    ] {
        let column = columns
            .iter()
            .find(|row| row.try_get::<String>("", "name").unwrap() == name)
            .unwrap();
        assert_eq!(column.try_get::<String>("", "type").unwrap(), ty);
    }
    conn.execute_unprepared("INSERT INTO gap_times (at, precise_at, ratio, code) VALUES ('2026-10-09 12:00:00', '2026-10-09 12:00:00.123456', 1.23456789012345, '12345678901234567890')").await.unwrap();
    let row = conn
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT at, precise_at, ratio, code, flag FROM gap_times",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "precise_at").unwrap(),
        "2026-10-09 12:00:00.123456"
    );
    assert_eq!(row.try_get::<f64>("", "ratio").unwrap(), 1.23456789012345);
    assert_eq!(row.try_get::<i64>("", "flag").unwrap(), 0);
}
