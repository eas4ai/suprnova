//! Relation paths honour what each `#[model]` declares.
//!
//! The relation machinery reaches models through generic code and
//! macro-emitted raw SQL. Every path here must apply the same
//! declarations a plain `Model::query()` read applies: the soft-delete
//! filter and global scopes of the rows it reads, the storage casts of
//! the rows it writes, and the keys, pivot table and connection the
//! relation names.
//!
//! - Counts and aggregates apply the related model's scopes and
//!   soft-delete filter, as the row loads do.
//! - A declared local key (`lk`) is the key a relation reads, eager or
//!   lazy.
//! - An eager `HasManyThrough` leaves out rows reached through a trashed
//!   intermediate, as the lazy read does.
//! - An eager many-to-many matches pivots on the declared related key.
//! - Pivot extras are written through the pivot model's casts, and the
//!   pivot context is read from the relation's own pivot table.
//! - Eager loads run on the transaction or connection the parent query
//!   was given.
//! - A mass prune deletes on the model's connection.
//! - A factory insert of a SeaORM row joins the surrounding transaction.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serial_test::serial;
use suprnova::database::ConnectionRegistry;
use suprnova::eloquent::MassPrunable;
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};
use suprnova::testing::TestDatabase;
use suprnova::{AsEncrypted, Builder, DB, DbConnection, Factory, Model, attrs, model};

/// How long a relation read may take before the test calls it stuck. A
/// read sent to a pool whose one connection a transaction holds waits
/// for that connection; this turns the wait into a failure.
const STUCK: Duration = Duration::from_secs(5);

// ---- Counts and aggregates apply the related model's filters ------------

#[model(table = "rd_authors", relations = {
    books: HasMany<RdBook>,
    notes: HasMany<RdNote>,
    remarks: MorphMany<RdRemark> { name = "remarkable" },
    shelves: BelongsToMany<RdShelf, RdAuthorShelf>,
}, morph_type = "author")]
pub struct RdAuthor {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_books", soft_deletes)]
pub struct RdBook {
    pub id: i64,
    pub rd_author_id: i64,
    pub pages: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "rd_notes")]
pub struct RdNote {
    pub id: i64,
    pub rd_author_id: i64,
    pub visible: i64,
    pub words: i64,
}

static RD_NOTES_VISIBLE: AtomicI64 = AtomicI64::new(1);

/// Hides the notes whose `visible` is not the current value.
pub struct RdVisibleNotes;

impl GlobalScope<RdNote> for RdVisibleNotes {
    fn apply(&self, query: Builder<RdNote>) -> Builder<RdNote> {
        query.filter("visible", RD_NOTES_VISIBLE.load(Ordering::SeqCst))
    }
}

#[model(table = "rd_remarks", soft_deletes)]
pub struct RdRemark {
    pub id: i64,
    pub remarkable_id: i64,
    pub remarkable_type: String,
    pub stars: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "rd_shelves", soft_deletes)]
pub struct RdShelf {
    pub id: i64,
    pub width: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "rd_author_shelf", timestamps = false)]
pub struct RdAuthorShelf {
    pub id: i64,
    pub rd_author_id: i64,
    pub rd_shelf_id: i64,
}

async fn author_fixture() -> (TestDatabase, i64) {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_books (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_author_id INTEGER NOT NULL, pages INTEGER NOT NULL, deleted_at TEXT)",
        "CREATE TABLE rd_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_author_id INTEGER NOT NULL, visible INTEGER NOT NULL, words INTEGER NOT NULL)",
        "CREATE TABLE rd_remarks (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            remarkable_id INTEGER NOT NULL, remarkable_type TEXT NOT NULL, \
            stars INTEGER NOT NULL, deleted_at TEXT)",
        "CREATE TABLE rd_shelves (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            width INTEGER NOT NULL, deleted_at TEXT)",
        "CREATE TABLE rd_author_shelf (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_author_id INTEGER NOT NULL, rd_shelf_id INTEGER NOT NULL)",
        "INSERT INTO rd_authors (id, name) VALUES (1, 'ann')",
        // Two live books and one trashed one.
        "INSERT INTO rd_books (rd_author_id, pages, deleted_at) VALUES \
            (1, 10, NULL), (1, 20, NULL), (1, 100, '2026-01-01T00:00:00+00:00')",
        // Two visible notes and one the scope hides.
        "INSERT INTO rd_notes (rd_author_id, visible, words) VALUES \
            (1, 1, 3), (1, 1, 4), (1, 0, 500)",
        // Two live remarks and one trashed one.
        "INSERT INTO rd_remarks (remarkable_id, remarkable_type, stars, deleted_at) VALUES \
            (1, 'author', 2, NULL), (1, 'author', 3, NULL), \
            (1, 'author', 50, '2026-01-01T00:00:00+00:00')",
        // A live shelf and a trashed one, both attached.
        "INSERT INTO rd_shelves (id, width, deleted_at) VALUES \
            (1, 7, NULL), (2, 900, '2026-01-01T00:00:00+00:00')",
        "INSERT INTO rd_author_shelf (rd_author_id, rd_shelf_id) VALUES (1, 1), (1, 2)",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    ScopeRegistry::register::<RdNote, _>(RdVisibleNotes);
    (db, 1)
}

/// `with_count` and `with_sum` / `with_max` count and total the rows
/// `with` loads: the related model's soft-delete filter and global
/// scopes apply to them, on every relation kind.
#[tokio::test]
async fn counts_and_aggregates_apply_the_related_models_filters() {
    let (_db, id) = author_fixture().await;

    let loaded = RdAuthor::query()
        .with(["books", "notes", "remarks", "shelves"])
        .get()
        .await
        .unwrap();
    let author = loaded.iter().find(|a| a.id == id).unwrap();
    assert_eq!(author.books_loaded().len(), 2);
    assert_eq!(author.notes_loaded().len(), 2);
    assert_eq!(author.remarks_loaded().len(), 2);
    assert_eq!(author.shelves_loaded().len(), 1);

    let counted = RdAuthor::query()
        .with_count(["books", "notes", "remarks"])
        .with_sum(("books", "pages"))
        .with_max(("books", "pages"))
        .with_sum(("notes", "words"))
        .with_sum(("remarks", "stars"))
        .with_max(("shelves", "width"))
        .get()
        .await
        .unwrap();
    let author = counted.iter().find(|a| a.id == id).unwrap();
    assert_eq!(author.books_count(), 2, "a trashed book is not counted");
    assert_eq!(author.books_sum_of("pages"), Some(30.0));
    assert_eq!(author.books_max_of("pages"), Some(Some(20.0)));
    assert_eq!(
        author.notes_count(),
        2,
        "a note the scope hides is not counted"
    );
    assert_eq!(author.notes_sum_of("words"), Some(7.0));
    assert_eq!(author.remarks_count(), 2, "a trashed remark is not counted");
    assert_eq!(author.remarks_sum_of("stars"), Some(5.0));
    assert_eq!(
        author.shelves_max_of("width"),
        Some(Some(7.0)),
        "a trashed shelf is not in the aggregate"
    );
}

// ---- Through relations and trashed intermediates ------------------------

#[model(table = "rd_countries", relations = {
    articles: HasManyThrough<RdCitizen, RdArticle>,
})]
pub struct RdCountry {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_citizens", soft_deletes)]
pub struct RdCitizen {
    pub id: i64,
    pub rd_country_id: i64,
    pub name: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "rd_articles", soft_deletes)]
pub struct RdArticle {
    pub id: i64,
    pub rd_citizen_id: i64,
    pub words: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

async fn country_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_countries (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_citizens (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_country_id INTEGER NOT NULL, name TEXT NOT NULL, deleted_at TEXT)",
        "CREATE TABLE rd_articles (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_citizen_id INTEGER NOT NULL, words INTEGER NOT NULL, deleted_at TEXT)",
        "INSERT INTO rd_countries (id, name) VALUES (1, 'here')",
        // Citizen 1 is live, citizen 2 is trashed.
        "INSERT INTO rd_citizens (id, rd_country_id, name, deleted_at) VALUES \
            (1, 1, 'live', NULL), (2, 1, 'gone', '2026-01-01T00:00:00+00:00')",
        // A live article and a trashed one of the live citizen, and a live
        // article of the trashed citizen.
        "INSERT INTO rd_articles (rd_citizen_id, words, deleted_at) VALUES \
            (1, 10, NULL), (1, 70, '2026-01-01T00:00:00+00:00'), (2, 400, NULL)",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db
}

/// An eager `HasManyThrough` leaves out the rows reached through a
/// trashed intermediate, as `country.articles().get()` does, and its
/// count and aggregate agree with both.
#[tokio::test]
async fn eager_through_skips_rows_of_a_trashed_intermediate() {
    let _db = country_fixture().await;
    let country = RdCountry::find(1).await.unwrap().unwrap();
    let lazy = country.articles().get().await.unwrap();
    assert_eq!(lazy.len(), 1, "the lazy read reaches the one live article");
    assert_eq!(country.articles().count().await.unwrap(), 1);

    let eager = RdCountry::query().with(["articles"]).get().await.unwrap();
    let words: Vec<i64> = eager[0].articles_loaded().iter().map(|a| a.words).collect();
    assert_eq!(words, vec![10], "the eager load agrees with the lazy read");

    let counted = RdCountry::query()
        .with_count(["articles"])
        .with_sum(("articles", "words"))
        .get()
        .await
        .unwrap();
    assert_eq!(counted[0].articles_count(), 1);
    assert_eq!(counted[0].articles_sum_of("words"), Some(10.0));
}

// ---- Declared local keys --------------------------------------------------

#[model(table = "rd_owners", relations = {
    items: HasMany<RdItem> { fk = "owner_key", lk = "external_key" },
    badge: HasOne<RdOwnerBadge> { fk = "owner_key", lk = "external_key" },
    teams: BelongsToMany<RdTeam, RdOwnerTeam> {
        lk = "external_key",
        pivot_foreign_key = "owner_key",
    },
    tasks: HasManyThrough<RdSquad, RdTask> { first_key = "owner_key", lk = "external_key" },
})]
pub struct RdOwner {
    pub id: i64,
    pub external_key: i64,
    pub name: String,
}

#[model(table = "rd_items")]
pub struct RdItem {
    pub id: i64,
    pub owner_key: i64,
    pub label: String,
}

#[model(table = "rd_owner_badges")]
pub struct RdOwnerBadge {
    pub id: i64,
    pub owner_key: i64,
    pub label: String,
}

#[model(table = "rd_teams")]
pub struct RdTeam {
    pub id: i64,
    pub label: String,
}

#[model(table = "rd_owner_team", timestamps = false)]
pub struct RdOwnerTeam {
    pub id: i64,
    pub owner_key: i64,
    pub rd_team_id: i64,
}

#[model(table = "rd_squads")]
pub struct RdSquad {
    pub id: i64,
    pub owner_key: i64,
}

#[model(table = "rd_tasks")]
pub struct RdTask {
    pub id: i64,
    pub rd_squad_id: i64,
    pub label: String,
}

/// Owner 1 has the key 42; owner 42 has the key 7. A relation that read
/// the primary key would give owner 1 nothing and give owner 42 the rows
/// of owner 1.
async fn owner_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_owners (id INTEGER PRIMARY KEY, external_key INTEGER NOT NULL, \
            name TEXT NOT NULL)",
        "CREATE TABLE rd_items (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            owner_key INTEGER NOT NULL, label TEXT NOT NULL)",
        "CREATE TABLE rd_owner_badges (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            owner_key INTEGER NOT NULL, label TEXT NOT NULL)",
        "CREATE TABLE rd_teams (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
        "CREATE TABLE rd_owner_team (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            owner_key INTEGER NOT NULL, rd_team_id INTEGER NOT NULL)",
        "CREATE TABLE rd_squads (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            owner_key INTEGER NOT NULL)",
        "CREATE TABLE rd_tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_squad_id INTEGER NOT NULL, label TEXT NOT NULL)",
        "INSERT INTO rd_owners (id, external_key, name) VALUES (1, 42, 'one'), (42, 7, 'forty-two')",
        "INSERT INTO rd_items (owner_key, label) VALUES (42, 'of one'), (7, 'of forty-two')",
        "INSERT INTO rd_owner_badges (owner_key, label) VALUES (42, 'of one'), (7, 'of forty-two')",
        "INSERT INTO rd_teams (id, label) VALUES (1, 'of one'), (2, 'of forty-two')",
        "INSERT INTO rd_squads (id, owner_key) VALUES (1, 42), (2, 7)",
        "INSERT INTO rd_tasks (rd_squad_id, label) VALUES (1, 'of one'), (2, 'of forty-two')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db
}

fn labels<'a>(rows: impl IntoIterator<Item = &'a String>) -> Vec<&'a str> {
    rows.into_iter().map(String::as_str).collect()
}

/// A relation declared with `lk = "external_key"` reads with that key,
/// lazy and eager, and the pivot writes of a many-to-many use it too.
#[tokio::test]
async fn a_declared_local_key_is_the_key_a_relation_reads() {
    let _db = owner_fixture().await;
    let one = RdOwner::find(1).await.unwrap().unwrap();
    let forty_two = RdOwner::find(42).await.unwrap().unwrap();

    // Pivot writes go through the declared key as well.
    one.teams().attach(1).await.unwrap();
    forty_two.teams().attach(2).await.unwrap();

    for (owner, label) in [(&one, "of one"), (&forty_two, "of forty-two")] {
        let items = owner.items().get().await.unwrap();
        assert_eq!(labels(items.iter().map(|i| &i.label)), vec![label]);
        let badge = owner.badge().first().await.unwrap().unwrap();
        assert_eq!(badge.label, label);
        let teams = owner.teams().get().await.unwrap();
        assert_eq!(labels(teams.iter().map(|t| &t.label)), vec![label]);
        let tasks = owner.tasks().get().await.unwrap();
        assert_eq!(labels(tasks.iter().map(|t| &t.label)), vec![label]);
    }

    let eager = RdOwner::query()
        .with(["items", "badge", "teams", "tasks"])
        .with_count(["items", "teams", "tasks"])
        .order_by("id", suprnova::Direction::Asc)
        .get()
        .await
        .unwrap();
    for (owner, label) in eager.iter().zip(["of one", "of forty-two"]) {
        assert_eq!(
            labels(owner.items_loaded().iter().map(|i| &i.label)),
            vec![label]
        );
        assert_eq!(owner.badge_loaded().map(|b| b.label.as_str()), Some(label));
        assert_eq!(
            labels(owner.teams_loaded().iter().map(|t| &t.label)),
            vec![label]
        );
        assert_eq!(
            labels(owner.tasks_loaded().iter().map(|t| &t.label)),
            vec![label]
        );
        assert_eq!(owner.items_count(), 1);
        assert_eq!(owner.teams_count(), 1);
        assert_eq!(owner.tasks_count(), 1);
    }
}

// ---- Declared related key of a many-to-many -----------------------------

#[model(table = "rd_members", relations = {
    medals: BelongsToMany<RdMedal, RdMemberMedal> {
        related_key = "code",
        pivot_related_key = "medal_code",
    },
})]
pub struct RdMember {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_medals")]
pub struct RdMedal {
    pub id: i64,
    pub code: String,
    pub label: String,
}

#[model(table = "rd_member_medal", timestamps = false)]
pub struct RdMemberMedal {
    pub id: i64,
    pub rd_member_id: i64,
    pub medal_code: String,
}

/// The pivot holds medal codes, so an eager load matches them against
/// `code`, as the lazy read does, not against `id`.
#[tokio::test]
async fn eager_many_to_many_matches_on_the_declared_related_key() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_members (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_medals (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, \
            label TEXT NOT NULL)",
        "CREATE TABLE rd_member_medal (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_member_id INTEGER NOT NULL, medal_code TEXT NOT NULL)",
        "INSERT INTO rd_members (id, name) VALUES (1, 'mo')",
        "INSERT INTO rd_medals (id, code, label) VALUES (1, 'gold', 'Gold'), (2, 'silver', 'Silver')",
        "INSERT INTO rd_member_medal (rd_member_id, medal_code) VALUES (1, 'silver')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let member = RdMember::find(1).await.unwrap().unwrap();
    let lazy = member.medals().get().await.unwrap();
    assert_eq!(labels(lazy.iter().map(|m| &m.label)), vec!["Silver"]);

    let eager = RdMember::query().with(["medals"]).get().await.unwrap();
    let medals = eager[0].medals_loaded();
    assert_eq!(labels(medals.iter().map(|m| &m.label)), vec!["Silver"]);
    assert_eq!(medals[0].pivot::<RdMemberMedal>().medal_code, "silver");
}

// ---- Pivot casts and the pivot table override ---------------------------

#[model(table = "rd_accounts", morph_type = "account", relations = {
    vaults: BelongsToMany<RdVault, RdAccountVault>,
    labels: MorphToMany<RdLabel, RdLabelled> { name = "labelled" },
})]
pub struct RdAccount {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_vaults")]
pub struct RdVault {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_account_vault", timestamps = false, casts = { note = AsEncrypted })]
pub struct RdAccountVault {
    pub id: i64,
    pub rd_account_id: i64,
    pub rd_vault_id: i64,
    pub note: String,
}

#[model(table = "rd_labels")]
pub struct RdLabel {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_labelled", timestamps = false, casts = { note = AsEncrypted })]
pub struct RdLabelled {
    pub id: i64,
    pub rd_label_id: i64,
    pub labelled_id: i64,
    pub labelled_type: String,
    pub note: String,
}

/// `attach_with` writes a pivot extra through the pivot model's cast:
/// an `AsEncrypted` note is stored as ciphertext and reads back as the
/// plaintext, on a `BelongsToMany` and on a `MorphToMany`.
#[cfg(feature = "testing")]
#[tokio::test]
async fn attach_with_writes_pivot_extras_through_the_pivot_casts() {
    crate::key_ring::rotation_keys();
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_accounts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_vaults (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_account_vault (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_account_id INTEGER NOT NULL, rd_vault_id INTEGER NOT NULL, note TEXT NOT NULL)",
        "CREATE TABLE rd_labels (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_labelled (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_label_id INTEGER NOT NULL, labelled_id INTEGER NOT NULL, \
            labelled_type TEXT NOT NULL, note TEXT NOT NULL)",
        "INSERT INTO rd_accounts (id, name) VALUES (1, 'acct')",
        "INSERT INTO rd_vaults (id, name) VALUES (1, 'vault')",
        "INSERT INTO rd_labels (id, name) VALUES (1, 'label')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let account = RdAccount::find(1).await.unwrap().unwrap();
    account
        .vaults()
        .attach_with(1, attrs! { note: "the vault secret" })
        .await
        .unwrap();
    account
        .labels()
        .attach_with(1, attrs! { note: "the label secret" })
        .await
        .unwrap();

    for (table, plaintext) in [
        ("rd_account_vault", "the vault secret"),
        ("rd_labelled", "the label secret"),
    ] {
        let row = db
            .fetch_one(&format!("SELECT note FROM {table}"), vec![])
            .await
            .unwrap();
        let stored: String = row.try_get("", "note").unwrap();
        assert_ne!(stored, plaintext, "{table} stores the note encrypted");
    }

    let vaults = account.vaults().get().await.unwrap();
    assert_eq!(vaults[0].pivot::<RdAccountVault>().note, "the vault secret");
    let labelled = account.labels().get().await.unwrap();
    assert_eq!(labelled[0].pivot::<RdLabelled>().note, "the label secret");
}

#[model(table = "rd_groups", morph_type = "group", relations = {
    perms: BelongsToMany<RdPerm, RdGroupPerm> { pivot_table = "rd_group_perm_actual" },
    tags: MorphToMany<RdTag, RdTagged> { name = "tagged", pivot_table = "rd_tagged_actual" },
})]
pub struct RdGroup {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_perms")]
pub struct RdPerm {
    pub id: i64,
    pub name: String,
}

/// Declared over a template table; the relation names the table it
/// actually uses.
#[model(table = "rd_group_perm_template", timestamps = false)]
pub struct RdGroupPerm {
    pub id: i64,
    pub rd_group_id: i64,
    pub rd_perm_id: i64,
    pub level: i64,
}

#[model(table = "rd_tags", relations = {
    groups: MorphedByMany<RdGroup, RdTagged> {
        name = "tagged",
        target_morph_type = "group",
        pivot_table = "rd_tagged_actual",
    },
})]
pub struct RdTag {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_tagged_template", timestamps = false)]
pub struct RdTagged {
    pub id: i64,
    pub rd_tag_id: i64,
    pub tagged_id: i64,
    pub tagged_type: String,
    pub level: i64,
}

/// The pivot context comes from the relation's own pivot table, not
/// from the table the pivot model is declared over: lazy, eager, and on
/// the inverse side of a polymorphic many-to-many.
#[tokio::test]
async fn pivot_context_is_read_from_the_relations_pivot_table() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_groups (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_perms (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "INSERT INTO rd_groups (id, name) VALUES (1, 'group')",
        "INSERT INTO rd_perms (id, name) VALUES (1, 'perm')",
        "INSERT INTO rd_tags (id, name) VALUES (1, 'tag')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    for table in ["rd_group_perm_template", "rd_group_perm_actual"] {
        db.execute_unprepared(&format!(
            "CREATE TABLE {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                rd_group_id INTEGER NOT NULL, rd_perm_id INTEGER NOT NULL, \
                level INTEGER NOT NULL)"
        ))
        .await
        .unwrap();
    }
    for table in ["rd_tagged_template", "rd_tagged_actual"] {
        db.execute_unprepared(&format!(
            "CREATE TABLE {table} (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                rd_tag_id INTEGER NOT NULL, tagged_id INTEGER NOT NULL, \
                tagged_type TEXT NOT NULL, level INTEGER NOT NULL)"
        ))
        .await
        .unwrap();
    }
    // The template tables hold an unrelated row each, with a level the
    // relation must never report.
    db.execute_unprepared(
        "INSERT INTO rd_group_perm_template (rd_group_id, rd_perm_id, level) VALUES (1, 1, 99)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "INSERT INTO rd_tagged_template (rd_tag_id, tagged_id, tagged_type, level) \
            VALUES (1, 1, 'group', 99)",
    )
    .await
    .unwrap();

    let group = RdGroup::find(1).await.unwrap().unwrap();
    group
        .perms()
        .attach_with(1, attrs! { level: 5 })
        .await
        .unwrap();
    group
        .tags()
        .attach_with(1, attrs! { level: 6 })
        .await
        .unwrap();

    let perms = group.perms().get().await.unwrap();
    assert_eq!(perms[0].pivot::<RdGroupPerm>().level, 5);
    let tags = group.tags().get().await.unwrap();
    assert_eq!(tags[0].pivot::<RdTagged>().level, 6);
    let tag = RdTag::find(1).await.unwrap().unwrap();
    let groups = tag.groups().get().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].pivot::<RdTagged>().level, 6);

    let eager = RdGroup::query()
        .with(["perms", "tags"])
        .get()
        .await
        .unwrap();
    assert_eq!(eager[0].perms_loaded()[0].pivot::<RdGroupPerm>().level, 5);
    assert_eq!(eager[0].tags_loaded()[0].pivot::<RdTagged>().level, 6);
    let eager_tags = RdTag::query().with(["groups"]).get().await.unwrap();
    let eager_groups = eager_tags[0].groups_loaded();
    assert_eq!(eager_groups.len(), 1);
    assert_eq!(eager_groups[0].pivot::<RdTagged>().level, 6);
}

// ---- Eager loads follow the parent query's routing ----------------------

#[model(table = "rd_carts", relations = {
    entries: HasMany<RdEntry>,
})]
pub struct RdCart {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_entries")]
pub struct RdEntry {
    pub id: i64,
    pub rd_cart_id: i64,
    pub qty: i64,
}

/// A child written through an explicit transaction is loaded by an eager
/// load of a parent query given the same transaction, and so are its
/// count and total. The test pool has one connection, which the
/// transaction holds, so a read sent to the pool would wait for it.
#[tokio::test]
async fn eager_loads_run_on_the_parent_querys_transaction() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_carts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_cart_id INTEGER NOT NULL, qty INTEGER NOT NULL)",
        "INSERT INTO rd_carts (id, name) VALUES (1, 'cart')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let tx = DB::begin_transaction().await.unwrap();
    RdEntry::create_with_tx(&tx, attrs! { rd_cart_id: 1, qty: 3 })
        .await
        .unwrap();

    let carts = tokio::time::timeout(
        STUCK,
        RdCart::query()
            .with_tx(&tx)
            .with(["entries"])
            .with_count(["entries"])
            .with_sum(("entries", "qty"))
            .get(),
    )
    .await
    .expect("the eager load ran on the transaction, not on the held pool")
    .unwrap();
    assert_eq!(
        carts[0].entries_loaded().len(),
        1,
        "the uncommitted child is seen"
    );
    assert_eq!(carts[0].entries_count(), 1);
    assert_eq!(carts[0].entries_sum_of("qty"), Some(3.0));
    tx.rollback().await.unwrap();
}

#[model(table = "rd_routed_carts", relations = {
    entries: HasMany<RdRoutedEntry>,
})]
pub struct RdRoutedCart {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_routed_entries")]
pub struct RdRoutedEntry {
    pub id: i64,
    pub rd_routed_cart_id: i64,
    pub qty: i64,
}

/// A parent query sent to a named connection with `on(name)` loads its
/// relations from that connection too, when the related model declares
/// no connection of its own.
#[tokio::test]
#[serial]
async fn eager_loads_follow_the_parent_querys_connection() {
    let _primary = TestDatabase::sqlite_memory().await.unwrap();
    let archive = sea_orm::Database::connect("sqlite::memory:?mode=rwc")
        .await
        .unwrap();
    let archive = DbConnection::from_raw(archive);
    for sql in [
        "CREATE TABLE rd_routed_carts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_routed_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_routed_cart_id INTEGER NOT NULL, qty INTEGER NOT NULL)",
        "INSERT INTO rd_routed_carts (id, name) VALUES (1, 'archived')",
        "INSERT INTO rd_routed_entries (rd_routed_cart_id, qty) VALUES (1, 4), (1, 5)",
    ] {
        use sea_orm::ConnectionTrait;
        archive.inner().execute_unprepared(sql).await.unwrap();
    }
    // The primary has the tables too, empty, so a read sent there finds
    // nothing rather than failing.
    for sql in [
        "CREATE TABLE rd_routed_carts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_routed_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_routed_cart_id INTEGER NOT NULL, qty INTEGER NOT NULL)",
    ] {
        _primary.execute_unprepared(sql).await.unwrap();
    }
    ConnectionRegistry::register_existing("rd_routed_archive", archive.clone())
        .await
        .unwrap();

    let carts = RdRoutedCart::query()
        .on("rd_routed_archive")
        .with(["entries"])
        .with_count(["entries"])
        .get()
        .await;
    ConnectionRegistry::clear();
    let carts = carts.unwrap();
    assert_eq!(carts.len(), 1);
    assert_eq!(carts[0].entries_loaded().len(), 2);
    assert_eq!(carts[0].entries_count(), 2);
}

// ---- Mass prune deletes on the model's connection -----------------------

#[model(
    table = "rd_archive_logs",
    connection = "rd_prune_archive",
    timestamps = false
)]
pub struct RdArchiveLog {
    pub id: i64,
    pub message: String,
}

#[suprnova::prunable]
#[async_trait]
impl MassPrunable for RdArchiveLog {
    fn prunable() -> Builder<Self> {
        Self::query().filter_op("id", ">", 0)
    }
}

/// A `MassPrunable` model declared on a named connection is pruned on
/// that connection, where its dry run counted, and never on the primary.
#[tokio::test]
#[serial]
async fn mass_prune_deletes_on_the_models_connection() {
    let primary = TestDatabase::sqlite_memory().await.unwrap();
    primary
        .execute_unprepared(
            "CREATE TABLE rd_archive_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                message TEXT NOT NULL)",
        )
        .await
        .unwrap();
    primary
        .execute_unprepared("INSERT INTO rd_archive_logs (message) VALUES ('primary row')")
        .await
        .unwrap();
    let archive = sea_orm::Database::connect("sqlite::memory:?mode=rwc")
        .await
        .unwrap();
    let archive = DbConnection::from_raw(archive);
    for sql in [
        "CREATE TABLE rd_archive_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, message TEXT NOT NULL)",
        "INSERT INTO rd_archive_logs (message) VALUES ('archived 1'), ('archived 2')",
    ] {
        use sea_orm::ConnectionTrait;
        archive.inner().execute_unprepared(sql).await.unwrap();
    }
    ConnectionRegistry::register_existing("rd_prune_archive", archive.clone())
        .await
        .unwrap();

    let dry = suprnova::eloquent::prune_one("RdArchiveLog", true).await;
    let pruned = suprnova::eloquent::prune_one("RdArchiveLog", false).await;
    let left_in_archive = RdArchiveLog::query().count().await;
    ConnectionRegistry::clear();
    assert_eq!(dry.unwrap(), Some(2));
    assert_eq!(
        pruned.unwrap(),
        Some(2),
        "the archive's two rows are pruned"
    );
    assert_eq!(left_in_archive.unwrap(), 0);
    let primary_rows = primary
        .fetch_one("SELECT COUNT(*) AS n FROM rd_archive_logs", vec![])
        .await
        .unwrap();
    let n: i64 = primary_rows.try_get("", "n").unwrap();
    assert_eq!(n, 1, "the primary's row is untouched");
}

// ---- Factory inserts of SeaORM rows join the transaction ----------------

#[model(table = "rd_seeded", timestamps = false)]
pub struct RdSeeded {
    pub id: i64,
    pub name: String,
}

/// Produces the SeaORM row of `RdSeeded`, the shape the blanket
/// `Persistable` impl inserts.
pub struct RdSeededRowFactory;

impl Factory for RdSeededRowFactory {
    type Model = rd_seeded::Model;

    fn definition() -> rd_seeded::Model {
        rd_seeded::Model {
            id: 0,
            name: "seeded".into(),
        }
    }
}

/// Factory rows created inside `DB::transaction` are inserted in that
/// transaction, so a rollback removes them. The test pool has one
/// connection, which the transaction holds, so an insert sent to the
/// pool would wait for it.
#[tokio::test]
async fn factory_rows_join_the_surrounding_transaction() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rd_seeded (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .unwrap();

    let outcome = tokio::time::timeout(
        STUCK,
        DB::transaction(|_tx| {
            Box::pin(async move {
                let rows = RdSeededRowFactory::new().count(2).create_many().await?;
                assert_eq!(rows.len(), 2);
                assert_eq!(
                    RdSeeded::query().count().await?,
                    2,
                    "seen inside the transaction"
                );
                Err::<(), _>(suprnova::FrameworkError::internal("roll back"))
            })
        }),
    )
    .await
    .expect("the factory inserts ran on the transaction, not on the held pool");
    assert!(outcome.is_err(), "the closure's error rolls back");
    assert_eq!(
        RdSeeded::query().count().await.unwrap(),
        0,
        "the rollback removed them"
    );
}
