# Nearest-location queries

Status: Agreed 2026-10-23
Prefix: GEO

Drafted 2026-10-23 from the developer's direction on
`reference/laraversine-1.1.0`: "I was thinking about adding these
features ... add these as commitments", and on 22:53, "no geo database"
is needed. Laraversine computes the haversine distance in SQL on two
float columns; this spec brings that to Eloquent queries.

## Observed at d448ed10c

Status: Observed

- The search index filters by `_geoRadius` and sorts by distance from a
  `GeoPoint` on the search engines (`manual/search.md`, "Sorting and geo
  distance"); the Eloquent query builder has no distance expression.
- The database layer renders dialect-specific SQL for SQLite, Postgres,
  MySQL and MariaDB (`framework/src/database/`).

## Requirements

[GEO-001] A model MUST be able to declare its latitude and longitude
columns, and its query builder MUST then offer `nearby(lat, lng)`, which
selects a `distance` column computed by the haversine formula in SQL and
makes it readable on each result, `within(radius)`, which keeps rows
within the radius, and `order_by_distance()`, nearest first; the unit
MUST be kilometres unless `in_miles()` is called; the three MUST
compose with the builder's other clauses, eager loads and pagination;
and the SQL MUST be rendered per dialect for SQLite, Postgres, MySQL and
MariaDB with no spatial extension. A `within` query MUST prefilter with
a bounding box on the latitude and longitude columns, so an index on
those columns bounds the rows the formula visits.
Falsifier: the same fixture on two engines orders two rows differently or reports distances that differ by more than one metre; `within(10)` returns a row 10.5 km away or omits one 9.5 km away; the rendered SQL of a `within` query has no latitude and longitude range predicates; or `nearby(...).paginate(10)` loses the distance column.
Mechanism: `geo-scopes`.
Rationale: Laraversine's `nearby`, `within` and distance attribute computed in SQL on MySQL, MariaDB, PostgreSQL and SQLite with no PostGIS; the bounding box is the standard way to let an ordinary index serve the query.
Status: Agreed 2026-10-23

[GEO-002] Invalid input MUST be refused before any SQL runs: a latitude
outside -90 to 90, a longitude outside -180 to 180, a negative or
non-finite radius, or `within` and `order_by_distance` without a
preceding `nearby`.
Falsifier: `nearby(91.0, 0.0)` runs a query; `within(-1.0)` returns rows; or `order_by_distance()` on a builder without `nearby` succeeds silently.
Mechanism: `geo-scopes`.
Rationale: house rule 2, public surface returns `Result`; a coordinate outside its range is a caller bug the database would not report.
Status: Agreed 2026-10-23
