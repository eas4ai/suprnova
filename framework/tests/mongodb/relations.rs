//! PAR-185: documents relate with `HasOne`, `HasMany`, `BelongsTo` and
//! `BelongsToMany` (the many-to-many kept as arrays of keys on both
//! sides), eager-loaded with `with(..)` in one query per relation; across
//! stores an SQL `#[model]` declares `HasManyDocuments<D>` and
//! `HasOneDocument<D>` and a document declares `BelongsToModel<M>`, each
//! loaded through its own store and eager-loadable; and a relation whose
//! key types cannot match does not compile, naming both models.
//!
//! The tests without the `mongodb_` prefix run without a server: the
//! lookup each relation renders, the declared relations, the calls that
//! answer before any query, and the compile errors. The `mongodb_` tests
//! need `MONGODB_TEST_URL`, and the cross-store ones keep the SQL side in
//! an in-memory SQLite database.

use futures::TryStreamExt;
use suprnova::bson::oid::ObjectId;
use suprnova::bson::{Bson, Document, doc};
use suprnova::sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{DB, DocumentModel, FrameworkError, Model, Mongo, MongoConfig, model};

use crate::support::{database_of, test_url};

// --- Document models -------------------------------------------------------

/// A writer: posts and a profile point at it, and it keeps the keys of its
/// roles in `rel_role_ids`.
#[suprnova::document(
    collection = "m4_rel_writers",
    fillable = ["name", "team"],
    relations = {
        posts: HasMany<RelPost>,
        profile: HasOne<RelProfile>,
        roles: BelongsToMany<RelRole>,
    }
)]
pub struct RelWriter {
    pub name: String,
    pub team: String,
    pub rel_role_ids: Vec<ObjectId>,
}

/// A post points at its writer by `rel_writer_id`.
#[suprnova::document(
    collection = "m4_rel_posts",
    fillable = ["rel_writer_id", "title"],
    relations = { writer: BelongsTo<RelWriter> }
)]
pub struct RelPost {
    pub rel_writer_id: ObjectId,
    pub title: String,
}

/// A profile whose writer is optional.
#[suprnova::document(
    collection = "m4_rel_profiles",
    fillable = ["rel_writer_id", "bio"],
    relations = { writer: BelongsTo<RelWriter> }
)]
pub struct RelProfile {
    pub rel_writer_id: Option<ObjectId>,
    pub bio: String,
}

/// A role keeps the keys of its writers in `rel_writer_ids`.
#[suprnova::document(
    collection = "m4_rel_roles",
    fillable = ["name"],
    relations = { writers: BelongsToMany<RelWriter> }
)]
pub struct RelRole {
    pub name: String,
    pub rel_writer_ids: Vec<ObjectId>,
}

/// A team keyed by its slug; its members name it in `team_slug`.
#[suprnova::document(
    collection = "m4_rel_teams",
    primary_key = "slug",
    fillable = ["slug", "title"],
    relations = { members: HasMany<RelMember> { fk = "team_slug" } }
)]
pub struct RelTeam {
    pub slug: String,
    pub title: String,
}

/// A member of a team, or of none.
#[suprnova::document(
    collection = "m4_rel_members",
    fillable = ["team_slug", "name"],
    relations = { team: BelongsTo<RelTeam> { fk = "team_slug" } }
)]
pub struct RelMember {
    pub team_slug: Option<String>,
    pub name: String,
}

/// A role whose arrays have names of their own: it keeps its holders in
/// `holders` and each holder keeps it in `badges`.
#[suprnova::document(
    collection = "m4_rel_badges",
    fillable = ["label"],
    relations = {
        holders: BelongsToMany<RelHolder> {
            pivot_foreign_key = "badges",
            pivot_related_key = "holders",
        },
    }
)]
pub struct RelBadge {
    pub label: String,
    pub holders: Vec<ObjectId>,
}

/// The other side of [`RelBadge`].
#[suprnova::document(
    collection = "m4_rel_holders",
    fillable = ["name"],
    relations = {
        badges: BelongsToMany<RelBadge> {
            pivot_foreign_key = "holders",
            pivot_related_key = "badges",
        },
    }
)]
pub struct RelHolder {
    pub name: String,
    pub badges: Vec<ObjectId>,
}

// --- Across stores ---------------------------------------------------------

/// An SQL account whose notes and setting are documents.
#[model(table = "rel_accounts", relations = {
    notes: HasManyDocuments<RelNote>,
    setting: HasOneDocument<RelSetting>,
})]
pub struct RelAccount {
    pub id: i64,
    pub name: String,
}

/// A note document that belongs to an SQL account.
#[suprnova::document(
    collection = "m4_rel_notes",
    fillable = ["rel_account_id", "body"],
    relations = { account: BelongsToModel<RelAccount> }
)]
pub struct RelNote {
    pub rel_account_id: i64,
    pub body: String,
}

/// A setting document whose account is optional.
#[suprnova::document(
    collection = "m4_rel_settings",
    fillable = ["rel_account_id", "theme"],
    relations = { account: BelongsToModel<RelAccount> }
)]
pub struct RelSetting {
    pub rel_account_id: Option<i64>,
    pub theme: String,
}

fn writer(name: &str) -> RelWriter {
    RelWriter::make(doc! { "name": name, "team": "t" }).expect("make a writer")
}

fn account(id: i64) -> RelAccount {
    RelAccount {
        id,
        name: format!("account {id}"),
        ..Default::default()
    }
}

// --- Rendered lookups ------------------------------------------------------

#[test]
fn has_many_looks_up_the_children_whose_foreign_key_holds_the_parents_key() {
    let ada = writer("Ada");
    let posts = ada.posts();
    assert_eq!(posts.foreign_key(), "rel_writer_id");
    let rendered = posts.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_writer_id": ada.id });

    // Another writer's lookup names its own key, never the first one's.
    let grace = writer("Grace");
    let rendered = grace.posts().query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_writer_id": grace.id });
}

#[test]
fn has_one_looks_up_by_the_same_key_and_reads_one_document() {
    let ada = writer("Ada");
    let profile = ada.profile();
    assert_eq!(profile.foreign_key(), "rel_writer_id");
    let rendered = profile.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_writer_id": ada.id });
}

#[test]
fn belongs_to_looks_up_the_owner_by_its_key() {
    let ada = writer("Ada");
    let post =
        RelPost::make(doc! { "rel_writer_id": ada.id, "title": "Notes" }).expect("make a post");
    let owner = post.writer();
    assert_eq!(owner.foreign_key(), "rel_writer_id");
    assert_eq!(owner.owner_key(), "id");
    let rendered = owner.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "_id": ada.id });
}

#[test]
fn foreign_and_local_keys_can_be_named() {
    let team = RelTeam::make(doc! { "slug": "core", "title": "Core" }).expect("make a team");
    let rendered = team.members().query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "team_slug": "core" });

    let member =
        RelMember::make(doc! { "team_slug": "core", "name": "Ada" }).expect("make a member");
    let rendered = member.team().query().to_filter().expect("render");
    assert_eq!(
        rendered.filter,
        doc! { "_id": "core" },
        "the slug is the key"
    );
}

#[test]
fn belongs_to_many_reads_the_array_on_the_related_side_from_both_sides() {
    let ada = writer("Ada");
    let roles = ada.roles();
    assert_eq!(roles.foreign_pivot_key(), "rel_writer_ids");
    assert_eq!(roles.related_pivot_key(), "rel_role_ids");
    let rendered = roles.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_writer_ids": ada.id });

    let editor = RelRole::make(doc! { "name": "editor" }).expect("make a role");
    let writers = editor.writers();
    assert_eq!(writers.foreign_pivot_key(), "rel_role_ids");
    assert_eq!(writers.related_pivot_key(), "rel_writer_ids");
    let rendered = writers.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_role_ids": editor.id });

    let badge = RelBadge::make(doc! { "label": "gold" }).expect("make a badge");
    let rendered = badge.holders().query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "badges": badge.id }, "named arrays");
}

#[test]
fn an_sql_model_looks_up_its_documents_by_its_key() {
    let account = account(7);
    let notes = account.notes();
    assert_eq!(notes.foreign_key(), "rel_account_id");
    let rendered = notes.query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_account_id": 7_i64 });

    let rendered = account.setting().query().to_filter().expect("render");
    assert_eq!(rendered.filter, doc! { "rel_account_id": 7_i64 });
}

#[test]
fn a_document_looks_up_its_sql_model_through_the_sql_builder() {
    let note = RelNote::make(doc! { "rel_account_id": 7_i64, "body": "hi" }).expect("make a note");
    let owner = note.account();
    assert_eq!(owner.foreign_key(), "rel_account_id");
    assert_eq!(owner.owner_key(), "id");
    let (sql, bindings) = owner.query().to_sql_with_bindings_for(DbBackend::Sqlite);
    assert_eq!(sql, "SELECT * FROM rel_accounts WHERE id = ?");
    assert_eq!(bindings.len(), 1, "{sql}");
    assert_eq!(
        bindings[0],
        suprnova::sea_orm::Value::BigInt(Some(7)),
        "the account's key is bound"
    );
}

// --- Before any query ------------------------------------------------------

#[test]
fn the_model_lists_its_relations_and_reports_none_loaded() {
    assert_eq!(RelWriter::RELATIONS, &["posts", "profile", "roles"]);
    assert_eq!(RelPost::RELATIONS, &["writer"]);
    assert!(RelTeam::RELATIONS.contains(&"members"));
    let ada = writer("Ada");
    assert!(!ada.relation_loaded("posts"));
    assert!(ada.posts_loaded().is_none(), "not loaded is not empty");
    assert!(ada.profile_loaded().is_none());
    assert!(ada.roles_loaded().is_none());
    let account = account(1);
    assert!(account.notes_loaded().is_none());
    assert!(account.setting_loaded().is_none());
}

#[tokio::test]
async fn a_relation_the_model_does_not_declare_is_an_error_naming_it_before_any_query() {
    // No connection is registered in this process: the error comes first.
    let error = RelWriter::query()
        .with(["comments"])
        .get()
        .await
        .expect_err("an unknown relation");
    assert!(error.to_string().contains("comments"), "{error}");
    assert!(error.to_string().contains("RelWriter"), "{error}");

    let error = RelWriter::query()
        .with(["posts", "writer.posts"])
        .first()
        .await
        .expect_err("an unknown relation in a path");
    assert!(error.to_string().contains("writer"), "{error}");
}

#[tokio::test]
async fn a_missing_foreign_key_answers_no_owner_without_a_query() {
    // No connection is registered: an owner lookup that queried would fail.
    let profile = RelProfile::make(doc! { "bio": "unattached" }).expect("make a profile");
    assert!(profile.writer().get().await.expect("no query").is_none());
    let rendered = profile.writer().query().to_filter().expect("render");
    assert_eq!(
        rendered.filter,
        doc! { "_id": { "$in": [] } },
        "a lookup without a key matches nothing"
    );

    let setting = RelSetting::make(doc! { "theme": "dark" }).expect("make a setting");
    assert!(setting.account().get().await.expect("no query").is_none());
}

// --- Key types -------------------------------------------------------------

/// Relations whose key types cannot match fail the build, and each error
/// names both models: a writer's `ObjectId` against a `String` foreign key,
/// a role array of strings, an SQL account's `i64` key against an
/// `ObjectId` foreign key in both directions. The glob runs every case in
/// `compile_fail/`, so it also runs the document models' other build
/// errors, such as `timestamps = true` on a model without the fields.
#[test]
fn relation_key_types_that_cannot_match_fail_to_compile_naming_both_models() {
    trybuild::TestCases::new().compile_fail("tests/mongodb/compile_fail/*.rs");
}

// --- Against a server ------------------------------------------------------

async fn connect() {
    let url = test_url();
    let config = MongoConfig::builder()
        .uri(url.clone())
        .database(database_of(&url))
        .build()
        .expect("the test configuration");
    Mongo::init_with(config).await.expect("connect");
}

async fn remove(collection: &str, filter: Document) {
    Mongo::collection::<Document>(collection)
        .expect("collection")
        .delete_many(filter)
        .await
        .expect("remove the test's documents");
}

/// How many queries scanned `collection` since the server started. Each
/// find on an unindexed field scans the collection once, so the change
/// across a call counts its queries.
async fn collection_scans(collection: &str) -> i64 {
    let stats: Vec<Document> = Mongo::collection::<Document>(collection)
        .expect("collection")
        .aggregate(vec![doc! { "$collStats": { "queryExecStats": {} } }])
        .await
        .expect("collStats")
        .try_collect()
        .await
        .expect("read collStats");
    let total = stats
        .first()
        .and_then(|stats| stats.get_document("queryExecStats").ok())
        .and_then(|exec| exec.get_document("collectionScans").ok())
        .and_then(|scans| scans.get("total").cloned());
    match total {
        Some(Bson::Int64(count)) => count,
        Some(Bson::Int32(count)) => i64::from(count),
        other => panic!("collStats has no collection scan count: {other:?}"),
    }
}

/// A writer the query-count test alone writes, in collections of its own,
/// so no other test's queries reach the counts.
#[suprnova::document(
    collection = "m4_tally_writers",
    fillable = ["name", "team"],
    relations = { posts: HasMany<TallyPost> }
)]
pub struct TallyWriter {
    pub name: String,
    pub team: String,
}

/// A post of a [`TallyWriter`].
#[suprnova::document(collection = "m4_tally_posts", fillable = ["tally_writer_id", "title"])]
pub struct TallyPost {
    pub tally_writer_id: ObjectId,
    pub title: String,
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_has_many_answers_only_its_own_and_with_loads_ten_parents_in_two_queries() {
    connect().await;
    remove(TallyWriter::COLLECTION, doc! {}).await;
    remove(TallyPost::COLLECTION, doc! {}).await;

    // Ten writers of team `t` with 0 to 9 posts each, and one writer of
    // another team with three, so a lookup that ignored the key would
    // answer posts of another writer.
    let mut writers = Vec::new();
    for index in 0..10 {
        let writer = TallyWriter::create(doc! { "name": format!("w{index}"), "team": "t" })
            .await
            .expect("create a writer");
        for post in 0..index {
            TallyPost::create(doc! {
                "tally_writer_id": writer.id,
                "title": format!("w{index}-p{post}"),
            })
            .await
            .expect("create a post");
        }
        writers.push(writer);
    }
    let outsider = TallyWriter::create(doc! { "name": "outsider", "team": "other" })
        .await
        .expect("create the outsider");
    for post in 0..3 {
        TallyPost::create(doc! { "tally_writer_id": outsider.id, "title": format!("o-p{post}") })
            .await
            .expect("create an outsider post");
    }

    let own = writers[3].posts().get().await.expect("lazy posts");
    assert_eq!(own.len(), 3);
    assert!(
        own.iter().all(|post| post.title.starts_with("w3-")),
        "{own:?}"
    );
    assert_eq!(writers[0].posts().count().await.expect("count"), 0);

    let writer_scans = collection_scans(TallyWriter::COLLECTION).await;
    let post_scans = collection_scans(TallyPost::COLLECTION).await;
    let loaded = TallyWriter::query()
        .where_("team", "=", "t")
        .with(["posts"])
        .get()
        .await
        .expect("eager posts");
    assert_eq!(
        collection_scans(TallyWriter::COLLECTION).await - writer_scans,
        1,
        "one query reads the writers"
    );
    assert_eq!(
        collection_scans(TallyPost::COLLECTION).await - post_scans,
        1,
        "one query reads the posts of all ten writers"
    );

    assert_eq!(loaded.len(), 10);
    for writer in loaded.iter() {
        let index: usize = writer.name[1..].parse().expect("the writer's index");
        let posts = writer.posts_loaded().expect("posts are loaded");
        assert_eq!(posts.len(), index, "{} owns {index} posts", writer.name);
        assert!(
            posts
                .iter()
                .all(|post| post.title.starts_with(&format!("{}-", writer.name))),
            "{} got another writer's post: {posts:?}",
            writer.name
        );
        assert!(writer.relation_loaded("posts"));
    }
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_has_one_and_belongs_to_load_lazily_and_eagerly() {
    connect().await;
    let slug = format!("core-{}", ObjectId::new().to_hex());
    let team = RelTeam::create(doc! { "slug": slug.as_str(), "title": "Core" })
        .await
        .expect("create a team");
    let member = RelMember::create(doc! { "team_slug": slug.as_str(), "name": "Ada" })
        .await
        .expect("create a member");
    let loner = RelMember::create(doc! { "name": "Loner" })
        .await
        .expect("create a member without a team");

    let members = team.members().get().await.expect("lazy members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].id, member.id);
    let owner = member
        .team()
        .get()
        .await
        .expect("lazy team")
        .expect("a team");
    assert_eq!(owner.slug, slug);
    assert!(loner.team().get().await.expect("no team").is_none());

    let loaded = RelMember::query()
        .where_in("id", [member.id, loner.id])
        .with(["team"])
        .get()
        .await
        .expect("eager teams");
    let by_name = |name: &str| loaded.iter().find(|m| m.name == name).expect("loaded");
    assert_eq!(
        by_name("Ada").team_loaded().map(|team| team.slug.as_str()),
        Some(slug.as_str())
    );
    assert!(by_name("Loner").relation_loaded("team"));
    assert!(by_name("Loner").team_loaded().is_none());

    let teams = RelTeam::query()
        .where_("slug", "=", slug.as_str())
        .with(["members"])
        .get()
        .await
        .expect("eager members");
    let loaded_members = teams[0].members_loaded().expect("members are loaded");
    assert_eq!(loaded_members.len(), 1);
    assert_eq!(loaded_members[0].name, "Ada");

    // HasOne, lazily and eagerly, and a document without one.
    let ada = RelWriter::create(doc! { "name": "Ada", "team": "has-one" })
        .await
        .expect("create a writer");
    let grace = RelWriter::create(doc! { "name": "Grace", "team": "has-one" })
        .await
        .expect("create a writer");
    let profile = ada
        .profile()
        .create(doc! { "bio": "Countess" })
        .await
        .expect("create the profile through the relation");
    assert_eq!(
        profile.rel_writer_id,
        Some(ada.id),
        "the relation sets the key"
    );
    let found = ada
        .profile()
        .get()
        .await
        .expect("lazy profile")
        .expect("a profile");
    assert_eq!(found.bio, "Countess");
    assert!(grace.profile().get().await.expect("lazy profile").is_none());
    let writers = RelWriter::query()
        .where_in("id", [ada.id, grace.id])
        .with(["profile"])
        .get()
        .await
        .expect("eager profiles");
    for writer in writers.iter() {
        match writer.name.as_str() {
            "Ada" => assert_eq!(
                writer.profile_loaded().map(|p| p.bio.as_str()),
                Some("Countess")
            ),
            _ => assert!(writer.profile_loaded().is_none()),
        }
    }
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_belongs_to_many_keeps_the_keys_on_both_sides() {
    connect().await;
    let ada = RelWriter::create(doc! { "name": "Ada", "team": "roles" })
        .await
        .expect("create a writer");
    let editor = RelRole::create(doc! { "name": "editor" })
        .await
        .expect("create a role");
    let admin = RelRole::create(doc! { "name": "admin" })
        .await
        .expect("create a role");

    let ada = ada
        .roles()
        .attach([editor.id, admin.id])
        .await
        .expect("attach");
    assert_eq!(
        ada.rel_role_ids,
        vec![editor.id, admin.id],
        "the writer's array"
    );
    let editor = editor.fresh().await.expect("fresh").expect("the role");
    assert_eq!(editor.rel_writer_ids, vec![ada.id], "the role's array");

    let mut names: Vec<String> = ada
        .roles()
        .get()
        .await
        .expect("the writer's roles")
        .iter()
        .map(|role| role.name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["admin", "editor"]);
    let writers = editor.writers().get().await.expect("the role's writers");
    assert_eq!(writers.len(), 1);
    assert_eq!(writers[0].id, ada.id, "visible from the role's side");

    // Attaching again changes nothing.
    let ada = ada.roles().attach([editor.id]).await.expect("attach again");
    assert_eq!(ada.rel_role_ids.len(), 2);

    let loaded = RelWriter::query()
        .where_("id", "=", ada.id)
        .with(["roles"])
        .get()
        .await
        .expect("eager roles");
    assert_eq!(loaded[0].roles_loaded().expect("loaded").len(), 2);
    let loaded = RelRole::query()
        .where_in("id", [editor.id, admin.id])
        .with(["writers"])
        .get()
        .await
        .expect("eager writers");
    for role in loaded.iter() {
        let writers = role.writers_loaded().expect("loaded");
        assert_eq!(writers.len(), 1, "{}", role.name);
        assert_eq!(writers[0].id, ada.id);
    }

    let ada = ada.roles().detach([editor.id]).await.expect("detach");
    assert_eq!(ada.rel_role_ids, vec![admin.id]);
    assert!(editor.writers().get().await.expect("writers").is_empty());

    let ada = ada.roles().sync([editor.id]).await.expect("sync");
    assert_eq!(ada.rel_role_ids, vec![editor.id]);
    assert_eq!(editor.writers().count().await.expect("count"), 1);
    assert_eq!(admin.writers().count().await.expect("count"), 0);

    let ada = ada.roles().detach_all().await.expect("detach all");
    assert!(ada.rel_role_ids.is_empty());
    assert_eq!(editor.writers().count().await.expect("count"), 0);

    // Arrays with names of their own.
    let holder = RelHolder::create(doc! { "name": "Ada" })
        .await
        .expect("create a holder");
    let gold = RelBadge::create(doc! { "label": "gold" })
        .await
        .expect("create a badge");
    let holder = holder.badges().attach([gold.id]).await.expect("attach");
    assert_eq!(holder.badges, vec![gold.id]);
    let gold = gold.fresh().await.expect("fresh").expect("the badge");
    assert_eq!(gold.holders, vec![holder.id]);
    assert_eq!(
        gold.holders().get().await.expect("holders")[0].id,
        holder.id
    );
}

/// The SQL side of the cross-store tests: an in-memory SQLite database with
/// the accounts table and two accounts whose keys no other test uses, so
/// the notes of one test never meet another's.
async fn accounts(first: i64) -> (TestDatabase, RelAccount, RelAccount) {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    db.execute_unprepared("CREATE TABLE rel_accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .expect("create the accounts table");
    db.execute_unprepared(&format!(
        "INSERT INTO rel_accounts (id, name) VALUES ({first}, 'first'), ({}, 'second')",
        first + 1
    ))
    .await
    .expect("insert the accounts");
    let one = RelAccount::find(first)
        .await
        .expect("find")
        .expect("the first");
    let two = RelAccount::find(first + 1)
        .await
        .expect("find")
        .expect("the second");
    connect().await;
    remove(
        RelNote::COLLECTION,
        doc! { "rel_account_id": { "$in": [first, first + 1] } },
    )
    .await;
    remove(
        RelSetting::COLLECTION,
        doc! { "rel_account_id": { "$in": [first, first + 1] } },
    )
    .await;
    (db, one, two)
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_an_sql_models_documents_load_through_the_collection() {
    let (_db, first, second) = accounts(410_100).await;
    for body in ["a", "b"] {
        RelNote::create(doc! { "rel_account_id": first.id, "body": body })
            .await
            .expect("create a note");
    }
    second
        .notes()
        .create(doc! { "body": "c" })
        .await
        .expect("create a note through the relation");
    first
        .setting()
        .create(doc! { "theme": "dark" })
        .await
        .expect("create a setting through the relation");

    let notes = first.notes().get().await.expect("lazy notes");
    let mut bodies: Vec<&str> = notes.iter().map(|note| note.body.as_str()).collect();
    bodies.sort_unstable();
    assert_eq!(bodies, ["a", "b"], "only the first account's notes");
    assert_eq!(second.notes().count().await.expect("count"), 1);
    assert_eq!(
        first
            .setting()
            .get()
            .await
            .expect("lazy setting")
            .map(|setting| setting.theme),
        Some("dark".to_owned())
    );
    assert!(second.setting().get().await.expect("no setting").is_none());

    let loaded = RelAccount::query()
        .where_in("id", [first.id, second.id])
        .order_by_asc("id")
        .with(["notes", "setting"])
        .with_count(["notes"])
        .get()
        .await
        .expect("eager documents");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].notes_loaded().expect("loaded").len(), 2);
    assert_eq!(loaded[1].notes_loaded().expect("loaded")[0].body, "c");
    assert_eq!(loaded[0].notes_count(), Some(2));
    assert_eq!(loaded[1].notes_count(), Some(1));
    assert_eq!(
        loaded[0]
            .setting_loaded()
            .map(|setting| setting.theme.as_str()),
        Some("dark")
    );
    assert!(loaded[1].setting_loaded().is_none());
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_documents_sql_model_loads_through_the_sql_connection() {
    let (_db, first, second) = accounts(410_200).await;
    let note = RelNote::create(doc! { "rel_account_id": first.id, "body": "a" })
        .await
        .expect("create a note");
    let other = RelNote::create(doc! { "rel_account_id": second.id, "body": "b" })
        .await
        .expect("create a note");
    let orphan = RelSetting::create(doc! { "theme": "light" })
        .await
        .expect("create a setting without an account");

    let owner = note
        .account()
        .get()
        .await
        .expect("lazy account")
        .expect("one");
    assert_eq!(owner.id, first.id);
    assert_eq!(owner.name, "first");
    assert!(orphan.account().get().await.expect("no account").is_none());

    DB::enable_query_log().expect("query log");
    DB::flush_query_log().expect("flush");
    let loaded = RelNote::query()
        .where_in("id", [note.id, other.id])
        .with(["account"])
        .get()
        .await
        .expect("eager accounts");
    assert_eq!(
        DB::get_query_log().expect("log").len(),
        1,
        "one SQL query reads the accounts of every note"
    );
    for note in loaded.iter() {
        let account = note.account_loaded().expect("loaded");
        assert_eq!(account.id, note.rel_account_id);
    }
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_nested_paths_cross_the_stores_both_ways() {
    let (_db, first, _second) = accounts(410_300).await;
    RelNote::create(doc! { "rel_account_id": first.id, "body": "nested" })
        .await
        .expect("create a note");

    // SQL, then documents, then SQL again.
    let loaded = RelAccount::query()
        .filter("id", first.id)
        .with(["notes.account"])
        .get()
        .await
        .expect("nested load");
    let notes = loaded[0].notes_loaded().expect("notes are loaded");
    assert_eq!(notes.len(), 1);
    assert_eq!(
        notes[0].account_loaded().map(|account| account.id),
        Some(first.id)
    );

    // Documents, then documents.
    let ada = RelWriter::create(doc! { "name": "Ada", "team": "nested" })
        .await
        .expect("create a writer");
    RelPost::create(doc! { "rel_writer_id": ada.id, "title": "first" })
        .await
        .expect("create a post");
    let loaded = RelWriter::query()
        .where_("id", "=", ada.id)
        .with(["posts.writer"])
        .get()
        .await
        .expect("nested document load");
    let posts = loaded[0].posts_loaded().expect("posts are loaded");
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].writer_loaded().map(|writer| writer.id),
        Some(ada.id)
    );
}

/// A cross-store load that cannot run is an error that says why, never a
/// panic or a silent miss: the document server unreachable, an SQL
/// aggregate over documents, and a `with_where` constraint, which takes an
/// SQL builder, on a relation to documents.
#[tokio::test]
async fn cross_store_loads_that_cannot_run_answer_errors_that_say_why() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    db.execute_unprepared("CREATE TABLE rel_accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .expect("create the accounts table");
    db.execute_unprepared("INSERT INTO rel_accounts (id, name) VALUES (1, 'only')")
        .await
        .expect("insert an account");

    // No MongoDB connection is registered in this process.
    let error: FrameworkError = RelAccount::query()
        .with(["notes"])
        .get()
        .await
        .expect_err("the documents cannot load");
    assert!(error.to_string().contains("MongoDB"), "{error}");

    let error = RelAccount::query()
        .with_sum(("notes", "words"))
        .get()
        .await
        .expect_err("no SQL aggregate over documents");
    assert!(
        error.to_string().contains("notes") && error.to_string().contains("MongoDB"),
        "{error}"
    );

    let error = RelAccount::query()
        .with_where(("notes", |query: suprnova::Builder<RelAccount>| query))
        .get()
        .await
        .expect_err("no SQL constraint on documents");
    assert!(
        error.to_string().contains("with_where") && error.to_string().contains("notes"),
        "{error}"
    );
}
