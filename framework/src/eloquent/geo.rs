//! Great-circle queries keep distance and bounding predicates on the same origin and unit.

use sea_orm::{DbBackend, QueryResult, Value};

use super::builder::OrderTerm;
use super::{Builder, EloquentModel};
use crate::FrameworkError;
use crate::database::{clauses::quote_identifier, placeholder::placeholder, validate_identifier};

const EARTH_RADIUS_KM: f64 = 6371.0;
const KM_PER_MILE: f64 = 1.609344;
/// Rows this far beyond a radius still count as inside: one micrometre, so a
/// row at exactly the radius, or two coordinates naming the same point at a
/// pole, are not lost to floating-point rounding in the SQL formula.
const RADIUS_TOLERANCE_KM: f64 = 1e-9;

#[derive(Debug, Clone, Default)]
pub(super) struct GeoQuery {
    origin: Option<Origin>,
    radius: Option<f64>,
    miles: bool,
}

#[derive(Debug, Clone)]
struct Origin {
    lat: f64,
    lng: f64,
    columns: (&'static str, &'static str),
    table: &'static str,
}

impl<M: EloquentModel> Builder<M> {
    /// Select a great-circle distance so results can be ranked without a spatial extension.
    ///
    /// The model declares `geo(lat = "latitude", lng = "longitude")`. Coordinates
    /// are degrees and distances are kilometres unless [`Self::in_miles`] is used.
    /// Returns an error before database access for non-finite or out-of-range
    /// coordinates, or a model without declared coordinate columns. A row whose
    /// latitude or longitude is NULL has no distance and is never nearby.
    pub fn nearby(mut self, lat: f64, lng: f64) -> Result<Self, FrameworkError> {
        if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
            return Err(FrameworkError::param(
                "nearby latitude must be finite and within -90..90",
            ));
        }
        if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
            return Err(FrameworkError::param(
                "nearby longitude must be finite and within -180..180",
            ));
        }
        let columns = M::GEO_COLUMNS
            .ok_or_else(|| FrameworkError::param("nearby requires model geo columns"))?;
        for column in [columns.0, columns.1] {
            validate_identifier(column)?;
        }
        self.geo.origin = Some(Origin {
            lat,
            lng,
            columns,
            table: M::TABLE,
        });
        Ok(self)
    }

    /// Keep rows within an inclusive radius, with indexable coordinate ranges first.
    ///
    /// Returns an error before database access unless [`Self::nearby`] precedes
    /// this call and the radius is finite and non-negative. Repeated calls keep
    /// the smallest radius. [`Self::in_miles`] sets the unit for the whole query.
    /// Rows up to one micrometre beyond the radius count as inside, so rounding
    /// in the SQL formula never drops a row at the radius.
    pub fn within(mut self, radius: f64) -> Result<Self, FrameworkError> {
        if !self.geo.has_origin() {
            return Err(FrameworkError::param("within requires a preceding nearby"));
        }
        if !radius.is_finite() || radius < 0.0 {
            return Err(FrameworkError::param(
                "within radius must be finite and non-negative",
            ));
        }
        self.geo.radius = Some(self.geo.radius.map_or(radius, |old| old.min(radius)));
        Ok(self)
    }

    /// Append nearest-first ordering while preserving earlier orders and other clauses.
    ///
    /// Returns an error before database access unless [`Self::nearby`] precedes
    /// this call. Add a primary-key order afterwards to break distance ties.
    pub fn order_by_distance(mut self) -> Result<Self, FrameworkError> {
        if !self.geo.has_origin() {
            return Err(FrameworkError::param(
                "order_by_distance requires a preceding nearby",
            ));
        }
        self.orders.push(OrderTerm::Distance);
        Ok(self)
    }

    /// Use miles for both the distance projection and every radius on this query.
    ///
    /// This may precede or follow [`Self::nearby`] and [`Self::within`]; repeated
    /// calls are idempotent. One mile is exactly 1.609344 kilometres.
    pub fn in_miles(mut self) -> Result<Self, FrameworkError> {
        self.geo.miles = true;
        Ok(self)
    }
}

impl GeoQuery {
    /// A typed morph predicate cannot use one model's coordinates on another table.
    pub(super) fn validate_table(&self, table: &str) -> Result<(), FrameworkError> {
        if let Some(origin) = &self.origin
            && origin.table != table
        {
            return Err(FrameworkError::param(format!(
                "geo existence predicate for {} cannot target {table}",
                origin.table
            )));
        }
        Ok(())
    }

    pub(super) fn has_origin(&self) -> bool {
        self.origin.is_some()
    }

    /// Keep the computed projection on the model without changing its stored fields.
    pub(super) fn hydrate<M: EloquentModel>(
        &self,
        models: &mut [M],
        rows: &[QueryResult],
    ) -> Result<(), FrameworkError> {
        if self.has_origin() {
            for (model, row) in models.iter_mut().zip(rows) {
                let distance = row.try_get::<f64>("", "distance").map_err(|error| {
                    FrameworkError::database(format!("read geo distance: {error}"))
                })?;
                model.__set_geo_distance(distance);
            }
        }
        Ok(())
    }

    fn earth_radius(&self) -> f64 {
        if self.miles {
            EARTH_RADIUS_KM / KM_PER_MILE
        } else {
            EARTH_RADIUS_KM
        }
    }

    pub(super) fn distance(
        &self,
        backend: DbBackend,
        table: &str,
        values: &mut Vec<Value>,
        n: &mut usize,
    ) -> Result<Option<String>, FrameworkError> {
        let Some(origin) = &self.origin else {
            return Ok(None);
        };
        let (lat, lng) = origin.columns(backend, table);
        // Each occurrence binds separately so positional and numbered dialects agree.
        let lat1 = bind(backend, origin.lat, values, n)?;
        let lat2 = bind(backend, origin.lat, values, n)?;
        let lng1 = bind(backend, origin.lng, values, n)?;
        let a = format!(
            "(POWER(SIN(RADIANS({lat} - {lat1}) / 2.0), 2.0) + COS(RADIANS({lat2})) * COS(RADIANS({lat})) * POWER(SIN(RADIANS({lng} - {lng1}) / 2.0), 2.0))"
        );
        let (least, greatest) = if backend == DbBackend::Sqlite {
            ("MIN", "MAX")
        } else {
            ("LEAST", "GREATEST")
        };
        // Roundoff near antipodes can push a outside [0, 1]. Clamp before asin.
        Ok(Some(format!(
            "(2.0 * {} * ASIN(SQRT({least}(1.0, {greatest}(0.0, {a})))))",
            self.earth_radius()
        )))
    }

    pub(super) fn predicate(
        &self,
        backend: DbBackend,
        table: &str,
        values: &mut Vec<Value>,
        n: &mut usize,
    ) -> Result<Option<String>, FrameworkError> {
        let Some(origin) = &self.origin else {
            return Ok(None);
        };
        let (lat, lng) = origin.columns(backend, table);
        // A row without both coordinates has no distance. Excluding it here keeps
        // every engine in agreement; otherwise Postgres's GREATEST drops the NULL
        // and reports distance zero while SQLite and MySQL sort the row apart.
        let located = format!("{lat} IS NOT NULL AND {lng} IS NOT NULL");
        let Some(radius) = self.radius else {
            return Ok(Some(located));
        };
        let (min_lat, max_lat, min_lng, max_lng) = origin.bounds(radius / self.earth_radius());
        let low_lat = bind(backend, min_lat, values, n)?;
        let high_lat = bind(backend, max_lat, values, n)?;
        let low_lng = bind(backend, min_lng, values, n)?;
        let high_lng = bind(backend, max_lng, values, n)?;
        let longitude = if min_lng <= max_lng {
            format!("{lng} BETWEEN {low_lng} AND {high_lng}")
        } else {
            format!("({lng} >= {low_lng} OR {lng} <= {high_lng})")
        };
        let distance = self
            .distance(backend, table, values, n)?
            .ok_or_else(|| FrameworkError::internal("geo radius lost its origin"))?;
        let tolerance = if self.miles {
            RADIUS_TOLERANCE_KM / KM_PER_MILE
        } else {
            RADIUS_TOLERANCE_KM
        };
        let radius = bind(backend, radius + tolerance, values, n)?;
        Ok(Some(format!(
            "{located} AND {lat} BETWEEN {low_lat} AND {high_lat} AND {longitude} AND {distance} <= {radius}"
        )))
    }
}

impl Origin {
    fn columns(&self, backend: DbBackend, table: &str) -> (String, String) {
        let table = quote_identifier(backend, table);
        (
            format!("{table}.{}", quote_identifier(backend, self.columns.0)),
            format!("{table}.{}", quote_identifier(backend, self.columns.1)),
        )
    }

    fn bounds(&self, angular_radius: f64) -> (f64, f64, f64, f64) {
        // Expand by less than a millimetre so rounding cannot clip a circle's edge.
        let delta = (angular_radius + 1e-12).min(std::f64::consts::PI);
        let lat = self.lat.to_radians();
        let min_lat = (lat - delta).to_degrees().max(-90.0);
        let max_lat = (lat + delta).to_degrees().min(90.0);
        if min_lat <= -90.0 || max_lat >= 90.0 {
            return (min_lat, max_lat, -180.0, 180.0);
        }
        let longitude_delta = (delta.sin() / lat.cos())
            .clamp(-1.0, 1.0)
            .asin()
            .to_degrees();
        let wrap = |lng: f64| (lng + 180.0).rem_euclid(360.0) - 180.0;
        (
            min_lat,
            max_lat,
            wrap(self.lng - longitude_delta),
            wrap(self.lng + longitude_delta),
        )
    }
}

fn bind(
    backend: DbBackend,
    value: f64,
    values: &mut Vec<Value>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    *n += 1;
    values.push(value.into());
    let parameter = placeholder(backend, *n)?;
    // PostgreSQL can otherwise infer numeric at POWER and decode a float8 as numeric.
    Ok(if backend == DbBackend::Postgres {
        format!("CAST({parameter} AS DOUBLE PRECISION)")
    } else {
        parameter
    })
}
