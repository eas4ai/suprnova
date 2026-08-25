//! Wave 6 T50 - `where_pivot` family on the three many-to-many
//! relations, plus the closure (nested group) form.
//!
//! Pins:
//!
//! - the filters narrow `get()` and `count()` on `BelongsToMany`,
//!   `MorphToMany` and `MorphedByMany`;
//! - the `or_` twins fold into a disjunction, and a closure group stays
//!   atomic inside that disjunction;
//! - a filter set on the relation makes `detach` / `sync` fail closed
//!   rather than silently ignore it;
//! - an illegal column name is rejected before it reaches SQL.

use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

// ---- Fixtures: BelongsToMany --------------------------------------------

#[model(table = "pf_users", relations = {
    roles: BelongsToMany<PfRole, PfRoleUser> {
        with_pivot = ["active", "note"],
    },
})]
pub struct PfUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "pf_roles")]
pub struct PfRole {
    pub id: i64,
    pub name: String,
}

#[model(table = "pf_role_user", primary_key = "id", timestamps = false)]
pub struct PfRoleUser {
    pub id: i64,
    pub pf_user_id: i64,
    pub pf_role_id: i64,
    pub active: i64,
    pub note: Option<String>,
}

// ---- Fixtures: MorphToMany / MorphedByMany ------------------------------

#[model(table = "pf_posts", morph_type = "pf_post", relations = {
    tags: MorphToMany<PfTag, PfTaggable> { name = "taggable" },
})]
pub struct PfPost {
    pub id: i64,
    pub title: String,
}

#[model(table = "pf_tags", relations = {
    posts: MorphedByMany<PfPost, PfTaggable> {
        name = "taggable",
        target_morph_type = "pf_post",
    },
})]
pub struct PfTag {
    pub id: i64,
    pub label: String,
}

#[model(table = "pf_taggables", primary_key = "id", timestamps = false)]
pub struct PfTaggable {
    pub id: i64,
    pub pf_tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
    pub active: i64,
}

async fn migrate(db: &TestDatabase) {
    db.execute_unprepared(
        "CREATE TABLE pf_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "CREATE TABLE pf_roles (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "CREATE TABLE pf_role_user (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            pf_user_id INTEGER NOT NULL, \
            pf_role_id INTEGER NOT NULL, \
            active INTEGER NOT NULL DEFAULT 0, \
            note TEXT, \
            UNIQUE(pf_user_id, pf_role_id)\
         )",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "CREATE TABLE pf_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "CREATE TABLE pf_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "CREATE TABLE pf_taggables (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            pf_tag_id INTEGER NOT NULL, \
            taggable_id INTEGER NOT NULL, \
            taggable_type TEXT NOT NULL, \
            active INTEGER NOT NULL DEFAULT 0\
         )",
    )
    .await
    .unwrap();
}

/// One user with three roles: admin (active, noted), editor (inactive,
/// noted), viewer (active, no note).
async fn seed_roles() -> (PfUser, PfRole, PfRole, PfRole) {
    let u = PfUser::create(attrs! { name: "Ada" }).await.unwrap();
    let admin = PfRole::create(attrs! { name: "admin" }).await.unwrap();
    let editor = PfRole::create(attrs! { name: "editor" }).await.unwrap();
    let viewer = PfRole::create(attrs! { name: "viewer" }).await.unwrap();

    u.roles()
        .attach_with(admin.id, attrs! { active: 1i64, note: "keep" })
        .await
        .unwrap();
    u.roles()
        .attach_with(editor.id, attrs! { active: 0i64, note: "keep" })
        .await
        .unwrap();
    u.roles()
        .attach_with(viewer.id, attrs! { active: 1i64 })
        .await
        .unwrap();

    (u, admin, editor, viewer)
}

fn ids(rows: &[PfRole]) -> Vec<i64> {
    rows.iter().map(|r| r.id).collect()
}

// ---- Happy path ---------------------------------------------------------

#[tokio::test]
async fn where_pivot_narrows_get_to_matching_pivot_rows() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _editor, viewer) = seed_roles().await;

    let active = u
        .roles()
        .where_pivot("active", 1i64)
        .get()
        .await
        .unwrap()
        .into_vec();

    let got = ids(&active);
    assert_eq!(got.len(), 2, "only the two active pivot rows, got {got:?}");
    assert!(got.contains(&admin.id));
    assert!(got.contains(&viewer.id));
}

#[tokio::test]
async fn where_pivot_narrows_count() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    assert_eq!(u.roles().count().await.unwrap(), 3, "unfiltered count");
    assert_eq!(
        u.roles().where_pivot("active", 1i64).count().await.unwrap(),
        2,
        "count must honour the same filter get() does",
    );
}

#[tokio::test]
async fn where_pivot_narrows_first() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _admin, editor, _viewer) = seed_roles().await;

    let row = u
        .roles()
        .where_pivot("active", 0i64)
        .first()
        .await
        .unwrap()
        .expect("the one inactive pivot row");
    assert_eq!(row.id, editor.id);
}

#[tokio::test]
async fn filtered_rows_still_carry_their_pivot_context() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    let rows = u
        .roles()
        .where_pivot_not_null("note")
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(rows.len(), 2);
    for r in &rows {
        // `pivot::<P>()` panics when `__pivot` is empty, so reaching
        // the assertion is itself the proof that a filtered read still
        // stamps the pivot context.
        let pivot = r.pivot::<PfRoleUser>();
        assert_eq!(pivot.note.as_deref(), Some("keep"));
    }
}

#[tokio::test]
async fn an_unfiltered_read_is_unchanged() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    let rows = u.roles().get().await.unwrap().into_vec();
    assert_eq!(rows.len(), 3);
    for r in &rows {
        // Panics if `__pivot` is empty - every row still carries it.
        assert_eq!(r.pivot::<PfRoleUser>().pf_role_id, r.id);
    }
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

// ---- Edge cases: the rest of the family ---------------------------------

#[tokio::test]
async fn where_pivot_in_and_not_in_select_complementary_sets() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, editor, viewer) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_in("pf_role_id", [admin.id, viewer.id])
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&picked).len(), 2);

    let rest = u
        .roles()
        .where_pivot_not_in("pf_role_id", [admin.id, viewer.id])
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&rest), vec![editor.id]);
}

#[tokio::test]
async fn where_pivot_null_and_not_null_split_on_a_nullable_column() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _admin, _editor, viewer) = seed_roles().await;

    let unnoted = u
        .roles()
        .where_pivot_null("note")
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&unnoted), vec![viewer.id]);

    let noted = u
        .roles()
        .where_pivot_not_null("note")
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(noted.len(), 2);
}

#[tokio::test]
async fn where_pivot_between_is_inclusive_on_both_bounds() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, editor, _viewer) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_between("pf_role_id", admin.id..=editor.id)
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&picked);
    assert!(got.contains(&admin.id), "low bound is inclusive: {got:?}");
    assert!(got.contains(&editor.id), "high bound is inclusive: {got:?}");
    assert_eq!(got.len(), 2);
}

#[tokio::test]
async fn where_pivot_not_between_excludes_the_closed_range() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, editor, viewer) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_not_between("pf_role_id", admin.id..=editor.id)
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&picked), vec![viewer.id]);
}

#[tokio::test]
async fn where_pivot_op_takes_an_arbitrary_comparison() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _admin, editor, viewer) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_op("pf_role_id", ">=", editor.id)
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&picked);
    assert_eq!(got.len(), 2);
    assert!(got.contains(&editor.id));
    assert!(got.contains(&viewer.id));
}

#[tokio::test]
async fn an_operator_outside_the_allowlist_is_rejected() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    u.roles()
        .where_pivot_op("active", "= 1 OR 1", 1i64)
        .get()
        .await
        .expect_err("the operator allowlist must reject this");
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

#[tokio::test]
async fn or_where_pivot_builds_a_disjunction() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _admin, editor, viewer) = seed_roles().await;

    // active = 0 OR note IS NULL  ->  editor + viewer
    let picked = u
        .roles()
        .where_pivot("active", 0i64)
        .or_where_pivot_null("note")
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&picked);
    assert_eq!(got.len(), 2, "expected editor + viewer, got {got:?}");
    assert!(got.contains(&editor.id));
    assert!(got.contains(&viewer.id));
}

#[tokio::test]
async fn the_or_twins_cover_the_whole_family() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, editor, viewer) = seed_roles().await;

    // pf_role_id IN (admin) OR pf_role_id NOT IN (admin, editor)
    //   -> admin + viewer
    let by_in = u
        .roles()
        .where_pivot_in("pf_role_id", [admin.id])
        .or_where_pivot_not_in("pf_role_id", [admin.id, editor.id])
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&by_in);
    assert_eq!(got.len(), 2, "expected admin + viewer, got {got:?}");
    assert!(got.contains(&admin.id));
    assert!(got.contains(&viewer.id));

    // active = 0 OR note IS NOT NULL -> admin + editor
    let by_null = u
        .roles()
        .where_pivot("active", 0i64)
        .or_where_pivot_not_null("note")
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&by_null);
    assert_eq!(got.len(), 2, "expected admin + editor, got {got:?}");
    assert!(got.contains(&admin.id));
    assert!(got.contains(&editor.id));

    // pf_role_id BETWEEN admin..=admin OR pf_role_id >= viewer
    //   -> admin + viewer
    let by_range = u
        .roles()
        .where_pivot_between("pf_role_id", admin.id..=admin.id)
        .or_where_pivot_op("pf_role_id", ">=", viewer.id)
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&by_range);
    assert_eq!(got.len(), 2, "expected admin + viewer, got {got:?}");
    assert!(got.contains(&admin.id));
    assert!(got.contains(&viewer.id));

    // pf_role_id NOT BETWEEN admin..=viewer (matches nothing)
    //   OR active = 0 -> editor
    let by_not_range = u
        .roles()
        .where_pivot_not_between("pf_role_id", admin.id..=viewer.id)
        .or_where_pivot("active", 0i64)
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&by_not_range), vec![editor.id]);
}

// ---- The closure form ---------------------------------------------------

#[tokio::test]
async fn where_pivot_group_ands_the_closure_terms() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _editor, _viewer) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_group(|q| q.filter("active", 1i64).filter("note", "keep"))
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(ids(&picked), vec![admin.id]);
}

#[tokio::test]
async fn a_closure_group_stays_atomic_inside_a_disjunction() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _editor, viewer) = seed_roles().await;

    // note IS NULL OR (active = 1 AND note = 'keep')
    //   -> viewer (null note) + admin (active + noted), NOT editor.
    // Without the group this would parse as
    //   note IS NULL OR active = 1, ANDed with note = 'keep',
    // which returns admin only. The group is what makes viewer appear.
    let picked = u
        .roles()
        .where_pivot_null("note")
        .or_where_pivot_group(|q| q.filter("active", 1i64).filter("note", "keep"))
        .get()
        .await
        .unwrap()
        .into_vec();
    let got = ids(&picked);
    assert_eq!(got.len(), 2, "expected admin + viewer, got {got:?}");
    assert!(got.contains(&admin.id));
    assert!(got.contains(&viewer.id));
}

#[tokio::test]
async fn a_closure_group_narrows_count_the_same_way_it_narrows_get() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    assert_eq!(
        u.roles()
            .where_pivot_null("note")
            .or_where_pivot_group(|q| q.filter("active", 1i64).filter("note", "keep"))
            .count()
            .await
            .unwrap(),
        2,
    );
}

#[tokio::test]
async fn an_empty_closure_group_changes_nothing() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    let picked = u
        .roles()
        .where_pivot_group(|q| q)
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(picked.len(), 3, "a no-op closure must not filter anything");
}

// ---- Failure modes ------------------------------------------------------

#[tokio::test]
async fn a_pivot_filter_makes_detach_fail_closed() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _, _) = seed_roles().await;

    let err = u
        .roles()
        .where_pivot("active", 1i64)
        .detach(admin.id)
        .await
        .expect_err("a filtered detach must refuse rather than delete unfiltered");
    assert!(
        format!("{err}").contains("reads only"),
        "message must explain the refusal, got: {err}"
    );

    assert_eq!(
        u.roles().count().await.unwrap(),
        3,
        "the refusal must not have deleted anything"
    );
}

#[tokio::test]
async fn a_pivot_filter_makes_sync_fail_closed() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _, _) = seed_roles().await;

    let err = u
        .roles()
        .where_pivot("active", 1i64)
        .sync([admin.id])
        .await
        .expect_err("a filtered sync must refuse");
    assert!(format!("{err}").contains("reads only"), "got: {err}");
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

#[tokio::test]
async fn a_pivot_filter_makes_attach_fail_closed() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;
    let extra = PfRole::create(attrs! { name: "auditor" }).await.unwrap();

    let err = u
        .roles()
        .where_pivot("active", 1i64)
        .attach(extra.id)
        .await
        .expect_err("a filtered attach must refuse");
    assert!(format!("{err}").contains("reads only"), "got: {err}");
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

#[tokio::test]
async fn a_pivot_filter_makes_attach_with_fail_closed() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;
    let extra = PfRole::create(attrs! { name: "auditor" }).await.unwrap();

    let err = u
        .roles()
        .where_pivot_group(|q| q.filter("active", 1i64))
        .attach_with(extra.id, attrs! { active: 1i64 })
        .await
        .expect_err("a filtered attach_with must refuse");
    assert!(format!("{err}").contains("reads only"), "got: {err}");
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

#[tokio::test]
async fn an_unfiltered_mutator_still_works() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, admin, _, _) = seed_roles().await;

    u.roles().detach(admin.id).await.unwrap();
    assert_eq!(u.roles().count().await.unwrap(), 2);
}

#[tokio::test]
async fn an_illegal_pivot_column_is_rejected_before_it_reaches_sql() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    let err = u
        .roles()
        .where_pivot("active; DROP TABLE pf_role_user", 1i64)
        .get()
        .await
        .expect_err("the identifier allowlist must reject this");
    let _ = err;

    // The table survived, so nothing was executed.
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

#[tokio::test]
async fn an_illegal_pivot_column_is_rejected_by_count_too() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;
    let (u, _, _, _) = seed_roles().await;

    u.roles()
        .where_pivot_null("note; DROP TABLE pf_role_user")
        .count()
        .await
        .expect_err("the identifier allowlist must reject this");
    assert_eq!(u.roles().count().await.unwrap(), 3);
}

// ---- Polymorphic m2m ----------------------------------------------------

#[tokio::test]
async fn morph_to_many_honours_pivot_filters() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;

    let post = PfPost::create(attrs! { title: "Pivot filters" })
        .await
        .unwrap();
    let rust = PfTag::create(attrs! { label: "rust" }).await.unwrap();
    let draft = PfTag::create(attrs! { label: "draft" }).await.unwrap();

    post.tags()
        .attach_with(rust.id, attrs! { active: 1i64 })
        .await
        .unwrap();
    post.tags()
        .attach_with(draft.id, attrs! { active: 0i64 })
        .await
        .unwrap();

    let live = post
        .tags()
        .where_pivot("active", 1i64)
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, rust.id);

    assert_eq!(
        post.tags()
            .where_pivot("active", 1i64)
            .count()
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn morph_to_many_pivot_filters_refuse_a_write() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;

    let post = PfPost::create(attrs! { title: "Pivot filters" })
        .await
        .unwrap();
    let rust = PfTag::create(attrs! { label: "rust" }).await.unwrap();
    post.tags()
        .attach_with(rust.id, attrs! { active: 1i64 })
        .await
        .unwrap();

    let err = post
        .tags()
        .where_pivot("active", 1i64)
        .detach(rust.id)
        .await
        .expect_err("a filtered detach must refuse");
    assert!(format!("{err}").contains("reads only"), "got: {err}");
    assert_eq!(post.tags().count().await.unwrap(), 1);

    let err = post
        .tags()
        .where_pivot("active", 1i64)
        .sync([rust.id])
        .await
        .expect_err("a filtered sync must refuse");
    assert!(format!("{err}").contains("reads only"), "got: {err}");
    assert_eq!(post.tags().count().await.unwrap(), 1);
}

#[tokio::test]
async fn morph_to_many_honours_the_closure_group() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;

    let post = PfPost::create(attrs! { title: "Grouped" }).await.unwrap();
    let rust = PfTag::create(attrs! { label: "rust" }).await.unwrap();
    let draft = PfTag::create(attrs! { label: "draft" }).await.unwrap();

    post.tags()
        .attach_with(rust.id, attrs! { active: 1i64 })
        .await
        .unwrap();
    post.tags()
        .attach_with(draft.id, attrs! { active: 0i64 })
        .await
        .unwrap();

    let live = post
        .tags()
        .where_pivot_group(|q| q.filter("active", 1i64).filter("pf_tag_id", rust.id))
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, rust.id);
}

#[tokio::test]
async fn morphed_by_many_honours_pivot_filters() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;

    let live_post = PfPost::create(attrs! { title: "live" }).await.unwrap();
    let dead_post = PfPost::create(attrs! { title: "dead" }).await.unwrap();
    let rust = PfTag::create(attrs! { label: "rust" }).await.unwrap();

    live_post
        .tags()
        .attach_with(rust.id, attrs! { active: 1i64 })
        .await
        .unwrap();
    dead_post
        .tags()
        .attach_with(rust.id, attrs! { active: 0i64 })
        .await
        .unwrap();

    let posts = rust
        .posts()
        .where_pivot("active", 1i64)
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(posts.len(), 1, "only the active taggable row");
    assert_eq!(posts[0].id, live_post.id);

    assert_eq!(
        rust.posts()
            .where_pivot("active", 1i64)
            .count()
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn morphed_by_many_honours_the_closure_group() {
    let _db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&_db).await;

    let live_post = PfPost::create(attrs! { title: "live" }).await.unwrap();
    let dead_post = PfPost::create(attrs! { title: "dead" }).await.unwrap();
    let rust = PfTag::create(attrs! { label: "rust" }).await.unwrap();

    live_post
        .tags()
        .attach_with(rust.id, attrs! { active: 1i64 })
        .await
        .unwrap();
    dead_post
        .tags()
        .attach_with(rust.id, attrs! { active: 0i64 })
        .await
        .unwrap();

    let posts = rust
        .posts()
        .where_pivot_group(|q| q.filter("active", 0i64).filter("taggable_id", dead_post.id))
        .get()
        .await
        .unwrap()
        .into_vec();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].id, dead_post.id);
}
