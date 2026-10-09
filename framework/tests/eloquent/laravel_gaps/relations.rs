//! Pivot identities, per-parent limits and morph writes.

use suprnova::testing::TestDatabase;
use suprnova::{Builder, Model, attrs, model};

/// An owner has ordinary and polymorphic children with custom keys.
#[model(table = "gap_owners", morph_type = "gap_owner", relations = {
    children: HasMany<Child> { fk = "owner_id" },
    child: HasOne<Child> { fk = "owner_id" },
    notes: MorphMany<Note> { name = "notable" },
    note: MorphOne<Note> { name = "notable" },
    roles: BelongsToMany<Role, Pair> { pivot_foreign_key = "owner_id", pivot_related_key = "role_id" },
    tags: MorphToMany<Role, MorphPair> { name = "taggable", pivot_related_key = "role_id" },
})]
pub struct Owner {
    /// The key identifies the parent.
    pub id: i64,
}

/// An ordinary child exercises grouped limits.
#[model(table = "gap_children")]
pub struct Child {
    /// The key also supplies a stable ordering.
    pub id: i64,
    /// This foreign key groups eager results.
    pub owner_id: i64,
    /// A score supplies a constraint and reversed ordering.
    pub score: i64,
}

/// A guarded morph child accepts content but trusts ownership to the relation.
#[model(table = "gap_notes", fillable = ["body"])]
pub struct Note {
    /// The key identifies the child.
    pub id: i64,
    /// The relation fills this owner id.
    pub notable_id: i64,
    /// The relation fills this owner type.
    pub notable_type: String,
    /// A unique body exercises upsert conflicts.
    pub body: String,
}

/// A related row carries a loaded pivot.
#[model(table = "gap_roles")]
pub struct Role {
    /// The key identifies the related row.
    pub id: i64,
}

/// The pivot has no surrogate id, so deletion must use both keys.
#[model(table = "gap_pairs", primary_key = "owner_id", auto_increment = false)]
pub struct Pair {
    /// One parent may have several rows with this same value.
    pub owner_id: i64,
    /// The pair identifies the attachment.
    pub role_id: i64,
}

/// The morph pivot also needs its type to identify an attachment.
#[model(
    table = "gap_morph_pairs",
    primary_key = "taggable_id",
    auto_increment = false
)]
pub struct MorphPair {
    /// Distinct morph families can share this value.
    pub taggable_id: i64,
    /// This value keeps those families apart.
    pub taggable_type: String,
    /// The related key completes the identity.
    pub role_id: i64,
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_owners (id INTEGER PRIMARY KEY AUTOINCREMENT)",
        "CREATE TABLE gap_children (id INTEGER PRIMARY KEY AUTOINCREMENT, owner_id INTEGER NOT NULL, score INTEGER NOT NULL)",
        "CREATE TABLE gap_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, notable_id INTEGER NOT NULL, notable_type TEXT NOT NULL, body TEXT NOT NULL CHECK(body <> 'invalid'), UNIQUE(notable_id, notable_type, body))",
        "CREATE TABLE gap_roles (id INTEGER PRIMARY KEY AUTOINCREMENT)",
        "CREATE TABLE gap_pairs (owner_id INTEGER NOT NULL, role_id INTEGER NOT NULL, UNIQUE(owner_id, role_id))",
        "CREATE TABLE gap_morph_pairs (taggable_id INTEGER NOT NULL, taggable_type TEXT NOT NULL, role_id INTEGER NOT NULL, UNIQUE(taggable_id, taggable_type, role_id))",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    db
}

#[tokio::test]
async fn eager_limits_are_per_parent_and_lazy_limits_stay_plain() {
    let _db = fixture().await;
    for _ in 0..3 {
        let owner = Owner::create(attrs! {}).await.expect("owner");
        for score in 1..=4 {
            Child::create(attrs! {owner_id: owner.id, score: score})
                .await
                .expect("child");
        }
        assert_eq!(owner.children().take(2).get().await.expect("lazy").len(), 2);
        assert!(
            owner
                .child()
                .limit(0)
                .first()
                .await
                .expect("zero")
                .is_none()
        );
    }
    suprnova::DB::enable_query_log().expect("enable log");
    suprnova::DB::flush_query_log().expect("flush log");
    let owners = Owner::query()
        .with_where(("children", |q: Builder<Child>| {
            q.filter_op("score", ">", 1).order_by_desc("score").limit(2)
        }))
        .get()
        .await
        .expect("eager");
    assert_eq!(owners.len(), 3);
    let selects = suprnova::DB::get_query_log()
        .expect("query log")
        .into_iter()
        .filter(|query| query.sql.trim_start().starts_with("SELECT"))
        .collect::<Vec<_>>();
    suprnova::DB::disable_query_log().expect("disable log");
    assert_eq!(selects.len(), 2, "eager loading stays batched");
    assert!(
        selects
            .iter()
            .any(|query| query.sql.contains("ROW_NUMBER() OVER (PARTITION BY"))
    );
    for owner in owners {
        assert_eq!(
            owner
                .children_loaded()
                .iter()
                .map(|r| r.score)
                .collect::<Vec<_>>(),
            [4, 3]
        );
    }
    let owners = Owner::query()
        .with_where(("children", |q: Builder<Child>| {
            q.order_by_desc("score").offset(1).take(2)
        }))
        .get()
        .await
        .expect("per-parent offset");
    assert!(owners.iter().all(|owner| {
        owner
            .children_loaded()
            .iter()
            .map(|row| row.score)
            .collect::<Vec<_>>()
            == [3, 2]
    }));
    let owners = Owner::query()
        .with_where(("children", |q: Builder<Child>| {
            q.in_order_of("score", [3])
                .order_by_desc("score")
                .filter_op("score", ">", 1)
                .limit(2)
        }))
        .get()
        .await
        .expect("bound order");
    assert!(owners.iter().all(|owner| {
        owner
            .children_loaded()
            .iter()
            .map(|row| row.score)
            .collect::<Vec<_>>()
            == [3, 4]
    }));
    let owners = Owner::query()
        .with_where(("children", |q: Builder<Child>| {
            q.filter_op("score", ">", 2)
                .union(
                    Child::query()
                        .filter_in("owner_id", [1, 2, 3])
                        .filter_op("score", "<=", 2),
                )
                .order_by_desc("score")
                .take(2)
        }))
        .get()
        .await
        .expect("window over a union");
    assert!(owners.iter().all(|owner| {
        owner
            .children_loaded()
            .iter()
            .map(|row| row.score)
            .collect::<Vec<_>>()
            == [4, 3]
    }));
    let owners = Owner::query()
        .with_where(("child", |q: Builder<Child>| {
            q.order_by_desc("score").take(1)
        }))
        .get()
        .await
        .expect("has one");
    assert!(
        owners
            .iter()
            .all(|owner| owner.child_loaded().expect("child").score == 4)
    );
    let owners = Owner::query()
        .with_where(("children", |q: Builder<Child>| q.take(0)))
        .get()
        .await
        .expect("zero eager");
    assert!(
        owners
            .iter()
            .all(|owner| owner.children_loaded().is_empty())
    );
    assert!(
        Owner::query()
            .with_where(("children", |q: Builder<Child>| q
                .order_by("bad;column", suprnova::Direction::Asc)
                .limit(2)))
            .get()
            .await
            .is_err()
    );
}

#[test]
fn eager_window_sql_keeps_bound_order_before_constraints_on_each_engine() {
    for backend in [
        suprnova::sea_orm::DbBackend::Sqlite,
        suprnova::sea_orm::DbBackend::Postgres,
        suprnova::sea_orm::DbBackend::MySql,
    ] {
        let query = Child::query()
            .filter_in("owner_id", [1, 2, 3])
            .filter_op("score", ">", 1)
            .in_order_of("score", [3])
            .take(2)
            .__eager_limit("owner_id");
        let (sql, values) = query
            .try_to_sql_with_bindings_for(backend)
            .expect("window SQL");
        assert!(sql.contains("ROW_NUMBER() OVER (PARTITION BY"), "{sql}");
        assert!(sql.contains("__suprnova_row_number <= 2"), "{sql}");
        assert!(!sql.contains(" LIMIT "), "{sql}");
        assert_eq!(values.len(), 5);
        assert_eq!(values[0], suprnova::sea_orm::Value::BigInt(Some(3)));
        if backend == suprnova::sea_orm::DbBackend::Postgres {
            for position in 1..=5 {
                assert!(sql.contains(&format!("${position}")), "{sql}");
            }
        }
    }
}

#[tokio::test]
async fn pivot_delete_uses_pair_in_lazy_and_eager_context() {
    let _db = fixture().await;
    let a = Owner::create(attrs! {}).await.expect("owner");
    let b = Owner::create(attrs! {}).await.expect("other owner");
    let x = Role::create(attrs! {}).await.expect("role");
    let y = Role::create(attrs! {}).await.expect("other role");
    a.roles().attach(x.id).await.expect("attach");
    a.roles().attach(y.id).await.expect("attach");
    b.roles().attach(x.id).await.expect("attach other");
    let rows = a.roles().get().await.expect("load");
    rows.iter()
        .find(|r| r.id == x.id)
        .expect("x")
        .pivot::<Pair>()
        .clone()
        .delete()
        .await
        .expect("delete pivot");
    assert_eq!(a.roles().count().await.expect("a count"), 1);
    assert_eq!(b.roles().count().await.expect("b count"), 1);
    let owners = Owner::with(["roles"]).get().await.expect("eager");
    let a = owners.iter().find(|r| r.id == a.id).expect("a");
    a.roles_loaded()[0]
        .pivot::<Pair>()
        .clone()
        .delete()
        .await
        .expect("eager pivot delete");
    assert_eq!(a.roles().count().await.expect("count"), 0);
}

#[tokio::test]
async fn morph_pivot_delete_keeps_other_types_with_the_same_pair() {
    let db = fixture().await;
    let a = Owner::create(attrs! {}).await.expect("owner");
    let role = Role::create(attrs! {}).await.expect("role");
    a.tags().attach(role.id).await.expect("attach");
    db.execute_unprepared("INSERT INTO gap_morph_pairs VALUES (1, 'other', 1)")
        .await
        .expect("other type");
    let rows = a.tags().get().await.expect("load");
    rows[0]
        .pivot::<MorphPair>()
        .clone()
        .delete()
        .await
        .expect("delete morph pivot");
    assert_eq!(a.tags().count().await.expect("count"), 0);
    assert_eq!(
        suprnova::DB::table("gap_morph_pairs")
            .count()
            .await
            .expect("other type survives"),
        1
    );
    a.tags().attach(role.id).await.expect("reattach");
    let owners = Owner::with(["tags"])
        .get()
        .await
        .expect("eager morph pivots");
    owners[0].tags_loaded()[0]
        .pivot::<MorphPair>()
        .clone()
        .delete()
        .await
        .expect("eager morph delete");
    assert_eq!(
        suprnova::DB::table("gap_morph_pairs")
            .count()
            .await
            .expect("other type still survives"),
        1
    );
}

#[tokio::test]
async fn morph_writes_set_guarded_ownership_and_report_write_failures() {
    let _db = fixture().await;
    let owner = Owner::create(attrs! {}).await.expect("owner");
    let other = Owner::create(attrs! {}).await.expect("other");
    let note = owner
        .notes()
        .create(attrs! {body: "one", notable_id: other.id, notable_type: "other"})
        .await
        .expect("create");
    assert_eq!(
        (note.notable_id, note.notable_type.as_str()),
        (owner.id, "gap_owner")
    );
    let saved = other.notes().save(note).await.expect("save");
    assert_eq!(
        (saved.notable_id, saved.notable_type.as_str()),
        (other.id, "gap_owner")
    );
    assert_eq!(other.notes().count().await.expect("saved count"), 1);
    let single = owner
        .note()
        .create(attrs! {body: "single"})
        .await
        .expect("morph one create");
    assert_eq!(single.notable_id, owner.id);
    let saved = other.note().save(single).await.expect("morph one save");
    assert_eq!(saved.notable_id, other.id);
    for _ in 0..2 {
        assert_eq!(
            owner
                .notes()
                .upsert(
                    vec![attrs! {body: "upsert"}],
                    vec!["notable_id", "notable_type", "body"],
                    Some(vec!["body"])
                )
                .await
                .expect("upsert"),
            1
        );
    }
    assert_eq!(owner.notes().count().await.expect("count"), 1);
    let mut fresh = Note::new();
    fresh.body = "new via save".to_owned();
    let inserted = owner.notes().save(fresh).await.expect("insert new child");
    assert!(inserted.id > 0);
    assert_eq!(
        (inserted.notable_id, inserted.notable_type.as_str()),
        (owner.id, "gap_owner")
    );
    let mut inserted = inserted;
    inserted.body = "changed through save".to_owned();
    let updated = other
        .note()
        .save(inserted)
        .await
        .expect("save modified child");
    assert_eq!(updated.body, "changed through save");
    let mut invalid = updated;
    invalid.body = "invalid".to_owned();
    assert!(owner.notes().save(invalid.clone()).await.is_err());
    assert!(owner.note().save(invalid).await.is_err());
    assert_eq!(
        other
            .notes()
            .filter("body", "changed through save")
            .count()
            .await
            .expect("failed save rollback"),
        1
    );
    assert_eq!(
        owner
            .note()
            .upsert(
                vec![attrs! {body: "upsert one"}],
                vec!["notable_id", "notable_type", "body"],
                None
            )
            .await
            .expect("one upsert"),
        1
    );
    assert_eq!(
        owner
            .note()
            .upsert(
                vec![attrs! {body: "upsert one"}],
                vec!["notable_id", "notable_type", "body"],
                None
            )
            .await
            .expect("upsert only unique columns"),
        1
    );
    assert_eq!(
        owner
            .note()
            .upsert(
                vec![attrs! {body: "upsert one"}],
                vec!["notable_id", "notable_type", "body"],
                Some(vec![])
            )
            .await
            .expect("empty update"),
        0
    );
    assert_eq!(
        owner
            .notes()
            .upsert(vec![], vec![], None)
            .await
            .expect("empty upsert"),
        0
    );
    assert!(
        owner
            .notes()
            .create(attrs! {body: "invalid"})
            .await
            .is_err()
    );
    assert!(owner.note().create(attrs! {body: "invalid"}).await.is_err());
    assert!(
        owner
            .notes()
            .upsert(
                vec![attrs! {body: "invalid"}],
                vec!["notable_id", "notable_type", "body"],
                None
            )
            .await
            .is_err()
    );
    assert!(
        owner
            .note()
            .upsert(
                vec![attrs! {body: "invalid"}],
                vec!["notable_id", "notable_type", "body"],
                None
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn morph_eager_limits_keep_each_owner_and_discriminator() {
    let db = fixture().await;
    for _ in 0..3 {
        let owner = Owner::create(attrs! {}).await.expect("owner");
        for n in 0..4 {
            owner
                .notes()
                .create(attrs! {body: format!("note {n}")})
                .await
                .expect("note");
        }
        db.execute_unprepared(&format!("INSERT INTO gap_notes (notable_id, notable_type, body) VALUES ({}, 'other', 'other type')", owner.id)).await.expect("other family");
        assert_eq!(owner.notes().take(2).get().await.expect("lazy").len(), 2);
        assert!(
            owner
                .note()
                .take(0)
                .first()
                .await
                .expect("zero morph one")
                .is_none()
        );
    }
    let owners = Owner::query()
        .with_where(("notes", |q: Builder<Note>| q.order_by_desc("id").take(2)))
        .get()
        .await
        .expect("eager");
    assert_eq!(owners.len(), 3);
    for owner in owners {
        assert_eq!(owner.notes_loaded().len(), 2);
        assert!(
            owner
                .notes_loaded()
                .iter()
                .all(|n| n.notable_id == owner.id && n.notable_type == "gap_owner")
        );
    }
    let owners = Owner::query()
        .with_where(("note", |q: Builder<Note>| q.order_by_desc("id").limit(1)))
        .get()
        .await
        .expect("morph one eager");
    for owner in owners {
        let child = owner.note_loaded().expect("one child");
        assert_eq!(
            (child.notable_id, child.notable_type.as_str()),
            (owner.id, "gap_owner")
        );
        assert_eq!(child.body, "note 3");
    }
}

#[tokio::test]
async fn pivot_delete_preserves_original_keys_and_propagates_failure() {
    let db = fixture().await;
    let a = Owner::create(attrs! {}).await.expect("owner");
    let x = Role::create(attrs! {}).await.expect("role");
    let y = Role::create(attrs! {}).await.expect("other role");
    a.roles().attach(x.id).await.expect("attach");
    a.roles().attach(y.id).await.expect("attach other");
    let rows = a.roles().get().await.expect("load");
    let mut pivot = rows
        .iter()
        .find(|row| row.id == x.id)
        .expect("x")
        .pivot::<Pair>()
        .clone();
    pivot.role_id = y.id;
    pivot.clone().delete().await.expect("delete original pair");
    pivot.delete().await.expect("already missing");
    let rows = a.roles().get().await.expect("remaining");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, y.id);
    db.execute_unprepared("CREATE TRIGGER refuse_pivot_delete BEFORE DELETE ON gap_pairs BEGIN SELECT RAISE(ABORT, 'delete refused'); END").await.expect("trigger");
    assert!(rows[0].pivot::<Pair>().clone().delete().await.is_err());
    assert_eq!(a.roles().count().await.expect("row remains"), 1);
}
