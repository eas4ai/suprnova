//! BIND-001: one public trait, `RouteBinding`, that `#[model]` implements
//! for every model on the struct the application declares, that
//! `RouteParam<T>` implements by delegating to `T`, and that
//! `AutoRouteBinding` names a second time.

use chrono::{DateTime, Utc};
use suprnova::testing::TestDatabase;
use suprnova::{BoundChild, Model, RouteBinding, RouteParam, attrs, model};

use super::run_sql;

#[model(table = "ct_authors", relations = {
    essays: HasMany<CtEssay>,
})]
pub struct CtAuthor {
    pub id: i64,
    pub name: String,
}

#[model(table = "ct_essays", soft_deletes, route_key = "slug")]
pub struct CtEssay {
    pub id: i64,
    pub ct_author_id: i64,
    pub slug: String,
    pub title: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE ct_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE ct_essays (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                ct_author_id INTEGER NOT NULL, slug TEXT NOT NULL, title TEXT NOT NULL, \
                deleted_at TEXT)",
            "INSERT INTO ct_authors (id, name) VALUES (1, 'Ada'), (2, 'Grace')",
            "INSERT INTO ct_essays (id, ct_author_id, slug, title, deleted_at) VALUES \
                (1, 1, 'engines', 'Engines', NULL), \
                (2, 1, 'notes', 'Notes', '2026-01-01T00:00:00+00:00'), \
                (3, 2, 'cobol', 'Cobol', NULL)",
        ],
    )
    .await;
    db
}

fn implements_route_binding<T: RouteBinding>() {}

#[test]
fn bind_001_a_model_and_its_route_param_implement_route_binding() {
    implements_route_binding::<CtAuthor>();
    implements_route_binding::<CtEssay>();
    implements_route_binding::<RouteParam<CtEssay>>();
    // The second name reaches the same trait through both paths.
    fn through_root<T: suprnova::AutoRouteBinding>() {}
    fn through_database<T: suprnova::database::AutoRouteBinding>() {}
    through_root::<CtEssay>();
    through_database::<RouteParam<CtEssay>>();
}

#[tokio::test]
async fn bind_001_from_route_param_resolves_by_the_route_key() {
    let _db = fixture().await;
    // `route_key = "slug"`: the value is a slug, not an id.
    let essay = <CtEssay as suprnova::AutoRouteBinding>::from_route_param("engines")
        .await
        .expect("bound by slug");
    assert_eq!(essay.title, "Engines");
    let wrapped =
        <RouteParam<CtEssay> as suprnova::database::AutoRouteBinding>::from_route_param("engines")
            .await
            .expect("bound by slug through RouteParam");
    assert_eq!(wrapped.id, essay.id);
    assert!(
        <CtEssay as suprnova::AutoRouteBinding>::from_route_param("1")
            .await
            .is_err(),
        "an id is not a slug"
    );
}

/// The id of the essay a child lookup found, `Err` when the lookup failed.
fn child_id(
    found: Result<Option<BoundChild>, suprnova::FrameworkError>,
) -> Result<Option<i64>, ()> {
    match found {
        Ok(found) => Ok(found.map(|child| {
            child
                .downcast::<CtEssay>()
                .map_err(|child| child.type_name())
                .expect("a child lookup through `essays` finds a CtEssay")
                .id
        })),
        Err(_) => Err(()),
    }
}

#[tokio::test]
async fn bind_001_route_param_resolves_as_its_type_does() {
    let _db = fixture().await;
    // Every lookup, by the route key and by a binding field, plain and
    // soft-deletable, finds through `RouteParam<T>` what it finds through `T`.
    let lookups = [
        ("engines", None),
        ("notes", None),
        ("missing", None),
        ("cobol", None),
        ("Engines", Some("title")),
        ("Notes", Some("title")),
        ("1", Some("id")),
        ("2", Some("id")),
        ("1x", Some("id")),
        ("engines", Some("slug")),
    ];
    for (value, field) in lookups {
        let plain = CtEssay::resolve_route_binding(value, field)
            .await
            .unwrap()
            .map(|essay| essay.id);
        let wrapped = RouteParam::<CtEssay>::resolve_route_binding(value, field)
            .await
            .unwrap()
            .map(|essay| essay.id);
        assert_eq!(plain, wrapped, "{value} by {field:?}");
        let plain_trashed = CtEssay::resolve_soft_deletable_route_binding(value, field)
            .await
            .unwrap()
            .map(|essay| essay.id);
        let wrapped_trashed =
            RouteParam::<CtEssay>::resolve_soft_deletable_route_binding(value, field)
                .await
                .unwrap()
                .map(|essay| essay.id);
        assert_eq!(plain_trashed, wrapped_trashed, "{value} by {field:?}");
    }
    // The fixture tells the lookups apart, so equal results mean something.
    assert_eq!(
        CtEssay::resolve_route_binding("Engines", Some("title"))
            .await
            .unwrap()
            .map(|essay| essay.id),
        Some(1)
    );
    assert_eq!(
        CtEssay::resolve_soft_deletable_route_binding("2", Some("id"))
            .await
            .unwrap()
            .map(|essay| essay.id),
        Some(2)
    );
    assert_eq!(
        RouteParam::<CtEssay>::route_key_name(),
        CtEssay::route_key_name()
    );

    // The values URLs use.
    let essay = CtEssay::find(1_i64).await.unwrap().expect("essay 1");
    let wrapped = RouteParam(essay.clone());
    assert_eq!(wrapped.route_key(), essay.route_key());
    for field in ["slug", "title", "id", "ct_author_id", "no_such_column"] {
        assert_eq!(
            wrapped.route_field(field),
            essay.route_field(field),
            "{field}"
        );
    }
    assert_eq!(wrapped.route_field("title").as_deref(), Some("Engines"));

    // The child lookups, through the author's `essays` relation.
    let ada = CtAuthor::find(1_i64).await.unwrap().expect("Ada");
    let wrapped_ada = RouteParam(ada.clone());
    let children = [
        ("essay", "engines", None),
        ("essay", "notes", None),
        ("essay", "cobol", None),
        ("essay", "Notes", Some("title")),
        ("essay", "1", Some("id")),
        ("essay", "3", Some("id")),
        ("essay", "1x", Some("id")),
        ("comment", "engines", None),
    ];
    for (child, value, field) in children {
        let plain = child_id(ada.resolve_child_route_binding(child, value, field).await);
        let through = child_id(
            wrapped_ada
                .resolve_child_route_binding(child, value, field)
                .await,
        );
        assert_eq!(plain, through, "{child} {value} by {field:?}");
        let plain_trashed = child_id(
            ada.resolve_soft_deletable_child_route_binding(child, value, field)
                .await,
        );
        let through_trashed = child_id(
            wrapped_ada
                .resolve_soft_deletable_child_route_binding(child, value, field)
                .await,
        );
        assert_eq!(
            plain_trashed, through_trashed,
            "{child} {value} by {field:?}, soft-deletable"
        );
    }
    assert_eq!(
        child_id(
            wrapped_ada
                .resolve_soft_deletable_child_route_binding("essay", "Notes", Some("title"))
                .await
        ),
        Ok(Some(2)),
        "the trashed child is found by a binding field through the wrapper"
    );
    assert_eq!(
        child_id(
            wrapped_ada
                .resolve_child_route_binding("essay", "Notes", Some("title"))
                .await
        ),
        Ok(None),
        "the plain child lookup leaves it out through the wrapper"
    );

    let info = RouteParam::<CtEssay>::route_binding_info();
    let inner = CtEssay::route_binding_info();
    assert_eq!(info.name(), inner.name());
    assert_eq!(info.type_id(), inner.type_id());
}

#[tokio::test]
async fn bind_001_the_trait_has_both_soft_deletable_lookups() {
    let _db = fixture().await;
    // The plain lookup leaves the trashed essay out; the soft-deletable
    // one finds it.
    assert!(
        CtEssay::resolve_route_binding("notes", None)
            .await
            .unwrap()
            .is_none()
    );
    let found = CtEssay::resolve_soft_deletable_route_binding("notes", None)
        .await
        .unwrap()
        .expect("the soft-deletable lookup finds a trashed row");
    assert!(found.deleted_at.is_some());

    // And the child lookups, through the author's `essays` relation.
    let ada = CtAuthor::find(1_i64).await.unwrap().expect("Ada");
    assert!(
        ada.resolve_child_route_binding("essay", "notes", None)
            .await
            .unwrap()
            .is_none(),
        "a trashed child is left out"
    );
    let child = ada
        .resolve_soft_deletable_child_route_binding("essay", "notes", None)
        .await
        .unwrap()
        .expect("the soft-deletable child lookup finds it");
    let child: CtEssay = child.downcast().map_err(|_| "a CtEssay").unwrap();
    assert_eq!(child.slug, "notes");
    // Grace's essay is not Ada's.
    assert!(
        ada.resolve_child_route_binding("essay", "cobol", None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn bind_001_route_key_and_route_field_give_the_values_urls_use() {
    let _db = fixture().await;
    let essay = CtEssay::create(attrs! { ct_author_id: 1_i64, slug: "loops", title: "Loops" })
        .await
        .unwrap();
    assert_eq!(essay.route_key(), "loops");
    assert_eq!(essay.route_field("title").as_deref(), Some("Loops"));
    assert_eq!(essay.route_field("id"), Some(essay.id.to_string()));
    assert_eq!(essay.route_field("no_such_column"), None);
    let author = CtAuthor::find(2_i64).await.unwrap().unwrap();
    assert_eq!(author.route_key(), "2");
}

#[test]
fn bind_001_a_bound_child_comes_back_as_its_type() {
    let child = BoundChild::new(41_i64);
    assert_eq!(child.downcast::<i64>().ok(), Some(41));
}
