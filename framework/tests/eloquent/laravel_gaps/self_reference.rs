//! The existence filters on a relation that points back at its own table:
//! a category's `children` and `parent`, a person's `friends` through a
//! pivot, a through relation whose intermediate and target are the parent's
//! table, and a note that is the morph owner of other notes. The probe
//! aliases its inner tables, so the correlation compares a row with its
//! related rows instead of with itself.

use crate::eager::EgUser;
use suprnova::sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{Collection, Model, attrs, model};

/// A category tree. `children`, `parent` and `grandchildren` all read
/// `sr_categories`, the table the query starts from.
#[model(table = "sr_categories", relations = {
    children: HasMany<SrCategory> { fk = "parent_id" },
    parent: BelongsTo<SrCategory> { fk = "parent_id" },
    grandchildren: HasManyThrough<SrCategory, SrCategory> {
        first_key = "parent_id",
        second_key = "parent_id",
    },
})]
pub struct SrCategory {
    /// The database assigns the key.
    pub id: i64,
    /// A root has no parent.
    pub parent_id: Option<i64>,
    /// The name labels each case.
    pub name: String,
}

/// A soft-deleting tree: a trashed child does not count.
#[model(table = "sr_folders", soft_deletes, relations = {
    children: HasMany<SrFolder> { fk = "parent_id" },
})]
pub struct SrFolder {
    /// The database assigns the key.
    pub id: i64,
    /// A root has no parent.
    pub parent_id: Option<i64>,
    /// The name labels each case.
    pub name: String,
    /// A tombstone hides the folder.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// People who befriend other people through `sr_friendships`.
#[model(table = "sr_people", relations = {
    friends: BelongsToMany<SrPerson, SrFriendship> {
        pivot_foreign_key = "person_id",
        pivot_related_key = "friend_id",
    },
})]
pub struct SrPerson {
    /// The database assigns the key.
    pub id: i64,
    /// The name labels each case.
    pub name: String,
}

/// One direction of a friendship: `person_id` names `friend_id` a friend.
#[model(table = "sr_friendships", primary_key = "id")]
pub struct SrFriendship {
    /// The database assigns the key.
    pub id: i64,
    /// The person who names the friend.
    pub person_id: i64,
    /// The friend.
    pub friend_id: i64,
}

/// A note can be about another note: it is its own morph owner.
#[model(table = "sr_notes", morph_type = "sr_note", relations = {
    subject: MorphTo { name = "subject", targets = [SrNote] },
    notes: MorphMany<SrNote> { name = "subject" },
})]
pub struct SrNote {
    /// The database assigns the key.
    pub id: i64,
    /// The note this one is about, if any.
    pub subject_id: Option<i64>,
    /// The subject's morph type, if any.
    pub subject_type: Option<String>,
    /// The body labels each case.
    pub body: String,
}

/// The tree `root -> (a -> x, b)` and a lone `solo`. The keys follow the
/// order of creation: root 1, a 2, b 3, x 4, solo 5.
async fn tree() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE sr_categories (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER, name TEXT NOT NULL)",
    )
    .await
    .expect("schema");
    for (name, parent) in [
        ("root", None),
        ("a", Some(1)),
        ("b", Some(1)),
        ("x", Some(2)),
        ("solo", None),
    ] {
        SrCategory::create(attrs! { name: name, parent_id: parent })
            .await
            .expect("category");
    }
    db
}

/// The names of `rows`, sorted, so a test reads the set it matched.
fn names(rows: Collection<SrCategory>) -> Vec<String> {
    let mut names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn has_and_doesnt_have_split_a_tree_into_parents_and_leaves() {
    let _db = tree().await;
    for (query, expected) in [
        (SrCategory::query().has("children"), vec!["a", "root"]),
        (
            SrCategory::query().doesnt_have("children"),
            vec!["b", "solo", "x"],
        ),
        (SrCategory::query().has("parent"), vec!["a", "b", "x"]),
        (
            SrCategory::query().doesnt_have("parent"),
            vec!["root", "solo"],
        ),
        (
            SrCategory::query()
                .filter("name", "solo")
                .or_has("children"),
            vec!["a", "root", "solo"],
        ),
        (
            SrCategory::query()
                .filter("name", "root")
                .or_doesnt_have("children"),
            vec!["b", "root", "solo", "x"],
        ),
    ] {
        assert_eq!(names(query.get().await.expect("query")), expected);
    }
}

#[tokio::test]
async fn has_count_counts_the_children_of_each_row() {
    let _db = tree().await;
    for (op, count, expected) in [
        (">=", 2, vec!["root"]),
        ("=", 1, vec!["a"]),
        ("=", 0, vec!["b", "solo", "x"]),
    ] {
        let rows = SrCategory::query()
            .has_count("children", op, count)
            .get()
            .await
            .expect("has_count");
        assert_eq!(names(rows), expected, "{op} {count}");
    }
}

#[tokio::test]
async fn where_has_constrains_the_children_not_the_parent() {
    let _db = tree().await;
    let parent_of_x = SrCategory::query()
        .where_has::<SrCategory, _>("children", |q| q.filter("name", "x"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(names(parent_of_x), ["a"]);
    let without_x = SrCategory::query()
        .where_doesnt_have::<SrCategory, _>("children", |q| q.filter("name", "x"))
        .get()
        .await
        .expect("where_doesnt_have");
    assert_eq!(names(without_x), ["b", "root", "solo", "x"]);
    let related = SrCategory::query()
        .where_relation("children", "name", "x")
        .get()
        .await
        .expect("where_relation");
    assert_eq!(names(related), ["a"]);
    let either = SrCategory::query()
        .filter("name", "solo")
        .or_where_has::<SrCategory, _>("children", |q| q.filter("name", "b"))
        .get()
        .await
        .expect("or_where_has");
    assert_eq!(names(either), ["root", "solo"]);
}

#[tokio::test]
async fn a_qualified_column_in_the_predicate_still_names_the_parent() {
    let _db = tree().await;
    // Only `x` sorts after its parent's name (`a`); `a` and `b` sort
    // before `root`.
    let rows = SrCategory::query()
        .where_has::<SrCategory, _>("children", |q| {
            q.where_column_op("name", ">", "sr_categories.name")
        })
        .get()
        .await
        .expect("where_has with a column comparison");
    assert_eq!(names(rows), ["a"]);
}

#[tokio::test]
async fn a_nested_predicate_finds_the_grandparents() {
    let _db = tree().await;
    let rows = SrCategory::query()
        .where_has::<SrCategory, _>("children", |q| q.has("children"))
        .get()
        .await
        .expect("nested where_has");
    assert_eq!(names(rows), ["root"]);
    let rows = SrCategory::query()
        .where_has::<SrCategory, _>("children", |q| {
            q.where_has::<SrCategory, _>("children", |q| q.filter("name", "x"))
        })
        .get()
        .await
        .expect("nested where_has with a predicate");
    assert_eq!(names(rows), ["root"]);
    let rows = SrCategory::query()
        .where_has::<SrCategory, _>("children", |q| q.doesnt_have("children"))
        .get()
        .await
        .expect("nested doesnt_have");
    assert_eq!(names(rows), ["a", "root"]);
}

#[tokio::test]
async fn a_through_relation_over_its_own_table_reads_the_grandchildren() {
    let _db = tree().await;
    let rows = SrCategory::query()
        .has("grandchildren")
        .get()
        .await
        .expect("has through");
    assert_eq!(names(rows), ["root"]);
    let rows = SrCategory::query()
        .doesnt_have("grandchildren")
        .get()
        .await
        .expect("doesnt_have through");
    assert_eq!(names(rows), ["a", "b", "solo", "x"]);
    let rows = SrCategory::query()
        .where_has::<SrCategory, _>("grandchildren", |q| q.filter("name", "x"))
        .get()
        .await
        .expect("where_has through");
    assert_eq!(names(rows), ["root"]);
}

#[tokio::test]
async fn a_joined_query_correlates_to_its_own_table() {
    let _db = tree().await;
    // The self-join keeps the rows that have a parent; `has` then keeps
    // the ones that also have children.
    let rows = SrCategory::query()
        .join(
            "sr_categories as up",
            "up.id",
            "=",
            "sr_categories.parent_id",
        )
        .has("children")
        .get()
        .await
        .expect("joined has");
    assert_eq!(names(rows), ["a"]);
}

#[tokio::test]
async fn with_exists_flags_each_row_without_loading_the_relation() {
    let _db = tree().await;
    let rows = SrCategory::query()
        .with_exists("children")
        .order_by_asc("id")
        .get()
        .await
        .expect("existence flags");
    let flags: Vec<(&str, Option<bool>)> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.__eager.get_exists("children")))
        .collect();
    assert_eq!(
        flags,
        [
            ("root", Some(true)),
            ("a", Some(true)),
            ("b", Some(false)),
            ("x", Some(false)),
            ("solo", Some(false)),
        ]
    );
    assert!(rows.iter().all(|row| !row.__eager.has("children")));
}

#[tokio::test]
async fn relation_counts_and_sums_read_the_children_of_each_row() {
    let _db = tree().await;
    let rows = SrCategory::query()
        .with_count(["children", "grandchildren"])
        .with_sum(("children", "id"))
        .order_by_asc("id")
        .get()
        .await
        .expect("relation aggregates");
    let counts: Vec<(&str, u64, u64, Option<f64>)> = rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.children_count(),
                row.grandchildren_count(),
                row.children_sum_of("id"),
            )
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("root", 2, 1, Some(5.0)),
            ("a", 1, 0, Some(4.0)),
            ("b", 0, 0, Some(0.0)),
            ("x", 0, 0, Some(0.0)),
            ("solo", 0, 0, Some(0.0)),
        ]
    );
}

#[tokio::test]
async fn a_trashed_child_does_not_count() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE sr_folders (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER, name TEXT NOT NULL, deleted_at TEXT)",
    )
    .await
    .expect("schema");
    let kept = SrFolder::create(attrs! { name: "kept", parent_id: None::<i64> })
        .await
        .expect("folder");
    let emptied = SrFolder::create(attrs! { name: "emptied", parent_id: None::<i64> })
        .await
        .expect("folder");
    SrFolder::create(attrs! { name: "live", parent_id: Some(kept.id) })
        .await
        .expect("child");
    let trashed = SrFolder::create(attrs! { name: "trashed", parent_id: Some(emptied.id) })
        .await
        .expect("child");
    trashed.delete().await.expect("trash the child");
    let folders = |rows: Collection<SrFolder>| {
        let mut names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
        names.sort_unstable();
        names
    };
    let has = SrFolder::query().has("children").get().await.expect("has");
    assert_eq!(folders(has), ["kept"]);
    let missing = SrFolder::query()
        .doesnt_have("children")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(folders(missing), ["emptied", "live"]);
}

#[tokio::test]
async fn a_self_referential_many_to_many_reads_the_pivot() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE sr_people (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE sr_friendships (id INTEGER PRIMARY KEY AUTOINCREMENT, person_id INTEGER NOT NULL, friend_id INTEGER NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    for name in ["ann", "bob", "cat"] {
        SrPerson::create(attrs! { name: name })
            .await
            .expect("person");
    }
    // Ann names Bob a friend; nobody names Ann or Cat.
    SrFriendship::create(attrs! { person_id: 1, friend_id: 2 })
        .await
        .expect("friendship");
    let people = |rows: Collection<SrPerson>| {
        let mut names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
        names.sort_unstable();
        names
    };
    let has = SrPerson::query().has("friends").get().await.expect("has");
    assert_eq!(people(has), ["ann"]);
    let missing = SrPerson::query()
        .doesnt_have("friends")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(people(missing), ["bob", "cat"]);
    let bob = SrPerson::query()
        .where_has::<SrPerson, _>("friends", |q| q.filter("name", "bob"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(people(bob), ["ann"]);
    let ann = SrPerson::query()
        .where_has::<SrPerson, _>("friends", |q| q.filter("name", "ann"))
        .get()
        .await
        .expect("where_has without a match");
    assert!(ann.is_empty());
    let counted = SrPerson::query()
        .with_count(["friends"])
        .order_by_asc("id")
        .get()
        .await
        .expect("with_count");
    let counts: Vec<(&str, u64)> = counted
        .iter()
        .map(|row| (row.name.as_str(), row.friends_count()))
        .collect();
    assert_eq!(counts, [("ann", 1), ("bob", 0), ("cat", 0)]);
}

#[tokio::test]
async fn a_note_that_owns_notes_reads_its_morph_relations() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE sr_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id INTEGER, subject_type TEXT, body TEXT NOT NULL)",
    )
    .await
    .expect("schema");
    for (body, kind, key) in [
        ("first", None, None),
        ("reply", Some("sr_note"), Some(1)),
        ("dangling", Some("sr_note"), Some(99)),
    ] {
        SrNote::create(attrs! { body: body, subject_type: kind, subject_id: key })
            .await
            .expect("note");
    }
    let notes = |rows: Collection<SrNote>| {
        let mut bodies: Vec<String> = rows.iter().map(|row| row.body.clone()).collect();
        bodies.sort_unstable();
        bodies
    };
    let with_subject = SrNote::query().has("subject").get().await.expect("has");
    assert_eq!(notes(with_subject), ["reply"]);
    let without_subject = SrNote::query()
        .doesnt_have("subject")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(notes(without_subject), ["dangling", "first"]);
    let about_first = SrNote::query()
        .where_has::<SrNote, _>("subject", |q| q.filter("body", "first"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(notes(about_first), ["reply"]);
    let with_notes = SrNote::query().has("notes").get().await.expect("has");
    assert_eq!(notes(with_notes), ["first"]);
    let without_notes = SrNote::query()
        .doesnt_have("notes")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(notes(without_notes), ["dangling", "reply"]);
}

#[test]
fn only_a_self_referential_probe_is_aliased() {
    assert_eq!(
        EgUser::query().has("posts").to_sql_for(DbBackend::Postgres),
        "SELECT * FROM eg_users WHERE EXISTS (SELECT 1 FROM eg_posts WHERE eg_posts.eg_user_id = eg_users.id)"
    );
    assert_eq!(
        SrCategory::query()
            .has("children")
            .to_sql_for(DbBackend::Postgres),
        "SELECT * FROM sr_categories WHERE EXISTS (SELECT 1 FROM sr_categories AS __suprnova_related WHERE __suprnova_related.parent_id = sr_categories.id)"
    );
    assert_eq!(
        SrCategory::query()
            .where_has::<SrCategory, _>("children", |q| q.has("children"))
            .to_sql_for(DbBackend::Postgres),
        "SELECT * FROM sr_categories WHERE EXISTS (SELECT 1 FROM sr_categories AS __suprnova_related WHERE __suprnova_related.parent_id = sr_categories.id AND (EXISTS (SELECT 1 FROM sr_categories AS __suprnova_related_1 WHERE __suprnova_related_1.parent_id = __suprnova_related.id)))"
    );
    assert_eq!(
        SrPerson::query()
            .has("friends")
            .to_sql_for(DbBackend::Postgres),
        "SELECT * FROM sr_people WHERE EXISTS (SELECT 1 FROM sr_friendships AS __suprnova_pivot INNER JOIN sr_people AS __suprnova_related ON __suprnova_pivot.friend_id = __suprnova_related.id WHERE __suprnova_pivot.person_id = sr_people.id)"
    );
}
