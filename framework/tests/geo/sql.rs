use sea_orm::DbBackend;
use suprnova::Model;

use super::{CustomPlace, ORIGIN, nearby};

fn assert_sql(backend: DbBackend) {
    let query = nearby()
        .filter("kind", "kept")
        .within(10.0)
        .unwrap()
        .order_by_distance()
        .unwrap();
    let (sql, values) = query.try_to_sql_with_bindings_for(backend).unwrap();
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    let latitude = format!("{quote}geo_places{quote}.{quote}latitude{quote}");
    let longitude = format!("{quote}geo_places{quote}.{quote}longitude{quote}");
    let bounds = sql
        .find(&format!("{latitude} BETWEEN"))
        .expect("latitude range");
    let longitude_bounds = sql
        .find(&format!("{longitude} BETWEEN"))
        .expect("longitude range");
    let radius = sql[bounds..].find("ASIN").unwrap() + bounds;
    assert!(
        bounds < longitude_bounds && longitude_bounds < radius,
        "{sql}"
    );
    assert!(sql.contains("AS distance"));
    assert!(sql.ends_with("ORDER BY distance ASC"));
    assert!(sql.contains("POWER(SIN(RADIANS("));
    assert_eq!(values.len(), 12);
    assert_eq!(values[0], ORIGIN.0.into());
    assert_eq!(values[2], ORIGIN.1.into());
    assert_eq!(values[3], "kept".into());
    assert_eq!(values.last(), Some(&10.0.into()));
    if backend == DbBackend::Postgres {
        assert!(sql.contains("LEAST(1.0, GREATEST(0.0,"));
        assert!(!sql.contains('?'));
        for index in 1..=values.len() {
            assert!(
                sql.contains(&format!("${index}")),
                "missing ${index}: {sql}"
            );
        }
        assert!(sql.contains("CAST($1 AS DOUBLE PRECISION)"));
    } else {
        assert_eq!(sql.matches('?').count(), values.len());
        assert!(sql.contains(if backend == DbBackend::Sqlite {
            "MIN(1.0, MAX(0.0,"
        } else {
            "LEAST(1.0, GREATEST(0.0,"
        }));
    }
}

#[test]
fn geo_001_rendered_sql_sqlite() {
    assert_sql(DbBackend::Sqlite);
}
#[test]
fn geo_001_rendered_sql_postgres() {
    assert_sql(DbBackend::Postgres);
}
#[test]
fn geo_001_rendered_sql_mysql() {
    assert_sql(DbBackend::MySql);
}
// MariaDB uses SeaORM's MySql backend; it has no distinct DbBackend variant.
#[test]
fn geo_001_rendered_sql_mariadb() {
    assert_sql(DbBackend::MySql);
}

#[test]
fn geo_001_custom_columns_and_repeated_origins() {
    let (sql, bindings) = CustomPlace::query()
        .nearby(10.0, 20.0)
        .unwrap()
        .within(10.0)
        .unwrap()
        .nearby(30.0, 40.0)
        .unwrap()
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .unwrap();
    assert!(sql.contains("\"geo_custom\".\"north\" BETWEEN"));
    assert!(sql.contains("\"geo_custom\".\"east\" BETWEEN"));
    assert_eq!(sql.matches("AS distance").count(), 1);
    assert_eq!(bindings[0], 30.0.into());
    assert_eq!(bindings[2], 40.0.into());
}
