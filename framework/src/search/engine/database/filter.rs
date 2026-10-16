//! Bound predicates preserve collection null semantics on every SQL backend.

use super::{Dialect, Sql};
use crate::{
    FrameworkError,
    search::{
        AttributeType, AttributeValue, Filter, GeoPoint, IndexDescriptor, Sort, SortDirection,
    },
};
use chrono::SecondsFormat;
use sea_orm::Value;

pub(super) fn value(value: &AttributeValue) -> Result<Value, FrameworkError> {
    match value {
        AttributeValue::Text(value) => Ok(value.clone().into()),
        AttributeValue::Integer(value) => Ok((*value).into()),
        AttributeValue::Float(value) if value.is_finite() => Ok((*value).into()),
        AttributeValue::Bool(value) => Ok((*value).into()),
        AttributeValue::DateTime(value) => {
            Ok(value.to_rfc3339_opts(SecondsFormat::Nanos, true).into())
        }
        AttributeValue::List(_) => serde_json::to_string(&value.to_json())
            .map(Into::into)
            .map_err(|error| FrameworkError::param(format!("search list value: {error}"))),
        AttributeValue::Null => Ok(Value::String(None)),
        AttributeValue::Geo(_) => Err(FrameworkError::param(
            "search geo value requires separate latitude and longitude columns",
        )),
        AttributeValue::Float(_) => Err(FrameworkError::param("search float value must be finite")),
    }
}

fn identifier(index: &IndexDescriptor, name: &str) -> Result<(), FrameworkError> {
    super::identifier(name)
        .map_err(|e| FrameworkError::param(format!("search index {}: {e}", index.name)))
}

fn column(sql: &Sql, attribute: &str) -> String {
    format!("s.{}", sql.ident(attribute))
}

fn text_expression(sql: &Sql, expression: String) -> String {
    match sql.dialect {
        Dialect::Mysql | Dialect::MariaDb => format!("CAST({expression} AS BINARY)"),
        Dialect::Postgres => format!("({expression} COLLATE \"C\")"),
        Dialect::Sqlite => expression,
    }
}

fn null_column(
    index: &IndexDescriptor,
    attribute: &str,
    sql: &Sql,
) -> Result<String, FrameworkError> {
    let attribute = index.attribute(attribute)?;
    Ok(column(
        sql,
        &if attribute.ty == AttributeType::Geo {
            format!("{}_lat", attribute.name)
        } else {
            attribute.name.clone()
        },
    ))
}

pub(super) fn predicate(
    index: &IndexDescriptor,
    table: &str,
    filter: &Filter,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    identifier(index, table)?;
    for attribute in &index.attributes {
        identifier(index, &attribute.name)?;
    }
    filter.validate(index)?;
    render(index, table, filter, sql)
}

fn render(
    index: &IndexDescriptor,
    table: &str,
    filter: &Filter,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    match filter {
        Filter::And(filters) | Filter::Or(filters) => {
            let and = matches!(filter, Filter::And(_));
            if filters.is_empty() {
                return Ok(if and { "TRUE" } else { "FALSE" }.into());
            }
            let children = filters
                .iter()
                .map(|filter| render(index, table, filter, sql))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!(
                "({})",
                children.join(if and { " AND " } else { " OR " })
            ))
        }
        Filter::Not(filter) => Ok(format!("(NOT {})", render(index, table, filter, sql)?)),
        Filter::IsNull(attribute) | Filter::IsNotNull(attribute) => Ok(format!(
            "({} IS {}NULL)",
            null_column(index, attribute, sql)?,
            if matches!(filter, Filter::IsNotNull(_)) {
                "NOT "
            } else {
                ""
            }
        )),
        Filter::Eq(attribute, value) | Filter::Ne(attribute, value) => {
            let result = equal(index, table, attribute, value, sql)?;
            Ok(if matches!(filter, Filter::Ne(..)) {
                format!("(NOT {result})")
            } else {
                result
            })
        }
        Filter::In(attribute, values) | Filter::NotIn(attribute, values) => {
            let values = values
                .iter()
                .map(|value| equal(index, table, attribute, value, sql))
                .collect::<Result<Vec<_>, _>>()?;
            let expression = if values.is_empty() {
                "FALSE".into()
            } else {
                format!("({})", values.join(" OR "))
            };
            Ok(if matches!(filter, Filter::NotIn(..)) {
                format!("(NOT {expression})")
            } else {
                expression
            })
        }
        Filter::Gt(attribute, value)
        | Filter::Gte(attribute, value)
        | Filter::Lt(attribute, value)
        | Filter::Lte(attribute, value) => {
            let operator = match filter {
                Filter::Gt(..) => ">",
                Filter::Gte(..) => ">=",
                Filter::Lt(..) => "<",
                _ => "<=",
            };
            compare(index, attribute, operator, value, sql)
        }
        Filter::Between(attribute, low, high) => Ok(format!(
            "({} AND {})",
            compare(index, attribute, ">=", low, sql)?,
            compare(index, attribute, "<=", high, sql)?
        )),
        Filter::GeoRadius(attribute, point, radius) => {
            let distance = chord(attribute, point, sql);
            let threshold = ((*radius / 6_371_008.8).min(std::f64::consts::PI) / 2.0)
                .sin()
                .powi(2);
            let bound = sql.bind(threshold);
            Ok(format!("COALESCE(({distance} <= {bound}), FALSE)"))
        }
    }
}

fn equal(
    index: &IndexDescriptor,
    table: &str,
    attribute: &str,
    expected: &AttributeValue,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    let declaration = index.attribute(attribute)?;
    if matches!(expected, AttributeValue::Null) {
        return Ok(format!("({} IS NULL)", null_column(index, attribute, sql)?));
    }
    if let AttributeValue::Geo(point) = expected {
        point.validate()?;
        let latitude = sql.bind(point.lat);
        let longitude = sql.bind(point.lng);
        return Ok(format!(
            "COALESCE(({} = {latitude} AND {} = {longitude}), FALSE)",
            column(sql, &format!("{attribute}_lat")),
            column(sql, &format!("{attribute}_lng"))
        ));
    }
    let expected_sql = sql.bind(value(expected)?);
    if declaration.ty == AttributeType::TextList && !matches!(expected, AttributeValue::List(_)) {
        let member = text_expression(sql, format!("members.{}", sql.ident("value")));
        return Ok(format!(
            "EXISTS (SELECT 1 FROM {} AS members WHERE members.{} = s.{} AND {member} = {expected_sql})",
            sql.ident(&format!("{table}_{attribute}")),
            sql.ident("document_key"),
            sql.ident("key")
        ));
    }
    let expression = if matches!(
        declaration.ty,
        AttributeType::Text | AttributeType::TextList
    ) {
        text_expression(sql, column(sql, attribute))
    } else {
        column(sql, attribute)
    };
    Ok(format!(
        "COALESCE(({} = {expected_sql}), FALSE)",
        expression
    ))
}

fn compare(
    index: &IndexDescriptor,
    attribute: &str,
    operator: &str,
    expected: &AttributeValue,
    sql: &mut Sql,
) -> Result<String, FrameworkError> {
    if matches!(
        index.attribute(attribute)?.ty,
        AttributeType::TextList | AttributeType::Geo
    ) || matches!(expected, AttributeValue::Null)
    {
        return Ok("FALSE".into());
    }
    let expected = sql.bind(value(expected)?);
    let expression = if index.attribute(attribute)?.ty == AttributeType::Text {
        text_expression(sql, column(sql, attribute))
    } else {
        column(sql, attribute)
    };
    Ok(format!(
        "COALESCE(({} {operator} {expected}), FALSE)",
        expression
    ))
}

pub(super) fn order(
    index: &IndexDescriptor,
    sorts: &[Sort],
    sql: &mut Sql,
) -> Result<Vec<String>, FrameworkError> {
    let mut result = Vec::with_capacity(sorts.len() * 2);
    for sort in sorts {
        let (attribute, direction, point) = match sort {
            Sort::Attribute {
                attribute,
                direction,
            } => (attribute, direction, None),
            Sort::Distance {
                attribute,
                direction,
                point,
            } => (attribute, direction, Some(point)),
        };
        identifier(index, attribute)?;
        let declaration = index.attribute(attribute)?;
        if !declaration.sortable
            || (point.is_some() && declaration.ty != AttributeType::Geo)
            || (point.is_none()
                && matches!(declaration.ty, AttributeType::Geo | AttributeType::TextList))
        {
            return Err(FrameworkError::param(format!(
                "search index '{}': sort attribute '{attribute}' requires a sortable scalar or an explicit geo distance",
                index.name
            )));
        }
        let expression = if let Some(point) = point {
            point.validate()?;
            chord(attribute, point, sql)
        } else {
            let expression = column(sql, attribute);
            if declaration.ty == AttributeType::Text {
                text_expression(sql, expression)
            } else {
                expression
            }
        };
        result.push(format!(
            "({} IS NULL) ASC",
            null_column(index, attribute, sql)?
        ));
        result.push(format!(
            "{expression} {}",
            if *direction == SortDirection::Asc {
                "ASC"
            } else {
                "DESC"
            }
        ));
    }
    Ok(result)
}

// The haversine chord is monotonic in surface distance and avoids asin/sqrt.
// A correlated scalar subquery binds each origin once even with '?' placeholders.
fn chord(attribute: &str, point: &GeoPoint, sql: &mut Sql) -> String {
    let latitude = sql.bind(point.lat.to_radians());
    let longitude = sql.bind(point.lng.to_radians());
    let cosine = sql.bind(point.lat.to_radians().cos());
    let cast = if matches!(sql.dialect, Dialect::Postgres) {
        "DOUBLE PRECISION"
    } else {
        "DOUBLE"
    };
    let origin = format!(
        "SELECT CAST({latitude} AS {cast}) AS lat, CAST({longitude} AS {cast}) AS lng, CAST({cosine} AS {cast}) AS cosine"
    );
    if matches!(sql.dialect, Dialect::Mysql | Dialect::MariaDb) {
        // MariaDB forbids outer references inside a derived table. Keep side
        // columns in the scalar SELECT and the bound origin in its FROM only.
        let latitude = format!(
            "({} * 0.017453292519943295)",
            column(sql, &format!("{attribute}_lat"))
        );
        let longitude = format!(
            "({} * 0.017453292519943295)",
            column(sql, &format!("{attribute}_lng"))
        );
        return format!(
            "(SELECT LEAST(1.0, GREATEST(0.0, POWER(SIN(({latitude} - origin.lat) / 2.0), 2) + COS({latitude}) * origin.cosine * POWER(SIN(({longitude} - origin.lng) / 2.0), 2))) FROM ({origin}) AS origin)"
        );
    }
    let angles = format!(
        "SELECT (({} * 0.017453292519943295) - origin.lat) / 2.0 AS dlat, (({} * 0.017453292519943295) - origin.lng) / 2.0 AS dlng, {} * 0.017453292519943295 AS latitude, origin.cosine FROM ({origin}) AS origin",
        column(sql, &format!("{attribute}_lat")),
        column(sql, &format!("{attribute}_lng")),
        column(sql, &format!("{attribute}_lat"))
    );
    let (sin_lat, sin_lng, cos_lat) = if matches!(sql.dialect, Dialect::Sqlite) {
        (
            polynomial("angles.dlat", false),
            polynomial("angles.dlng", false),
            polynomial("angles.latitude", true),
        )
    } else {
        (
            "SIN(angles.dlat)".into(),
            "SIN(angles.dlng)".into(),
            "COS(angles.latitude)".into(),
        )
    };
    let terms = format!(
        "SELECT {sin_lat} AS sin_lat, {sin_lng} AS sin_lng, {cos_lat} AS cos_lat, angles.cosine FROM ({angles}) AS angles"
    );
    format!(
        "(SELECT CASE WHEN hav.a < 0.0 THEN 0.0 WHEN hav.a > 1.0 THEN 1.0 ELSE hav.a END FROM (SELECT terms.sin_lat * terms.sin_lat + terms.cos_lat * terms.cosine * terms.sin_lng * terms.sin_lng AS a FROM ({terms}) AS terms) AS hav)"
    )
}

// Fourteen Taylor terms keep sin accurate through [-pi, pi] without SQLite's
// optional math extension. Horner form limits SQL size and parser depth.
fn polynomial(argument: &str, cosine: bool) -> String {
    let mut coefficients = Vec::with_capacity(14);
    let mut factorial = 1.0;
    let mut previous_degree = 0;
    for term in 0..14 {
        let degree = 2 * term + usize::from(!cosine);
        for factor in previous_degree + 1..=degree {
            factorial *= factor as f64;
        }
        previous_degree = degree;
        coefficients.push(if term % 2 == 0 {
            1.0 / factorial
        } else {
            -1.0 / factorial
        });
    }
    let mut expression = format!("{:.17e}", coefficients[13]);
    for coefficient in coefficients[..13].iter().rev() {
        expression = format!("({coefficient:.17e} + ({argument} * {argument}) * {expression})");
    }
    if cosine {
        expression
    } else {
        format!("({argument} * {expression})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AttributeDecl, IndexSettings, SearchDocument};
    use sea_orm::{ConnectionTrait, Database, DbBackend, DbErr, Statement};

    fn descriptor() -> IndexDescriptor {
        IndexDescriptor {
            name: "catalog",
            model_table: "catalog",
            key_column: "id",
            attributes: [
                ("number", AttributeType::Integer),
                ("tags", AttributeType::TextList),
                ("place", AttributeType::Geo),
            ]
            .into_iter()
            .map(|(name, ty)| AttributeDecl {
                name: name.into(),
                ty,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: false,
            })
            .collect(),
            embed_source: None,
            soft_delete: false,
            search_touches: Vec::new(),
            settings: IndexSettings::default,
            scope: None,
        }
    }

    fn sql() -> Sql {
        Sql::new(Dialect::Sqlite)
    }

    #[tokio::test]
    async fn sqlite_bound_filters_match_collection_null_and_list_semantics()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        database.execute_unprepared("CREATE TABLE side (key TEXT PRIMARY KEY, number BIGINT, tags TEXT, place_lat DOUBLE, place_lng DOUBLE)").await?;
        database
            .execute_unprepared(
                "CREATE TABLE side_tags (document_key TEXT, value TEXT, position BIGINT)",
            )
            .await?;
        database
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "INSERT INTO side (key, number, tags) VALUES (?, ?, ?), (?, ?, ?)",
                vec![
                    "present".into(),
                    7_i64.into(),
                    "[\"red\",\"blue\"]".into(),
                    "missing".into(),
                    Value::BigInt(None),
                    Value::String(None),
                ],
            ))
            .await?;
        for (position, tag) in ["red", "blue"].into_iter().enumerate() {
            database
                .execute_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "INSERT INTO side_tags (document_key, value, position) VALUES (?, ?, ?)",
                    vec!["present".into(), tag.into(), (position as i64).into()],
                ))
                .await?;
        }
        let documents = [
            SearchDocument {
                index: "catalog".into(),
                key: "present".into(),
                source_version: 1,
                attributes: [
                    ("number".into(), AttributeValue::Integer(7)),
                    (
                        "tags".into(),
                        AttributeValue::List(vec![
                            AttributeValue::Text("red".into()),
                            AttributeValue::Text("blue".into()),
                        ]),
                    ),
                ]
                .into(),
                embed_text: None,
                title: None,
            },
            SearchDocument {
                index: "catalog".into(),
                key: "missing".into(),
                source_version: 1,
                attributes: Default::default(),
                embed_text: None,
                title: None,
            },
        ];
        let filters = [
            Filter::Ne("number".into(), AttributeValue::Integer(7)),
            Filter::Not(Box::new(Filter::Gt(
                "number".into(),
                AttributeValue::Integer(9),
            ))),
            Filter::Eq("number".into(), AttributeValue::Null),
            Filter::NotIn("number".into(), vec![AttributeValue::Null]),
            Filter::Between(
                "number".into(),
                AttributeValue::Integer(6),
                AttributeValue::Integer(8),
            ),
            Filter::Eq("tags".into(), AttributeValue::Text("red".into())),
            Filter::Ne("tags".into(), AttributeValue::Text("red".into())),
            Filter::Eq("tags".into(), AttributeValue::Null),
            Filter::Eq(
                "tags".into(),
                AttributeValue::List(vec![
                    AttributeValue::Text("red".into()),
                    AttributeValue::Text("blue".into()),
                ]),
            ),
            Filter::Gt("tags".into(), AttributeValue::Text("a".into())),
            Filter::And(Vec::new()),
            Filter::Or(Vec::new()),
        ];
        for filter in filters {
            let mut bindings = sql();
            let expression = predicate(&descriptor(), "side", &filter, &mut bindings)?;
            let rows = database
                .query_all_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    format!("SELECT key FROM side s WHERE {expression} ORDER BY key"),
                    bindings.values,
                ))
                .await?;
            let actual = rows
                .iter()
                .map(|row| row.try_get::<String>("", "key"))
                .collect::<Result<Vec<_>, _>>()?;
            let mut expected = documents
                .iter()
                .filter(|document| filter.matches(document))
                .map(|document| document.key.clone())
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(actual, expected, "{filter:?}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn sqlite_geo_chord_handles_antimeridian_poles_and_null()
    -> Result<(), Box<dyn std::error::Error>> {
        let database = Database::connect("sqlite::memory:").await?;
        database
            .execute_unprepared("CREATE TABLE side (place_lat DOUBLE, place_lng DOUBLE)")
            .await?;
        for (point, origin) in [
            (
                GeoPoint { lat: 0.0, lng: 0.0 },
                GeoPoint { lat: 0.0, lng: 0.0 },
            ),
            (
                GeoPoint {
                    lat: 90.0,
                    lng: 180.0,
                },
                GeoPoint {
                    lat: 90.0,
                    lng: -180.0,
                },
            ),
            (
                GeoPoint {
                    lat: 0.0,
                    lng: 179.9,
                },
                GeoPoint {
                    lat: 0.0,
                    lng: -179.9,
                },
            ),
            (
                GeoPoint {
                    lat: -90.0,
                    lng: 0.0,
                },
                GeoPoint {
                    lat: 90.0,
                    lng: 0.0,
                },
            ),
            (
                GeoPoint {
                    lat: 45.0,
                    lng: 45.0,
                },
                GeoPoint {
                    lat: -45.0,
                    lng: -135.0,
                },
            ),
            (
                GeoPoint {
                    lat: 37.7,
                    lng: -122.4,
                },
                GeoPoint {
                    lat: 40.7,
                    lng: -74.0,
                },
            ),
        ] {
            database.execute_unprepared("DELETE FROM side").await?;
            database
                .execute_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "INSERT INTO side VALUES (?, ?)",
                    [point.lat.into(), point.lng.into()],
                ))
                .await?;
            let mut bindings = sql();
            let expression = chord("place", &origin, &mut bindings);
            let row = database
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    format!("SELECT {expression} AS chord FROM side s"),
                    bindings.values,
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("search geo test has no coordinate row".into()))?;
            let actual: f64 = row.try_get("", "chord")?;
            let expected = (point.distance_to(&origin) / 6_371_008.8 / 2.0)
                .sin()
                .powi(2);
            assert!(
                (actual - expected).abs() < 1e-14,
                "{point:?} to {origin:?}: {actual} != {expected}"
            );
            let distance = point.distance_to(&origin);
            // Distinct longitude representations of one pole differ only by libm
            // rounding. Keep exact zero for identical coordinates and use a
            // micrometer floor where that rounding has no physical meaning.
            let lower = if point == origin {
                0.0
            } else {
                (distance - 1.0).max(1e-6)
            };
            for radius in [lower, distance + 1.0] {
                let mut bindings = sql();
                let filter = Filter::GeoRadius("place".into(), origin, radius);
                let expression = predicate(&descriptor(), "side", &filter, &mut bindings)?;
                let row = database
                    .query_one_raw(Statement::from_sql_and_values(
                        DbBackend::Sqlite,
                        format!("SELECT {expression} AS inside FROM side s"),
                        bindings.values,
                    ))
                    .await?
                    .ok_or_else(|| {
                        DbErr::Custom("search radius test has no coordinate row".into())
                    })?;
                assert_eq!(
                    row.try_get::<bool>("", "inside")?,
                    distance <= radius,
                    "{point:?} to {origin:?}, radius {radius}"
                );
            }
        }
        database.execute_unprepared("DELETE FROM side").await?;
        database
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "INSERT INTO side VALUES (?, ?)",
                [Value::Double(None), Value::Double(None)],
            ))
            .await?;
        let mut bindings = sql();
        let expression = chord("place", &GeoPoint { lat: 0.0, lng: 0.0 }, &mut bindings);
        let row = database
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                format!("SELECT {expression} AS chord FROM side s"),
                bindings.values,
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("search geo test has no null coordinate row".into()))?;
        assert_eq!(row.try_get::<Option<f64>>("", "chord")?, None);
        Ok(())
    }

    #[test]
    fn sort_validation_requires_declared_scalar_or_geo_distance() {
        let index = descriptor();
        let mut bindings = sql();
        assert!(
            order(
                &index,
                &[Sort::Attribute {
                    attribute: "tags".into(),
                    direction: SortDirection::Asc
                }],
                &mut bindings
            )
            .is_err()
        );
        assert!(
            order(
                &index,
                &[Sort::Distance {
                    attribute: "number".into(),
                    point: GeoPoint { lat: 0.0, lng: 0.0 },
                    direction: SortDirection::Asc
                }],
                &mut bindings
            )
            .is_err()
        );
        let ordered = order(
            &index,
            &[Sort::Attribute {
                attribute: "number".into(),
                direction: SortDirection::Desc,
            }],
            &mut bindings,
        )
        .expect("declared integer sort");
        assert!(ordered[0].contains("IS NULL) ASC"));
        assert!(ordered[1].ends_with(" DESC"));
    }
}
