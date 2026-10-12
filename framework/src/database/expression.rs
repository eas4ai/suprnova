//! Explicit SQL expressions keep trusted SQL separate from bound values.

use indexmap::IndexMap;
use sea_orm::{DbBackend, Value as SeaValue};
use serde_json::Value;

use super::clauses::{ReadSet, quote_identifier};
use crate::{Attrs, DB, DbTableBuilder, FrameworkError};

/// Trusted SQL that you deliberately write without a bound parameter.
#[derive(Debug, Clone)]
pub struct RawExpression(pub(crate) String);

/// A validated column, trusted expression or scalar subquery for query clauses.
#[derive(Debug, Clone)]
pub enum QueryExpression {
    /// Validate and quote a column so plain names cannot inject SQL.
    Column(String),
    /// Write explicitly trusted SQL without treating it as a value.
    Raw(RawExpression),
    /// Render a scalar subquery with the enclosing statement's bindings.
    Subquery(Box<DbTableBuilder>),
}

impl From<&str> for QueryExpression {
    fn from(value: &str) -> Self {
        Self::Column(value.into())
    }
}
impl From<String> for QueryExpression {
    fn from(value: String) -> Self {
        Self::Column(value)
    }
}
impl From<&String> for QueryExpression {
    fn from(value: &String) -> Self {
        Self::Column(value.clone())
    }
}
impl From<RawExpression> for QueryExpression {
    fn from(value: RawExpression) -> Self {
        Self::Raw(value)
    }
}
impl From<DbTableBuilder> for QueryExpression {
    fn from(value: DbTableBuilder) -> Self {
        Self::Subquery(Box::new(value))
    }
}

impl QueryExpression {
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        match self {
            Self::Column(name) => super::validate_identifier(name).map(|_| ()),
            Self::Raw(_) => Ok(()),
            Self::Subquery(query) => query.validate_inputs(),
        }
    }
    pub(crate) fn render(
        &self,
        backend: DbBackend,
        values: &mut Vec<SeaValue>,
        n: &mut usize,
    ) -> Result<String, FrameworkError> {
        match self {
            Self::Column(name) => Ok(quote_identifier(backend, name)),
            Self::Raw(raw) => Ok(raw.0.clone()),
            Self::Subquery(query) => Ok(format!(
                "({})",
                query.render_select_into(backend, values, n)?
            )),
        }
    }
    pub(crate) fn collect_tables(&self, out: &mut ReadSet) {
        match self {
            Self::Column(_) => {}
            Self::Raw(_) => out.raw_fragment = true,
            Self::Subquery(query) => query.collect_tables(out),
        }
    }
}

/// An update value distinguishes data to bind from SQL to evaluate.
#[derive(Debug, Clone)]
pub enum UpdateValue {
    /// Bind application data as a parameter.
    Bound(Value),
    /// Evaluate explicitly trusted SQL on the database.
    Raw(RawExpression),
}
impl From<Value> for UpdateValue {
    fn from(value: Value) -> Self {
        Self::Bound(value)
    }
}
impl From<RawExpression> for UpdateValue {
    fn from(value: RawExpression) -> Self {
        Self::Raw(value)
    }
}
macro_rules! bound_values {
    ($($ty:ty),*) => { $(impl From<$ty> for UpdateValue { fn from(value: $ty) -> Self { Self::Bound(value.into()) } })* };
}
bound_values!(
    &str, String, bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64
);

/// An ordered update map lets you mix bound data with raw expressions.
#[derive(Debug, Clone, Default)]
pub struct UpdateAttrs(pub(crate) IndexMap<String, UpdateValue>);
impl UpdateAttrs {
    /// Start an empty update so you can add both data and expressions.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a column assignment while preserving assignment order.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<UpdateValue>) -> &mut Self {
        self.0.insert(key.into(), value.into());
        self
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &UpdateValue)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub(crate) fn bound(&self) -> Attrs {
        Attrs(
            self.0
                .iter()
                .filter_map(|(key, value)| match value {
                    UpdateValue::Bound(value) => Some((key.clone(), value.clone())),
                    UpdateValue::Raw(_) => None,
                })
                .collect(),
        )
    }
}
impl From<Attrs> for UpdateAttrs {
    fn from(value: Attrs) -> Self {
        Self(
            value
                .0
                .into_iter()
                .map(|(key, value)| (key, UpdateValue::Bound(value)))
                .collect(),
        )
    }
}
impl<S: Into<String>, V: Into<UpdateValue>, const N: usize> From<[(S, V); N]> for UpdateAttrs {
    fn from(value: [(S, V); N]) -> Self {
        Self(
            value
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}
impl DB {
    /// Mark trusted SQL as an expression so query clauses never bind it as data.
    pub fn raw(expression: impl Into<String>) -> RawExpression {
        RawExpression(expression.into())
    }
}

/// Render the ORDER BY terms for a random order.
///
/// SQLite's `random()` cannot be seeded, so a seeded SQLite order is a fixed
/// function of the seed and each row's `key` instead, followed by `key`
/// itself: two terms, which both callers splice into an ORDER BY list. `key`
/// is the row's `rowid`, written as the ORDER BY can reach it: both builders
/// pass the main table's `rowid`, qualified with the table, and the model
/// builder passes the alias its union operands project the rowid under when
/// the order applies to a whole union. MySQL and Postgres seed their own
/// random functions and ignore `key`.
///
/// The same seed on the same rows always gives the same order, whatever
/// plan reads them. The key is the rowid and not a primary key because
/// SQLite's `%` casts its operands to INTEGER: a text key reads as the
/// integer its text starts with, or 0, so most UUID or ULID keys would share
/// one value of the first term and the order would fall back to key order.
/// Every table a migration creates has a rowid; a view or a `WITHOUT ROWID`
/// table has none, and the engine returns its error.
pub(crate) fn random_order(backend: DbBackend, seed: Option<u64>, key: &str) -> String {
    match (backend, seed) {
        (DbBackend::MySql, Some(seed)) => format!("RAND({seed})"),
        (DbBackend::MySql, None) => "RAND()".into(),
        (DbBackend::Postgres, Some(seed)) => {
            let seed = seed as f64 / u64::MAX as f64;
            // The uncorrelated subquery runs once. CASE evaluates that seed
            // before each row's random() without adding a column to SELECT *.
            format!(
                "CASE WHEN (SELECT setseed({seed:.17e})) IS NULL THEN RANDOM() ELSE RANDOM() END"
            )
        }
        (DbBackend::Sqlite, Some(seed)) => {
            // The key is reduced mod 2^32 before the offset is added, so
            // the sum stays below 2^33 in magnitude and the product below
            // 2^63: no step leaves SQLite's integer range, for any key.
            // The multiplier is odd, so multiplying mod 2^32 is a bijection
            // that spreads consecutive keys across the range. Keys that
            // agree mod 2^32 still share a value, and the key itself, the
            // second term, orders them, so two rows with distinct keys
            // never tie.
            let offset = seed % 4_294_967_296;
            format!(
                "(((({key} % 4294967296) + {offset}) % 4294967296) * 1640531527) % 4294967296, {key}"
            )
        }
        _ => "RANDOM()".into(),
    }
}
