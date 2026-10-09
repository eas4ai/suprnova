//! Relation operations keep ownership, pivot context and bounded reads.

use std::sync::atomic::{AtomicUsize, Ordering};

use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, Model, attrs, model};

/// An owner exercises custom keys on every relation operation.
#[model(table = "delta_owners", relations = {
    records: HasMany<DeltaRecord> { fk = "owner_code", lk = "code" },
    record: HasOne<DeltaRecord> { fk = "owner_code", lk = "code" },
    roles: BelongsToMany<DeltaRole, DeltaPivot> {
        pivot_foreign_key = "owner_code", pivot_related_key = "role_code",
        lk = "code", related_key = "code", with_pivot = ["enabled"],
    },
    through_records: HasManyThrough<DeltaMember, DeltaRecord> {
        first_key = "owner_code", second_key = "member_code",
        lk = "code", second_local_key = "code",
    },
    through_record: HasOneThrough<DeltaMember, DeltaRecord> {
        first_key = "owner_code", second_key = "member_code",
        lk = "code", second_local_key = "code",
    },
})]
pub struct DeltaOwner {
    /// The id supplies the relation fixture value.
    pub id: i64,
    /// The code supplies the relation fixture value.
    pub code: String,
}

/// An intermediate checks reachability and soft-delete filtering.
#[model(table = "delta_members", soft_deletes)]
pub struct DeltaMember {
    /// The id supplies the relation fixture value.
    pub id: i64,
    /// The code supplies the relation fixture value.
    pub code: String,
    /// The owner code supplies the relation fixture value.
    pub owner_code: String,
    /// The deleted at supplies the relation fixture value.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A guarded child checks ownership and counter updates.
#[model(table = "delta_records", fillable = ["name", "score", "member_code"], soft_deletes)]
pub struct DeltaRecord {
    /// The id supplies the relation fixture value.
    pub id: i64,
    /// The owner code supplies the relation fixture value.
    pub owner_code: String,
    /// The member code supplies the relation fixture value.
    pub member_code: String,
    /// The name supplies the relation fixture value.
    pub name: String,
    /// The score supplies the relation fixture value.
    pub score: i64,
    /// The secret supplies the relation fixture value.
    pub secret: String,
    /// The deleted at supplies the relation fixture value.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A string-keyed target checks pivot attachment and mapping.
#[model(table = "delta_roles", primary_key = "code", fillable = ["code", "name", "score"], auto_increment = false)]
pub struct DeltaRole {
    /// The code supplies the relation fixture value.
    pub code: String,
    /// The name supplies the relation fixture value.
    pub name: String,
    /// The score supplies the relation fixture value.
    pub score: i64,
}

/// A pivot checks custom key binding and loaded context.
#[model(table = "delta_pivots")]
pub struct DeltaPivot {
    /// The id supplies the relation fixture value.
    pub id: i64,
    /// The owner code supplies the relation fixture value.
    pub owner_code: String,
    /// The role code supplies the relation fixture value.
    pub role_code: String,
    /// The enabled supplies the relation fixture value.
    pub enabled: i64,
}

async fn fixture() -> (TestDatabase, DeltaOwner, DeltaOwner) {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE delta_owners (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL UNIQUE)",
        "CREATE TABLE delta_members (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL UNIQUE, owner_code TEXT NOT NULL, deleted_at TEXT)",
        "CREATE TABLE delta_records (id INTEGER PRIMARY KEY AUTOINCREMENT, owner_code TEXT NOT NULL DEFAULT '', member_code TEXT NOT NULL DEFAULT '', name TEXT NOT NULL, score INTEGER NOT NULL DEFAULT 0 CHECK(score >= 0), secret TEXT NOT NULL DEFAULT '', deleted_at TEXT, UNIQUE(owner_code, name))",
        "CREATE TABLE delta_roles (code TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, score INTEGER NOT NULL DEFAULT 0 CHECK(score >= 0))",
        "CREATE TABLE delta_pivots (id INTEGER PRIMARY KEY AUTOINCREMENT, owner_code TEXT NOT NULL, role_code TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, UNIQUE(owner_code, role_code))",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    let owner = DeltaOwner::create(attrs! { code: "one" })
        .await
        .expect("owner");
    let other = DeltaOwner::create(attrs! { code: "two" })
        .await
        .expect("other owner");
    (db, owner, other)
}

async fn seed_through(owner: &DeltaOwner, count: i64) -> Vec<i64> {
    let member = DeltaMember::create(
        attrs! { code: format!("member-{}", owner.code), owner_code: &owner.code },
    )
    .await
    .expect("member");
    let mut ids = Vec::new();
    for score in 0..count {
        let row = owner
            .records()
            .first_or_create(
                attrs! { name: format!("record-{score}") },
                attrs! { member_code: &member.code, score: score },
            )
            .await
            .expect("record");
        ids.push(row.id);
    }
    ids
}

async fn scoped_relation_pools() -> (TestDatabase, DeltaOwner, String) {
    use sea_orm::ConnectionTrait;
    use suprnova::{ConnectionRegistry, DbConnection};

    let (primary, owner, _) = fixture().await;
    primary
        .execute_unprepared(
            "INSERT INTO delta_records (id, owner_code, name, score) VALUES (1, 'one', 'counter', 70)",
        )
        .await
        .expect("primary counter");
    let reporting = DbConnection::from_raw(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("reporting"),
    );
    for sql in [
        "CREATE TABLE delta_owners (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL UNIQUE)",
        "CREATE TABLE delta_records (id INTEGER PRIMARY KEY AUTOINCREMENT, owner_code TEXT NOT NULL DEFAULT '', member_code TEXT NOT NULL DEFAULT '', name TEXT NOT NULL, score INTEGER NOT NULL DEFAULT 0 CHECK(score >= 0), secret TEXT NOT NULL DEFAULT '', deleted_at TEXT, UNIQUE(owner_code, name))",
        "INSERT INTO delta_owners (id, code) VALUES (1, 'one')",
        "INSERT INTO delta_records (id, owner_code, name, score) VALUES (1, 'one', 'counter', 12)",
    ] {
        reporting
            .inner()
            .execute_unprepared(sql)
            .await
            .expect("reporting fixture");
    }
    let name = format!("relation_reporting_{}", uuid::Uuid::new_v4());
    ConnectionRegistry::register_existing(&name, reporting)
        .await
        .expect("register reporting");
    (primary, owner, name)
}

#[tokio::test]
async fn scoped_relation_increment_reloads_from_task_default() {
    let (_primary, owner, reporting) = scoped_relation_pools().await;
    DB::with_default_connection(&reporting, async {
        let scoped_owner = DeltaOwner::find(owner.id).await?.expect("reporting owner");
        let row = scoped_owner
            .records()
            .increment_or_create(attrs! { name: "counter" }, "score", 99, 5, attrs! {})
            .await?;
        assert_eq!(row.id, 1);
        assert_eq!(row.score, 17);
        let row = scoped_owner
            .record()
            .increment_or_create(attrs! { name: "counter" }, "score", 99, 3, attrs! {})
            .await?;
        assert_eq!(row.score, 20);
        assert_eq!(
            DeltaRecord::find(1).await?.expect("stored counter").score,
            20
        );

        // This counter has no primary counterpart, so its reload must also stay scoped.
        let created = scoped_owner
            .records()
            .increment_or_create(attrs! { name: "reporting-only" }, "score", 4, 2, attrs! {})
            .await?;
        assert_eq!(created.score, 4);
        let incremented = scoped_owner
            .records()
            .increment_or_create(attrs! { name: "reporting-only" }, "score", 99, 2, attrs! {})
            .await?;
        assert_eq!(incremented.id, created.id);
        assert_eq!(incremented.score, 6);
        Ok(())
    })
    .await
    .expect("scoped increment");
    assert_eq!(
        DeltaRecord::find(1)
            .await
            .expect("primary")
            .expect("counter")
            .score,
        70
    );
    assert_eq!(
        DeltaRecord::query().count().await.expect("primary count"),
        1
    );
}

#[tokio::test]
async fn scoped_relation_unique_recovery_reads_from_task_default() {
    use sea_orm::ConnectionTrait;

    let (primary, owner, reporting) = scoped_relation_pools().await;
    primary
        .execute_unprepared(
            "INSERT INTO delta_records (id, owner_code, name, score) VALUES (2, 'one', 'recovery', 70)",
        )
        .await
        .expect("primary recovery row");
    DB::with_default_connection(&reporting, async {
        let scoped_owner = DeltaOwner::find(owner.id).await?.expect("reporting owner");
        assert!(scoped_owner.records().filter("name", "recovery").first().await?.is_none());
        // Insert the competing row after the initial lookup. FAIL preserves that
        // row when the second trigger insert raises a real unique violation.
        DB::connection()?
            .inner()
            .execute_unprepared(
                "CREATE TRIGGER race_delta_record BEFORE INSERT ON delta_records
                 WHEN NEW.name = 'recovery' AND NOT EXISTS (
                     SELECT 1 FROM delta_records WHERE owner_code = NEW.owner_code AND name = NEW.name
                 ) BEGIN
                     INSERT INTO delta_records (id, owner_code, name, score) VALUES (2, NEW.owner_code, NEW.name, 20);
                     INSERT OR FAIL INTO delta_records (id, owner_code, name, score) VALUES (3, NEW.owner_code, NEW.name, 99);
                 END",
            )
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?;
        let row = scoped_owner
            .records()
            .first_or_create(attrs! { name: "recovery" }, attrs! { score: 99 })
            .await?;
        assert_eq!(row.id, 2);
        assert_eq!(row.owner_code, "one");
        assert_eq!(row.score, 20);
        assert_eq!(scoped_owner.records().filter("name", "recovery").count().await?, 1);
        Ok(())
    })
    .await
    .expect("scoped unique recovery");
    assert_eq!(
        DeltaRecord::find(2)
            .await
            .expect("primary")
            .expect("recovery row")
            .score,
        70
    );
    assert_eq!(
        DeltaRecord::query().count().await.expect("primary count"),
        2
    );
}

#[tokio::test]
async fn has_many_first_or_create_sets_guarded_custom_key_and_keeps_other_attributes_guarded() {
    let (_db, owner, other) = fixture().await;
    let unrelated = other
        .records()
        .first_or_create(attrs! { name: "shared" }, attrs! { score: 2 })
        .await
        .expect("other record");
    let row = owner
        .records()
        .first_or_create(
            attrs! { name: "shared" },
            attrs! { score: 7, secret: "private", owner_code: "wrong" },
        )
        .await
        .expect("record");
    assert_ne!(row.id, unrelated.id);
    assert_eq!(row.owner_code, "one");
    assert_eq!(row.score, 7);
    assert_eq!(row.secret, "");
    let again = owner
        .records()
        .first_or_create(attrs! { name: "shared" }, attrs! { score: 99 })
        .await
        .expect("found");
    assert_eq!(again.id, row.id);
    assert_eq!(again.score, 7);
    assert_eq!(owner.records().count().await.expect("count"), 1);
}

#[tokio::test]
async fn has_one_first_or_create_sets_custom_key_and_returns_existing() {
    let (_db, owner, _) = fixture().await;
    let row = owner
        .record()
        .first_or_create(attrs! { name: "profile" }, attrs! { score: 3 })
        .await
        .expect("create");
    let found = owner
        .record()
        .first_or_create(attrs! { name: "profile" }, attrs! { score: 55 })
        .await
        .expect("found");
    assert_eq!(row.owner_code, "one");
    assert_eq!(found.id, row.id);
    assert_eq!(found.score, 3);
}

#[tokio::test]
async fn has_many_increment_or_create_uses_default_then_exact_step_and_extra() {
    let (_db, owner, other) = fixture().await;
    other
        .records()
        .first_or_create(attrs! { name: "counter" }, attrs! { score: 70 })
        .await
        .expect("unrelated");
    let row = owner
        .records()
        .increment_or_create(
            attrs! { name: "counter" },
            "score",
            12,
            5,
            attrs! { member_code: "initial", score: 100 },
        )
        .await
        .expect("create");
    assert_eq!(row.owner_code, "one");
    assert_eq!(row.score, 12);
    let found = owner
        .records()
        .increment_or_create(
            attrs! { name: "counter" },
            "score",
            100,
            5,
            attrs! { member_code: "changed" },
        )
        .await
        .expect("increment");
    assert_eq!(found.id, row.id);
    assert_eq!(found.score, 17);
    assert_eq!(found.member_code, "changed");
    let found = owner
        .records()
        .increment_or_create(attrs! { name: "counter" }, "score", 0, -2, attrs! {})
        .await
        .expect("negative step");
    assert_eq!(found.score, 15);
    assert_eq!(
        other
            .records()
            .first()
            .await
            .expect("read")
            .expect("row")
            .score,
        70
    );
}

#[tokio::test]
async fn has_one_increment_or_create_uses_default_then_step() {
    let (_db, owner, _) = fixture().await;
    let row = owner
        .record()
        .increment_or_create(attrs! { name: "counter" }, "score", 4, 3, attrs! {})
        .await
        .expect("create");
    assert_eq!(row.owner_code, "one");
    assert_eq!(row.score, 4);
    let found = owner
        .record()
        .increment_or_create(attrs! { name: "counter" }, "score", 40, 3, attrs! {})
        .await
        .expect("increment");
    assert_eq!(found.id, row.id);
    assert_eq!(found.score, 7);
}

#[tokio::test]
async fn keyed_creation_and_increment_propagate_errors_without_partial_update() {
    let (_db, owner, _) = fixture().await;
    assert!(
        owner
            .record()
            .first_or_create(attrs! { name: "bad" }, attrs! { score: -1 })
            .await
            .is_err()
    );
    assert!(
        owner
            .records()
            .increment_or_create(attrs! { name: "bad" }, "score", -1, 1, attrs! {})
            .await
            .is_err()
    );
    let row = owner
        .records()
        .first_or_create(attrs! { name: "good" }, attrs! { score: 2 })
        .await
        .expect("create");
    assert!(
        owner
            .records()
            .increment_or_create(
                attrs! { name: "good" },
                "score",
                0,
                -3,
                attrs! { member_code: "changed" }
            )
            .await
            .is_err()
    );
    let stored = DeltaRecord::find(row.id).await.expect("read").expect("row");
    assert_eq!(stored.score, 2);
    assert_eq!(stored.member_code, "");
    assert!(
        owner
            .record()
            .increment_or_create(
                attrs! { name: "good" },
                "score; DROP TABLE delta_records",
                1,
                1,
                attrs! {}
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn belongs_to_many_first_or_create_attaches_new_and_existing_global_record_once() {
    let (_db, owner, other) = fixture().await;
    let role = owner
        .roles()
        .first_or_create(attrs! { code: "admin", name: "Admin" }, attrs! { score: 6 })
        .await
        .expect("create");
    assert_eq!(role.code, "admin");
    let found = owner
        .roles()
        .first_or_create(attrs! { code: "admin" }, attrs! { score: 90 })
        .await
        .expect("found");
    assert_eq!(found.score, 6);
    let shared = other
        .roles()
        .first_or_create(attrs! { code: "admin" }, attrs! { score: 100 })
        .await
        .expect("attach existing");
    assert_eq!(shared.score, 6);
    assert_eq!(DeltaRole::query().count().await.expect("roles"), 1);
    assert_eq!(owner.roles().count().await.expect("pivot"), 1);
    assert_eq!(other.roles().count().await.expect("other pivot"), 1);
}

#[tokio::test]
async fn belongs_to_many_increment_or_create_attaches_and_increments_shared_existing_record() {
    let (_db, owner, other) = fixture().await;
    let role = owner
        .roles()
        .increment_or_create(
            attrs! { code: "counter", name: "Counter" },
            "score",
            8,
            4,
            attrs! {},
        )
        .await
        .expect("create");
    assert_eq!(role.score, 8);
    let found = other
        .roles()
        .increment_or_create(
            attrs! { code: "counter" },
            "score",
            80,
            4,
            attrs! { name: "Updated" },
        )
        .await
        .expect("attach and increment");
    assert_eq!(found.score, 12);
    assert_eq!(found.name, "Updated");
    let again = other
        .roles()
        .increment_or_create(attrs! { code: "counter" }, "score", 80, 0, attrs! {})
        .await
        .expect("zero step");
    assert_eq!(again.score, 12);
    assert_eq!(other.roles().count().await.expect("pivot"), 1);
}

#[tokio::test]
async fn belongs_to_many_creation_propagates_record_and_attach_errors() {
    let (db, owner, _) = fixture().await;
    assert!(
        owner
            .roles()
            .first_or_create(attrs! { code: "bad", name: "Bad" }, attrs! { score: -1 })
            .await
            .is_err()
    );
    db.execute_unprepared("CREATE TRIGGER reject_delta_pivot BEFORE INSERT ON delta_pivots BEGIN SELECT RAISE(ABORT, 'attach failed'); END").await.expect("failing attachment");
    assert!(
        owner
            .roles()
            .first_or_create(attrs! { code: "missing", name: "Missing" }, attrs! {})
            .await
            .is_err()
    );
    assert_eq!(DeltaRole::query().count().await.expect("count"), 0);
}

#[tokio::test]
async fn belongs_to_many_chunk_map_maps_25_records_in_order_with_pivots_and_bounded_queries() {
    let (_db, owner, other) = fixture().await;
    for score in 0..25 {
        owner
            .roles()
            .first_or_create(
                attrs! { code: format!("r{score:02}"), name: format!("Role {score}") },
                attrs! { score: score },
            )
            .await
            .expect("role");
    }
    other
        .roles()
        .first_or_create(attrs! { code: "unrelated", name: "Other" }, attrs! {})
        .await
        .expect("other");
    DB::enable_query_log().expect("log");
    DB::flush_query_log().expect("flush");
    let calls = AtomicUsize::new(0);
    let mapped = owner
        .roles()
        .chunk_map(10, |role| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move {
                assert_eq!(role.pivot::<DeltaPivot>().owner_code, "one");
                Ok(role.score * 2)
            }
        })
        .await
        .expect("map");
    assert_eq!(calls.load(Ordering::SeqCst), 25);
    assert_eq!(
        mapped.into_vec(),
        (0..25).map(|n| n * 2).collect::<Vec<_>>()
    );
    let queries = DB::get_query_log().expect("queries");
    DB::disable_query_log().expect("disable");
    let reads: Vec<_> = queries
        .iter()
        .filter(|q| q.sql.contains("delta_roles"))
        .collect();
    assert_eq!(reads.len(), 3);
    assert!(reads.iter().all(|q| q.sql.to_lowercase().contains("limit")));
}

#[tokio::test]
async fn belongs_to_many_chunk_map_empty_zero_filtered_and_callback_error() {
    let (_db, owner, _) = fixture().await;
    let empty = owner
        .roles()
        .chunk_map(10, |_| async { Ok(1) })
        .await
        .expect("empty");
    assert!(empty.is_empty());
    assert!(
        owner
            .roles()
            .chunk_map(0, |_| async { Ok(1) })
            .await
            .is_err()
    );
    for score in 0..3 {
        owner
            .roles()
            .first_or_create(
                attrs! { code: format!("r{score}"), name: format!("Role {score}") },
                attrs! { score: score },
            )
            .await
            .expect("role");
    }
    let filtered = owner
        .roles()
        .where_pivot("role_code", "r1")
        .chunk_map(1, |role| async move { Ok(role.score) })
        .await
        .expect("filtered");
    assert_eq!(filtered.into_vec(), vec![1]);
    let calls = AtomicUsize::new(0);
    assert!(
        owner
            .roles()
            .chunk_map(2, |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err::<i64, _>(FrameworkError::bad_request("callback failed")) }
            })
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn through_chunk_map_maps_records_in_order_for_both_relations() {
    let (_db, owner, other) = fixture().await;
    seed_through(&owner, 25).await;
    seed_through(&other, 1).await;
    let calls = AtomicUsize::new(0);
    let values = owner
        .through_records()
        .chunk_map(10, |row| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { Ok(row.score) }
        })
        .await
        .expect("many map");
    assert_eq!(calls.load(Ordering::SeqCst), 25);
    assert_eq!(values.into_vec(), (0..25).collect::<Vec<_>>());
    let values = owner
        .through_record()
        .chunk_map(10, |row| async move { Ok(row.score) })
        .await
        .expect("one map");
    assert_eq!(values.into_vec(), (0..25).collect::<Vec<_>>());
}

#[tokio::test]
async fn through_chunk_map_empty_zero_callback_error_and_database_error() {
    let (db, owner, _) = fixture().await;
    assert!(
        owner
            .through_records()
            .chunk_map(2, |_| async { Ok(1) })
            .await
            .expect("empty")
            .is_empty()
    );
    assert!(
        owner
            .through_record()
            .chunk_map(0, |_| async { Ok(1) })
            .await
            .is_err()
    );
    seed_through(&owner, 1).await;
    assert!(
        owner
            .through_records()
            .chunk_map(1, |_| async {
                Err::<i64, _>(FrameworkError::bad_request("callback"))
            })
            .await
            .is_err()
    );
    db.execute_unprepared("DROP TABLE delta_members")
        .await
        .expect("drop intermediate");
    assert!(
        owner
            .through_record()
            .chunk_map(2, |_| async { Ok(1) })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn through_find_or_new_limits_lookup_to_reachable_rows_and_leaves_missing_unsaved() {
    let (_db, owner, other) = fixture().await;
    let ids = seed_through(&owner, 1).await;
    let other_ids = seed_through(&other, 1).await;
    let found = owner
        .through_records()
        .find_or_new(ids[0])
        .await
        .expect("found");
    assert_eq!(found.id, ids[0]);
    let found = owner
        .through_record()
        .find_or_new(ids[0])
        .await
        .expect("one found");
    assert_eq!(found.id, ids[0]);
    for key in [other_ids[0], 99999] {
        let missing = owner.through_records().find_or_new(key).await.expect("new");
        assert_eq!(missing.id, 0);
        assert_eq!(missing.owner_code, "");
        let missing = owner
            .through_record()
            .find_or_new(key)
            .await
            .expect("one new");
        assert_eq!(missing.id, 0);
    }
    assert_eq!(DeltaRecord::query().count().await.expect("count"), 2);
}

#[tokio::test]
async fn through_find_or_new_respects_intermediate_and_target_soft_deletes_and_errors() {
    let (db, owner, _) = fixture().await;
    let ids = seed_through(&owner, 1).await;
    db.execute_unprepared("UPDATE delta_records SET deleted_at = '2026-10-09T00:00:00Z'")
        .await
        .expect("delete target");
    assert_eq!(
        owner
            .through_records()
            .find_or_new(ids[0])
            .await
            .expect("new")
            .id,
        0
    );
    db.execute_unprepared("UPDATE delta_records SET deleted_at = NULL")
        .await
        .expect("restore target");
    db.execute_unprepared("UPDATE delta_members SET deleted_at = '2026-10-09T00:00:00Z'")
        .await
        .expect("delete intermediate");
    assert_eq!(
        owner
            .through_record()
            .find_or_new(ids[0])
            .await
            .expect("new")
            .id,
        0
    );
    db.execute_unprepared("DROP TABLE delta_members")
        .await
        .expect("drop intermediate");
    assert!(owner.through_records().find_or_new(ids[0]).await.is_err());
}

#[tokio::test]
async fn has_one_through_is_asks_database_once_without_loading_target() {
    let (_db, owner, other) = fixture().await;
    let ids = seed_through(&owner, 1).await;
    let others = seed_through(&other, 1).await;
    let reached = DeltaRecord::find(ids[0])
        .await
        .expect("read")
        .expect("target");
    let unrelated = DeltaRecord::find(others[0])
        .await
        .expect("read")
        .expect("other target");
    DB::enable_query_log().expect("log");
    for (target, expected) in [(&reached, true), (&unrelated, false)] {
        DB::flush_query_log().expect("flush");
        assert_eq!(
            owner.through_record().is(target).await.expect("is"),
            expected
        );
        let log = DB::get_query_log().expect("log");
        assert_eq!(log.len(), 1);
        assert!(log[0].sql.to_lowercase().contains("count("));
        assert!(!log[0].sql.contains("SELECT *"));
    }
    DB::disable_query_log().expect("disable");
}

#[tokio::test]
async fn has_one_through_is_returns_false_for_unsaved_and_deleted_and_propagates_errors() {
    let (db, owner, _) = fixture().await;
    let ids = seed_through(&owner, 1).await;
    let reached = DeltaRecord::find(ids[0])
        .await
        .expect("read")
        .expect("target");
    assert!(
        !owner
            .through_record()
            .is(&DeltaRecord::default())
            .await
            .expect("unsaved")
    );
    db.execute_unprepared("UPDATE delta_members SET deleted_at = '2026-10-09T00:00:00Z'")
        .await
        .expect("delete");
    assert!(!owner.through_record().is(&reached).await.expect("deleted"));
    db.execute_unprepared("DROP TABLE delta_members")
        .await
        .expect("drop intermediate");
    assert!(owner.through_record().is(&reached).await.is_err());
}

#[tokio::test]
async fn belongs_to_many_increment_failure_keeps_counter_and_extra_unchanged() {
    let (_db, owner, _) = fixture().await;
    owner
        .roles()
        .first_or_create(
            attrs! { code: "counter", name: "Counter" },
            attrs! { score: 2 },
        )
        .await
        .expect("create");
    assert!(
        owner
            .roles()
            .increment_or_create(
                attrs! { code: "counter" },
                "score",
                0,
                -3,
                attrs! { name: "Changed" }
            )
            .await
            .is_err()
    );
    let role = DeltaRole::find("counter".to_string())
        .await
        .expect("read")
        .expect("role");
    assert_eq!(role.score, 2);
    assert_eq!(role.name, "Counter");
    assert!(
        owner
            .roles()
            .increment_or_create(
                attrs! { code: "bad", name: "Bad" },
                "score; DROP TABLE delta_roles",
                0,
                1,
                attrs! {}
            )
            .await
            .is_err()
    );
    assert_eq!(DeltaRole::query().count().await.expect("count"), 1);
}

#[tokio::test]
async fn through_chunk_map_uses_bounded_queries_and_excludes_deleted_rows() {
    let (db, owner, _) = fixture().await;
    seed_through(&owner, 25).await;
    db.execute_unprepared(
        "UPDATE delta_records SET deleted_at = '2026-10-09T00:00:00Z' WHERE score = 7",
    )
    .await
    .expect("delete target");
    DB::enable_query_log().expect("log");
    DB::flush_query_log().expect("flush");
    let values = owner
        .through_records()
        .chunk_map(10, |row| async move { Ok(row.score) })
        .await
        .expect("map");
    let queries = DB::get_query_log().expect("queries");
    DB::disable_query_log().expect("disable");
    assert_eq!(values.len(), 24);
    assert!(!values.contains(&7));
    assert_eq!(queries.len(), 3);
    assert!(
        queries
            .iter()
            .all(|q| q.sql.to_lowercase().contains("limit"))
    );
    db.execute_unprepared("UPDATE delta_members SET deleted_at = '2026-10-09T00:00:00Z'")
        .await
        .expect("delete intermediate");
    assert!(
        owner
            .through_record()
            .chunk_map(10, |_| async { Ok(1) })
            .await
            .expect("map")
            .is_empty()
    );
}
