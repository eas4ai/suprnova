use sea_orm::DbBackend;
use suprnova::{Builder, Direction, EloquentModel, Model, attrs, model};

use super::{CustomPlace, ORIGIN, destination, execute, sqlite};

#[model(table = "geo_groups", timestamps = false, relations = {
    places: HasMany<ScopedPlace> { fk = "group_id" },
})]
pub struct GeoGroup {
    pub id: i64,
}

#[model(table = "geo_scoped_places", timestamps = false, soft_deletes, morph_type = "geo_scoped_place",
    geo(lat = "latitude", lng = "longitude"), relations = {
        children: HasMany<ScopedPlace> { fk = "parent_id" },
    },
)]
pub struct ScopedPlace {
    pub id: i64,
    pub group_id: i64,
    pub parent_id: Option<i64>,
    pub latitude: f64,
    pub longitude: f64,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[model(table = "geo_pins", timestamps = false, relations = {
    subject: MorphTo { targets = [ScopedPlace, CustomPlace] },
})]
pub struct GeoPin {
    pub id: i64,
    pub subject_id: i64,
    pub subject_type: String,
}

async fn fixture() -> suprnova::testing::TestDatabase {
    let db = sqlite().await;
    for sql in [
        "CREATE TABLE geo_groups (id BIGINT PRIMARY KEY)",
        "CREATE TABLE geo_scoped_places (id BIGINT PRIMARY KEY, group_id BIGINT NOT NULL, parent_id BIGINT, latitude DOUBLE PRECISION NOT NULL, longitude DOUBLE PRECISION NOT NULL, deleted_at TEXT)",
        "CREATE TABLE geo_pins (id BIGINT PRIMARY KEY, subject_id BIGINT NOT NULL, subject_type TEXT NOT NULL)",
        "CREATE TABLE geo_custom (id BIGINT PRIMARY KEY, north DOUBLE PRECISION NOT NULL, east DOUBLE PRECISION NOT NULL)",
        "INSERT INTO geo_groups (id) VALUES (1), (2), (3)",
    ] {
        execute(db.conn(), sql).await;
    }
    // Group 1 has only distant places. Groups 2 and 3 each have two nearby places.
    for (index, km) in [100.0, 25.0, 6.0, 3.0, 9.5, 1.0].into_iter().enumerate() {
        let id = index + 1;
        let group = index / 2 + 1;
        let parent = if id % 2 == 0 { id - 1 } else { id };
        let (lat, lng) = destination(ORIGIN, km, 90.0);
        execute(db.conn(), &format!("INSERT INTO geo_scoped_places (id, group_id, parent_id, latitude, longitude) VALUES ({id}, {group}, {parent}, {lat}, {lng})")).await;
        execute(db.conn(), &format!("INSERT INTO geo_pins (id, subject_id, subject_type) VALUES ({id}, {id}, 'geo_scoped_place')")).await;
    }
    db
}

fn radius(q: Builder<ScopedPlace>) -> Builder<ScopedPlace> {
    q.nearby(ORIGIN.0, ORIGIN.1).unwrap().within(10.0).unwrap()
}

#[tokio::test]
async fn geo_001_where_has_and_negation_keep_radius() {
    let _db = fixture().await;
    let yes = GeoGroup::query().where_has::<ScopedPlace, _>("places", |q| {
        radius(q.filter("id", -1).or_where_op("id", ">", 0))
    });
    assert_eq!(yes.pluck::<i64>("id").await.unwrap(), [2, 3]);
    let no = GeoGroup::query().where_doesnt_have::<ScopedPlace, _>("places", radius);
    assert_eq!(no.pluck::<i64>("id").await.unwrap(), [1]);
    assert_eq!(
        GeoGroup::query()
            .filter("id", -1)
            .or_where_has::<ScopedPlace, _>("places", radius)
            .pluck::<i64>("id")
            .await
            .unwrap(),
        [2, 3]
    );
    assert_eq!(
        GeoGroup::query()
            .filter("id", -1)
            .or_where_doesnt_have::<ScopedPlace, _>("places", radius)
            .pluck::<i64>("id")
            .await
            .unwrap(),
        [1]
    );
}

#[tokio::test]
async fn geo_001_nested_has_keeps_radius_on_related_alias() {
    let _db = fixture().await;
    let rows = GeoGroup::query()
        .where_has::<ScopedPlace, _>("places", |q| {
            q.where_has::<ScopedPlace, _>("children", |q| radius(q).has("children"))
        })
        .pluck::<i64>("id")
        .await
        .unwrap();
    assert_eq!(rows, [2, 3]);
    let rows = ScopedPlace::query()
        .where_has::<ScopedPlace, _>("children", radius)
        .order_by("id", Direction::Asc)
        .pluck::<i64>("id")
        .await
        .unwrap();
    assert_eq!(rows, [3, 5]);
}

#[tokio::test]
async fn geo_001_morph_existence_keeps_radius() {
    let _db = fixture().await;
    for query in [
        GeoPin::query().where_has::<ScopedPlace, _>("subject", radius),
        GeoPin::query().where_has_morph("subject", ["geo_scoped_place"], |q, _| radius(q)),
    ] {
        assert_eq!(
            query
                .order_by("id", Direction::Asc)
                .pluck::<i64>("id")
                .await
                .unwrap(),
            [3, 4, 5, 6]
        );
    }
    for query in [
        GeoPin::query().where_doesnt_have::<ScopedPlace, _>("subject", radius),
        GeoPin::query().where_doesnt_have_morph("subject", ["geo_scoped_place"], |q, _| radius(q)),
    ] {
        assert_eq!(
            query
                .order_by("id", Direction::Asc)
                .pluck::<i64>("id")
                .await
                .unwrap(),
            [1, 2]
        );
    }
}

#[test]
fn geo_002_morph_radius_rejects_other_owner_tables_before_sql() {
    for query in [
        GeoPin::query().where_has_morph("subject", "*", |q, _| radius(q)),
        GeoPin::query().where_doesnt_have_morph("subject", "*", |q, _| radius(q)),
    ] {
        let error = query
            .try_to_sql_with_bindings_for(DbBackend::Sqlite)
            .unwrap_err();
        assert!(error.to_string().contains("geo existence predicate"));
    }
}

#[tokio::test]
async fn geo_001_morph_has_preserves_owner_geo_scope() {
    struct NearbyOwner;
    impl suprnova::GlobalScope<ScopedPlace> for NearbyOwner {
        fn apply(&self, query: Builder<ScopedPlace>) -> Builder<ScopedPlace> {
            radius(query)
        }
    }
    let _db = fixture().await;
    suprnova::ScopeRegistry::register::<ScopedPlace, _>(NearbyOwner);
    let result = GeoPin::query()
        .has("subject")
        .order_by("id", Direction::Asc)
        .pluck::<i64>("id")
        .await;
    suprnova::ScopeRegistry::clear();
    assert_eq!(result.unwrap(), [3, 4, 5, 6]);
}

#[tokio::test]
async fn geo_001_limited_eager_load_orders_each_parent_by_distance() {
    let _db = fixture().await;
    let rows = GeoGroup::query()
        .order_by("id", Direction::Asc)
        .with_where(("places", |q: Builder<ScopedPlace>| {
            radius(q).order_by_distance().unwrap().limit(1)
        }))
        .get()
        .await
        .unwrap();
    assert!(rows[0].places_loaded().is_empty());
    for (group, id, km) in [(&rows[1], 4, 3.0), (&rows[2], 6, 1.0)] {
        let places = group.places_loaded();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].id, id);
        assert!((places[0].distance().unwrap() - km).abs() <= 0.0005);
    }
}

#[tokio::test]
async fn geo_001_distance_order_in_scalar_reads() {
    let _db = fixture().await;
    let query = radius(ScopedPlace::query()).order_by_distance().unwrap();
    assert_eq!(
        query.clone().pluck::<i64>("id").await.unwrap(),
        [6, 4, 3, 5]
    );
    assert_eq!(query.value::<i64>("id").await.unwrap(), Some(6));
}

fn joined() -> Builder<ScopedPlace> {
    radius(ScopedPlace::query())
        .join(
            "geo_groups",
            "geo_groups.id",
            "=",
            "geo_scoped_places.group_id",
        )
        .order_by_distance()
        .unwrap()
        .limit(1)
}

#[tokio::test]
async fn geo_001_distance_order_in_joined_update() {
    let _db = fixture().await;
    assert_eq!(joined().update(attrs! { group_id: 1 }).await.unwrap(), 1);
    assert_eq!(
        ScopedPlace::query()
            .filter("group_id", 1)
            .order_by("id", Direction::Asc)
            .pluck::<i64>("id")
            .await
            .unwrap(),
        [1, 2, 6]
    );
}

#[tokio::test]
async fn geo_001_distance_order_in_joined_soft_delete() {
    let _db = fixture().await;
    assert_eq!(joined().delete().await.unwrap(), 1);
    assert_eq!(
        ScopedPlace::query()
            .only_trashed()
            .pluck::<i64>("id")
            .await
            .unwrap(),
        [6]
    );
}

#[tokio::test]
async fn geo_001_distance_order_in_joined_delete() {
    let _db = fixture().await;
    assert_eq!(joined().force_delete_all().await.unwrap(), 1);
    assert_eq!(
        ScopedPlace::query()
            .with_trashed()
            .order_by("id", Direction::Asc)
            .pluck::<i64>("id")
            .await
            .unwrap(),
        [1, 2, 3, 4, 5]
    );
}

#[test]
fn geo_001_joined_write_selector_has_one_column_and_bound_distance_order() {
    for backend in [DbBackend::Sqlite, DbBackend::Postgres] {
        let (sql, values) = joined().to_delete_sql_with_bindings_for(backend, "ignored");
        let selector = sql.split(" IN (SELECT ").nth(1).unwrap();
        assert!(
            selector.starts_with("\"geo_scoped_places\".\"id\" FROM "),
            "{sql}"
        );
        assert!(!sql.contains("AS distance"), "{sql}");
        assert!(
            sql.split(" ORDER BY ").nth(1).unwrap().contains("ASIN"),
            "{sql}"
        );
        assert_eq!(
            &values[values.len() - 3..],
            &[ORIGIN.0.into(), ORIGIN.0.into(), ORIGIN.1.into()]
        );
    }
}

#[test]
fn geo_001_eager_window_binds_distance_before_projection_and_radius() {
    for backend in [DbBackend::Sqlite, DbBackend::Postgres, DbBackend::MySql] {
        let (sql, values) = radius(ScopedPlace::query())
            .order_by_distance()
            .unwrap()
            .limit(1)
            .__eager_limit("group_id")
            .try_to_sql_with_bindings_for(backend)
            .unwrap();
        let window = sql
            .split("ROW_NUMBER() OVER (")
            .nth(1)
            .unwrap()
            .split(" AS __suprnova_row_number")
            .next()
            .unwrap();
        assert!(window.contains("ASIN"), "{sql}");
        assert_eq!(values.len(), 14);
        assert_eq!(
            &values[..6],
            &[
                ORIGIN.0.into(),
                ORIGIN.0.into(),
                ORIGIN.1.into(),
                ORIGIN.0.into(),
                ORIGIN.0.into(),
                ORIGIN.1.into()
            ]
        );
        if backend == DbBackend::Postgres {
            for index in 1..=values.len() {
                assert_eq!(sql.matches(&format!("${index} AS")).count(), 1, "{sql}");
            }
        } else {
            assert_eq!(sql.matches('?').count(), values.len());
        }
    }
}

#[test]
#[should_panic(expected = "expected 0, got 0.00075")]
fn geo_001_reference_tolerance_rejects_more_than_half_a_metre() {
    let mut row = super::Place::default();
    row.latitude = ORIGIN.0;
    row.longitude = ORIGIN.1;
    row.__set_geo_distance(0.00075);
    super::assert_distances(&[row], false);
}
