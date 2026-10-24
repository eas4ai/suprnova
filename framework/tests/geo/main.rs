//! Geo contract proofs share one fixture across SQLite and disposable server engines.

mod engines;
mod sql;

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Direction, Model, model};

const ORIGIN: (f64, f64) = (37.0, -122.0);
const EARTH_RADIUS_KM: f64 = 6371.0;
const KM_PER_MILE: f64 = 1.609344;

#[model(table = "geo_places", timestamps = false,
    geo(lat = "latitude", lng = "longitude"),
    relations = { notes: HasMany<GeoNote> { fk = "place_id" } },
)]
pub struct Place {
    pub id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub kind: String,
}

#[model(table = "geo_notes", timestamps = false)]
pub struct GeoNote {
    pub id: i64,
    pub place_id: i64,
    pub body: String,
}

#[model(
    table = "geo_custom",
    timestamps = false,
    geo(lng = "east", lat = "north")
)]
pub struct CustomPlace {
    pub id: i64,
    pub north: f64,
    pub east: f64,
}

struct EmptyMigrator;
#[async_trait::async_trait]
impl MigratorTrait for EmptyMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![]
    }
}

fn nearby() -> Builder<Place> {
    Place::query()
        .nearby(ORIGIN.0, ORIGIN.1)
        .expect("valid origin")
}

// Solve the forward great-circle problem so fixtures cover bearings as well as radii.
fn destination(origin: (f64, f64), km: f64, bearing: f64) -> (f64, f64) {
    let lat = origin.0.to_radians();
    let lng = origin.1.to_radians();
    let angle = km / EARTH_RADIUS_KM;
    let bearing = bearing.to_radians();
    let target_lat = (lat.sin() * angle.cos() + lat.cos() * angle.sin() * bearing.cos()).asin();
    let target_lng = lng
        + (bearing.sin() * angle.sin() * lat.cos())
            .atan2(angle.cos() - lat.sin() * target_lat.sin());
    (
        target_lat.to_degrees(),
        (target_lng.to_degrees() + 180.0).rem_euclid(360.0) - 180.0,
    )
}

fn distance(origin: (f64, f64), target: (f64, f64)) -> f64 {
    let dlat = (target.0 - origin.0).to_radians();
    let dlng = (target.1 - origin.1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + origin.0.to_radians().cos() * target.0.to_radians().cos() * (dlng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.clamp(0.0, 1.0).sqrt().asin()
}

fn fixture_points() -> Vec<(f64, f64)> {
    let mut points: Vec<_> = [
        0.0, 0.001, 1.0, 3.0, 6.0, 9.5, 10.5, 12.0, 25.0, 100.0, 800.0,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, km)| destination(ORIGIN, km, index as f64 * 37.0))
    .collect();
    points[0] = ORIGIN;
    points.push((-ORIGIN.0, ORIGIN.1 + 180.0));
    points
}

async fn execute(conn: &DatabaseConnection, sql: &str) {
    conn.execute_raw(Statement::from_string(
        conn.get_database_backend(),
        sql.to_owned(),
    ))
    .await
    .expect("fixture statement");
}

async fn seed(conn: &DatabaseConnection) {
    for sql in [
        "DROP TABLE IF EXISTS geo_notes",
        "DROP TABLE IF EXISTS geo_places",
        "CREATE TABLE geo_places (id BIGINT PRIMARY KEY, name TEXT NOT NULL, latitude DOUBLE PRECISION NOT NULL, longitude DOUBLE PRECISION NOT NULL, kind TEXT NOT NULL)",
        "CREATE TABLE geo_notes (id BIGINT PRIMARY KEY, place_id BIGINT NOT NULL, body TEXT NOT NULL)",
        "CREATE INDEX geo_coordinates ON geo_places (latitude, longitude)",
    ] {
        execute(conn, sql).await;
    }
    let backend = conn.get_database_backend();
    let placeholders = if backend == DbBackend::Postgres {
        "$1, $2, $3, $4, $5"
    } else {
        "?, ?, ?, ?, ?"
    };
    for (index, (lat, lng)) in fixture_points().into_iter().enumerate() {
        let id = index as i64 + 1;
        conn.execute_raw(Statement::from_sql_and_values(backend,
            format!("INSERT INTO geo_places (id, name, latitude, longitude, kind) VALUES ({placeholders})"),
            [sea_orm::Value::from(id), format!("place-{id}").into(), lat.into(), lng.into(), if index.is_multiple_of(2) { "kept" } else { "other" }.into()],
        )).await.expect("insert coordinate fixture");
        execute(
            conn,
            &format!("INSERT INTO geo_notes (id, place_id, body) VALUES ({id}, {id}, 'note')"),
        )
        .await;
    }
}

async fn sqlite() -> TestDatabase {
    suprnova::context::Context::test_clear_query();
    let db = TestDatabase::fresh::<EmptyMigrator>()
        .await
        .expect("fresh SQLite");
    seed(db.conn()).await;
    db
}

fn assert_distances(rows: &[Place], miles: bool) {
    for row in rows {
        let expected = distance(ORIGIN, (row.latitude, row.longitude));
        let actual = row.distance().expect("distance projected");
        let actual_km = if miles { actual * KM_PER_MILE } else { actual };
        assert!(actual.is_finite() && actual >= 0.0);
        assert!(
            (actual_km - expected).abs() <= 0.001,
            "place {}: expected {expected}, got {actual_km}",
            row.id
        );
    }
}

async fn assert_shared_contract() {
    let rows = nearby()
        .order_by_distance()
        .unwrap()
        .order_by("id", Direction::Asc)
        .get()
        .await
        .unwrap();
    assert_eq!(rows.len(), 12);
    assert_distances(&rows, false);
    let mut expected: Vec<_> = fixture_points()
        .into_iter()
        .enumerate()
        .map(|(index, point)| (index as i64 + 1, distance(ORIGIN, point)))
        .collect();
    expected.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        expected.iter().map(|entry| entry.0).collect::<Vec<_>>()
    );
    assert!(rows[0].distance().unwrap() < 1e-10);

    let within = nearby()
        .within(10.0)
        .unwrap()
        .order_by_distance()
        .unwrap()
        .get()
        .await
        .unwrap();
    assert_eq!(
        within.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_distances(&within, false);
    assert!(within.iter().any(|row| row.id == 6), "keep 9.5 km");
    assert!(within.iter().all(|row| row.id != 7), "exclude 10.5 km");

    let miles = nearby()
        .within(10.0)
        .unwrap()
        .in_miles()
        .unwrap()
        .order_by_distance()
        .unwrap()
        .get()
        .await
        .unwrap();
    assert_eq!(miles.len(), 8);
    assert_distances(&miles, true);
    let before = Place::query()
        .in_miles()
        .unwrap()
        .nearby(ORIGIN.0, ORIGIN.1)
        .unwrap()
        .within(10.0)
        .unwrap()
        .count()
        .await
        .unwrap();
    assert_eq!(before, 8);

    let page = nearby()
        .filter("kind", "kept")
        .within(10.0)
        .unwrap()
        .with(["notes"])
        .order_by_distance()
        .unwrap()
        .paginate(2)
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(
        page.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_distances(&page.data, false);
    for row in &page.data {
        assert_eq!(row.notes_loaded().len(), 1);
        assert_eq!(row.notes_loaded()[0].place_id, row.id);
    }
    // This is the falsifier's exact page size, including its COUNT phase.
    let page = nearby()
        .order_by_distance()
        .unwrap()
        .paginate(10)
        .await
        .unwrap();
    assert_eq!(page.total, 12);
    assert_eq!(page.data.len(), 10);
    assert_distances(&page.data, false);

    let plain = Place::query().first().await.unwrap().unwrap();
    assert_eq!(plain.distance(), None);
    let selected = nearby()
        .select(["id", "name", "latitude", "longitude", "kind"])
        .within(10.0)
        .unwrap()
        .order_by_distance()
        .unwrap()
        .get()
        .await
        .unwrap();
    assert_eq!(selected.len(), 6);
    assert_distances(&selected, false);
}

#[tokio::test]
async fn geo_001_fixture_sqlite() {
    let _db = sqlite().await;
    assert_shared_contract().await;
}

#[tokio::test]
async fn geo_001_sqlite_math_is_available_on_connections() {
    let db = sqlite().await;
    let row = db.conn().query_one_raw(Statement::from_string(DbBackend::Sqlite,
        "SELECT sqlite_version() AS version, sqlite_compileoption_used('ENABLE_MATH_FUNCTIONS') AS native, RADIANS(180.0) AS radians, SIN(0.0) AS sin, COS(0.0) AS cos, ASIN(1.0) AS asin, SQRT(4.0) AS sqrt, POWER(2.0, 3.0) AS power".to_owned(),
    )).await.unwrap().unwrap();
    let native = row.try_get::<i32>("", "native").unwrap();
    assert!([0, 1].contains(&native));
    assert!((row.try_get::<f64>("", "radians").unwrap() - std::f64::consts::PI).abs() < 1e-14);
    assert_eq!(row.try_get::<f64>("", "sin").unwrap(), 0.0);
    assert_eq!(row.try_get::<f64>("", "cos").unwrap(), 1.0);
    assert_eq!(row.try_get::<f64>("", "sqrt").unwrap(), 2.0);
    assert_eq!(row.try_get::<f64>("", "power").unwrap(), 8.0);
    assert!((row.try_get::<f64>("", "asin").unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-14);
    eprintln!(
        "SQLite {}: native math = {native}",
        row.try_get::<String>("", "version").unwrap()
    );
}

#[tokio::test]
async fn geo_001_composes_filters_joins_and_cloned_pages() {
    let _db = sqlite().await;
    let query = nearby()
        .within(10.0)
        .unwrap()
        .filter("kind", "kept")
        .or_filter("kind", "other")
        .order_by_distance()
        .unwrap();
    assert_eq!(query.clone().count().await.unwrap(), 6);
    assert_eq!(
        query
            .clone()
            .within(5.0)
            .unwrap()
            .within(20.0)
            .unwrap()
            .count()
            .await
            .unwrap(),
        4
    );
    let joined = query
        .clone()
        .join("geo_notes", "geo_places.id", "=", "geo_notes.place_id")
        .get()
        .await
        .unwrap();
    assert_eq!(joined.len(), 6);
    assert_distances(&joined, false);
    suprnova::context::Context::test_set_query("page", "2");
    let page = query.clone().with(["notes"]).paginate(2).await.unwrap();
    suprnova::context::Context::test_clear_query();
    assert_eq!(page.total, 6);
    assert_eq!(
        page.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_distances(&page.data, false);
    assert!(page.data.iter().all(|row| row.notes_loaded().len() == 1));
    let simple = query.simple_paginate(2).await.unwrap();
    assert_distances(&simple.data, false);
}

#[tokio::test]
async fn geo_001_bounding_box_wraps_and_covers_poles_and_large_radii() {
    let db = sqlite().await;
    for origin in [
        (0.0, 179.99),
        (0.0, -179.99),
        (89.99, 10.0),
        (-89.99, -10.0),
        (90.0, 180.0),
        (-90.0, -180.0),
    ] {
        execute(db.conn(), "DELETE FROM geo_places").await;
        for (index, km) in [9.5, 10.5].into_iter().enumerate() {
            let target = destination(origin, km, 90.0);
            let id = index + 1;
            execute(db.conn(), &format!("INSERT INTO geo_places (id, name, latitude, longitude, kind) VALUES ({id}, 'edge', {}, {}, 'kept')", target.0, target.1)).await;
        }
        let rows = Place::query()
            .nearby(origin.0, origin.1)
            .unwrap()
            .within(10.0)
            .unwrap()
            .get()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "origin {origin:?}");
        assert_eq!(rows[0].id, 1);
        assert!((rows[0].distance().unwrap() - 9.5).abs() < 0.001);
    }
    seed(db.conn()).await;
    for radius in [std::f64::consts::PI * EARTH_RADIUS_KM + 1.0, f64::MAX] {
        assert_eq!(nearby().within(radius).unwrap().count().await.unwrap(), 12);
    }
    let zero = nearby().within(0.0).unwrap().get().await.unwrap();
    assert_eq!(zero.len(), 1);
    assert_eq!(zero[0].distance(), Some(0.0));
}

#[test]
fn geo_002_refuses_invalid_coordinates_without_a_database() {
    for lat in [-90.001, 90.001, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = Place::query().nearby(lat, 0.0).err().expect("bad latitude");
        assert_eq!(error.status_code(), 400);
        assert!(error.to_string().contains("latitude"));
    }
    for lng in [
        -180.001,
        180.001,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        let error = Place::query()
            .nearby(0.0, lng)
            .err()
            .expect("bad longitude");
        assert_eq!(error.status_code(), 400);
        assert!(error.to_string().contains("longitude"));
    }
    for lat in [-90.0, 0.0, 90.0] {
        for lng in [-180.0, 0.0, 180.0] {
            assert!(Place::query().nearby(lat, lng).is_ok());
        }
    }
}

#[test]
fn geo_002_refuses_invalid_radii_without_a_database() {
    for radius in [-0.001, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = nearby().within(radius).err().expect("bad radius");
        assert_eq!(error.status_code(), 400);
        assert!(error.to_string().contains("radius"));
    }
    assert!(nearby().within(0.0).is_ok());
    assert!(nearby().within(-0.0).is_ok());
    assert!(nearby().within(f64::MAX).is_ok());
}

#[test]
fn geo_002_requires_nearby_before_radius_or_order() {
    for error in [
        Place::query().within(1.0).err(),
        Place::query().order_by_distance().err(),
        Place::query().in_miles().unwrap().within(1.0).err(),
    ] {
        let error = error.expect("origin required");
        assert_eq!(error.status_code(), 400);
        assert!(error.to_string().contains("preceding nearby"));
    }
    assert!(GeoNote::query().nearby(0.0, 0.0).is_err());
}

#[tokio::test]
async fn geo_001_radius_stays_on_mass_writes() {
    let _db = sqlite().await;
    assert_eq!(
        nearby()
            .within(10.0)
            .unwrap()
            .update(suprnova::attrs! { kind: "updated" })
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        Place::query()
            .filter("kind", "updated")
            .count()
            .await
            .unwrap(),
        6
    );
    assert_eq!(nearby().within(10.0).unwrap().delete().await.unwrap(), 6);
    assert_eq!(Place::query().count().await.unwrap(), 6);
}

#[tokio::test]
async fn geo_001_sqlite_math_on_each_pool_connection() {
    let config = suprnova::DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(2)
        .min_connections(2)
        .logging(false)
        .build();
    let db = suprnova::DbConnection::connect(&config).await.unwrap();
    let pool = db.inner().get_sqlite_connection_pool();
    let mut first = pool.acquire().await.unwrap();
    let mut second = pool.acquire().await.unwrap();
    for connection in [&mut first, &mut second] {
        let value: f64 = sea_orm::sqlx::query_scalar(
            "SELECT POWER(SIN(RADIANS(90.0)), 2.0) + COS(0.0) + SQRT(4.0) + ASIN(0.0)",
        )
        .fetch_one(&mut **connection)
        .await
        .unwrap();
        assert_eq!(value, 4.0);
        let null: Option<f64> = sea_orm::sqlx::query_scalar("SELECT SIN(NULL) + SQRT(-1.0)")
            .fetch_one(&mut **connection)
            .await
            .unwrap();
        assert_eq!(null, None);
    }
}

#[tokio::test]
async fn geo_001_scopes_and_raw_select_keep_distance() {
    struct Kept;
    impl suprnova::GlobalScope<Place> for Kept {
        fn apply(&self, query: Builder<Place>) -> Builder<Place> {
            query.filter("kind", "kept")
        }
    }
    let _db = sqlite().await;
    suprnova::ScopeRegistry::register::<Place, _>(Kept);
    let rows = nearby()
        .within(10.0)
        .unwrap()
        .order_by_distance()
        .unwrap()
        .with(["notes"])
        .select_raw_with_bindings("geo_places.*, ? AS marker", vec![42_i64.into()])
        .get()
        .await
        .unwrap();
    suprnova::ScopeRegistry::clear();
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert_distances(&rows, false);
    assert!(rows.iter().all(|row| row.notes_loaded().len() == 1));
}

#[tokio::test]
async fn geo_001_sqlite_can_use_the_coordinate_index() {
    let db = sqlite().await;
    let (sql, values) = nearby()
        .within(10.0)
        .unwrap()
        .try_to_sql_with_bindings_for(DbBackend::Sqlite)
        .unwrap();
    let plan = db
        .conn()
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            format!("EXPLAIN QUERY PLAN {sql}"),
            values,
        ))
        .await
        .unwrap();
    assert!(plan.iter().any(|row| {
        row.try_get::<String>("", "detail")
            .unwrap()
            .contains("geo_coordinates")
    }));
}
