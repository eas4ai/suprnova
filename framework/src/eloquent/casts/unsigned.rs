//! `u64` model fields on every database.
//!
//! Laravel's `id()` and `foreignId()` create `BIGINT UNSIGNED` columns on
//! MySQL, so a model over such a table declares its keys `u64`. SeaORM
//! reads a `u64` on MySQL only, and sea-query-sqlx's Postgres and SQLite
//! binders unwrap the `u64` to `i64` conversion, so a value above
//! `i64::MAX` panics there. Neither database has unsigned integers: the
//! same model stores `0..=i64::MAX` in a signed `BIGINT`.
//!
//! The `#[suprnova::model]` macro gives every field declared `u64` or
//! `Option<u64>`, the primary key included, the cast [`AsU64`] or
//! [`AsOptionalU64`]. Their storage type, [`StoredU64`], reads the column
//! on each database, refusing a negative value with an error that names
//! the column. Writes bind it as a SeaORM `BigUnsigned`, and the framework
//! refuses one above `i64::MAX` on Postgres and SQLite before the statement
//! is built, naming the column.
//!
//! A read by such a value is not refused: no row of a signed column holds
//! it, so the answer is known without asking. A lookup by key finds
//! nothing, and a comparison is settled, true or false for every row.

use sea_orm::sea_query::{ArrayType, ColumnType, Nullable, ValueType, ValueTypeErr};
use sea_orm::{ColIdx, DbBackend, DbErr, QueryResult, TryFromU64, TryGetError, TryGetable, Value};
use serde::{Deserialize, Serialize};

use super::{Cast, DynCast, IntoDynCast};
use crate::error::FrameworkError;

/// A `u64` as a model's SeaORM entity stores it.
///
/// The `#[suprnova::model]` macro uses it for every `u64` and
/// `Option<u64>` field; the model struct keeps the plain `u64`. It exists
/// because SeaORM's own `u64` reads on MySQL only: this type reads an
/// unsigned MySQL column as it is, and a signed column on any database as
/// a non-negative `i64`. It serializes as the bare number.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct StoredU64(pub u64);

impl StoredU64 {
    /// The value.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for StoredU64 {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<u32> for StoredU64 {
    fn from(value: u32) -> Self {
        Self(value.into())
    }
}

impl From<u16> for StoredU64 {
    fn from(value: u16) -> Self {
        Self(value.into())
    }
}

impl From<u8> for StoredU64 {
    fn from(value: u8) -> Self {
        Self(value.into())
    }
}

impl From<StoredU64> for u64 {
    fn from(value: StoredU64) -> Self {
        value.0
    }
}

/// Parses the decimal digits of a `u64`, so a route parameter binds a
/// `u64`-keyed model: route-model binding parses the segment as the
/// entity's key type, which is this one.
impl std::str::FromStr for StoredU64 {
    type Err = std::num::ParseIntError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

/// The bare number, as the `u64` it holds displays, so the key of a bare
/// SeaORM model formats the way it did when the entity stored a `u64`.
impl std::fmt::Display for StoredU64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// The bare number, so a stored key can be a query's value, as the
/// soft-delete `find` passes it to `filter`.
impl From<StoredU64> for serde_json::Value {
    fn from(value: StoredU64) -> Self {
        serde_json::Value::from(value.0)
    }
}

impl From<StoredU64> for Value {
    fn from(value: StoredU64) -> Self {
        Value::BigUnsigned(Some(value.0))
    }
}

impl Nullable for StoredU64 {
    fn null() -> Value {
        Value::BigUnsigned(None)
    }
}

impl ValueType for StoredU64 {
    fn try_from(value: Value) -> Result<Self, ValueTypeErr> {
        match value {
            Value::BigUnsigned(Some(n)) => Ok(Self(n)),
            Value::BigInt(Some(n)) => <u64 as TryFrom<_>>::try_from(n)
                .map(Self)
                .map_err(|_| ValueTypeErr),
            Value::Int(Some(n)) => <u64 as TryFrom<_>>::try_from(n)
                .map(Self)
                .map_err(|_| ValueTypeErr),
            Value::Unsigned(Some(n)) => Ok(Self(n.into())),
            _ => Err(ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "StoredU64".to_owned()
    }

    fn array_type() -> ArrayType {
        ArrayType::BigUnsigned
    }

    fn column_type() -> ColumnType {
        ColumnType::BigUnsigned
    }
}

impl TryFromU64 for StoredU64 {
    fn try_from_u64(n: u64) -> Result<Self, DbErr> {
        Ok(Self(n))
    }
}

impl TryGetable for StoredU64 {
    /// An unsigned MySQL column decodes as `u64`. Every other integer
    /// column, signed on MySQL and the only kind Postgres and SQLite have,
    /// decodes as `i64` and must not be negative. The two decoders accept
    /// disjoint MySQL columns, so trying the unsigned one first never reads
    /// a signed column's bits as unsigned.
    fn try_get_by<I: ColIdx>(res: &QueryResult, index: I) -> Result<Self, TryGetError> {
        match <u64 as TryGetable>::try_get_by(res, index) {
            Ok(value) => return Ok(Self(value)),
            Err(TryGetError::Null(column)) => return Err(TryGetError::Null(column)),
            Err(TryGetError::DbErr(_)) => {}
        }
        let signed = <i64 as TryGetable>::try_get_by(res, index)?;
        <u64 as TryFrom<i64>>::try_from(signed)
            .map(Self)
            .map_err(|_| {
                TryGetError::DbErr(DbErr::Type(format!(
                    "column {} holds {signed}, which a u64 field cannot hold",
                    column_name(&index),
                )))
            })
    }
}

/// The column a decoder reads, as an error names it: `` `name` `` or the
/// position in the select list.
fn column_name<I: ColIdx>(index: &I) -> String {
    match (index.as_str(), index.as_usize()) {
        (Some(name), _) => format!("`{name}`"),
        (None, Some(position)) => format!("at position {position}"),
        (None, None) => format!("{index:?}"),
    }
}

/// Whether `value` is a `u64` that `backend` stores in a signed column
/// and no such column can hold: one above `i64::MAX` on Postgres or
/// SQLite. MySQL's unsigned columns hold every `u64`.
pub(crate) fn beyond_signed(backend: DbBackend, value: &Value) -> bool {
    matches!(value, Value::BigUnsigned(Some(n)) if *n > i64::MAX as u64)
        && backend != DbBackend::MySql
}

/// How a comparison of a signed column with a value above every value it
/// holds turns out on each row whose column is not NULL. Postgres and
/// SQLite store a `u64` in a signed `BIGINT`, so a query that compares one
/// with a larger `u64` already has its answer: it is rendered as that
/// answer rather than sent with a parameter the drivers there cannot bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settled {
    /// False on every row: `=`, `>`, `>=`, `IN`.
    Never,
    /// True on every row: `!=`, `<>`, `<`, `<=`, `NOT IN`.
    Always,
}

impl Settled {
    /// The outcome of `column <op> value` for a value above every value of
    /// the column, or `None` for an operator that does not order numbers,
    /// such as `LIKE` or `IS`.
    pub(crate) fn of_operator(op: &str) -> Option<Self> {
        match op.trim() {
            "=" | ">" | ">=" => Some(Self::Never),
            "!=" | "<>" | "<" | "<=" => Some(Self::Always),
            _ => None,
        }
    }

    /// The comparison as SQL over `column`: false, or true, on a row whose
    /// column holds a value, and NULL on a row whose column is NULL, as the
    /// comparison itself would be. Keeping the NULL is what lets a `NOT`
    /// or an `OR` around it mean what it meant: `NOT (quantity = x)` still
    /// leaves out a row without a quantity.
    pub(crate) fn sql(self, column: &str) -> String {
        match self {
            Self::Never => format!("({column} IS NULL AND NULL)"),
            Self::Always => format!("({column} IS NOT NULL OR NULL)"),
        }
    }
}

/// Refuses a `u64` that `column` cannot store on `backend`: above
/// `i64::MAX` on Postgres and SQLite, which store a `u64` in a signed
/// `BIGINT`. MySQL's unsigned columns take every `u64`. `table` qualifies
/// the column in the message; `""` leaves it bare.
///
/// The error names the column and the value. It is raised before the
/// statement is built, because sea-query-sqlx's Postgres and SQLite
/// binders panic on the conversion rather than return an error. It is for
/// writes and raw parameters, whose meaning a read cannot settle, and its
/// callers raise it as a database error: the text, which names the engine,
/// the table and the column, goes to the log, and a client gets the
/// generic 500 body unless debug is on.
pub(crate) fn refuse_unsigned_overflow(
    backend: DbBackend,
    table: &str,
    column: &str,
    value: &Value,
) -> Result<(), String> {
    match value {
        Value::BigUnsigned(Some(n)) if beyond_signed(backend, value) => {
            let column = if table.is_empty() {
                format!("`{column}`")
            } else {
                format!("`{table}.{column}`")
            };
            Err(format!(
                "{column}: {n} is above {}, the largest integer a {backend:?} column \
                 holds; {backend:?} has no unsigned integers, so a u64 column stores \
                 0 to {} there",
                i64::MAX,
                i64::MAX,
            ))
        }
        _ => Ok(()),
    }
}

// ---- AsU64 ------------------------------------------------------------------

/// Cast `u64` ↔ [`StoredU64`]. The model macro gives it to every field
/// declared `u64` that names no cast of its own, the primary key included.
pub struct AsU64;

impl Cast for AsU64 {
    type Runtime = u64;
    type Storage = StoredU64;

    fn to_storage(value: &u64) -> Result<StoredU64, FrameworkError> {
        Ok(StoredU64(*value))
    }

    fn from_storage(stored: &StoredU64) -> Result<u64, FrameworkError> {
        Ok(stored.0)
    }

    fn bind_json(value: &serde_json::Value) -> Option<Value> {
        unsigned_bind(value)
    }
}

/// A query binds a value above `i64::MAX` as a `BigUnsigned`, the column's
/// own type, rather than the text a JSON number that large otherwise
/// becomes. MySQL then receives the number, and on Postgres and SQLite the
/// query builder can tell a write of it from text and refuse it, naming the
/// column, before anything is sent. Smaller values bind as the signed
/// integer every database takes.
fn unsigned_bind(value: &serde_json::Value) -> Option<Value> {
    value
        .as_u64()
        .filter(|n| *n > i64::MAX as u64)
        .map(|n| Value::BigUnsigned(Some(n)))
}

/// How a value compared with, or written to, an integer field without a
/// cast binds: a `u64` above `i64::MAX` as an unsigned number, anything
/// else as it is (`None`). No integer column narrower than
/// `BIGINT UNSIGNED` holds such a value, so seeing it as the number it is
/// lets the query builder settle a comparison and refuse a write, where as
/// text Postgres refused the comparison and SQLite stored a rounded real.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::model]` generates for a model's column binder calls it.
#[doc(hidden)]
pub fn __bind_integer(value: &serde_json::Value) -> Option<Value> {
    unsigned_bind(value)
}

impl IntoDynCast for AsU64 {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(UnsignedDyn)
    }
}

// ---- AsOptionalU64 ----------------------------------------------------------

/// The nullable form of [`AsU64`], for a field declared `Option<u64>`.
pub struct AsOptionalU64;

impl Cast for AsOptionalU64 {
    type Runtime = Option<u64>;
    type Storage = Option<StoredU64>;

    fn to_storage(value: &Option<u64>) -> Result<Option<StoredU64>, FrameworkError> {
        Ok(value.map(StoredU64))
    }

    fn from_storage(stored: &Option<StoredU64>) -> Result<Option<u64>, FrameworkError> {
        Ok(stored.map(StoredU64::get))
    }

    fn bind_json(value: &serde_json::Value) -> Option<Value> {
        unsigned_bind(value)
    }
}

impl IntoDynCast for AsOptionalU64 {
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(UnsignedDyn)
    }
}

/// The erased form of both casts: [`StoredU64`] serializes as the bare
/// number, so the stored JSON is already the runtime JSON.
struct UnsignedDyn;

impl DynCast for UnsignedDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }

    fn from_storage_json_owned(
        &self,
        v: serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        Ok(v)
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_above_i64_max_is_refused_off_mysql_only() {
        let big = Value::BigUnsigned(Some(i64::MAX as u64 + 1));
        for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
            let error = refuse_unsigned_overflow(backend, "orders", "total", &big)
                .expect_err("a signed BIGINT cannot hold it");
            assert!(error.contains("`orders.total`"), "{error}");
            assert!(error.contains("9223372036854775808"), "{error}");
        }
        assert!(refuse_unsigned_overflow(DbBackend::MySql, "orders", "total", &big).is_ok());
        let fits = Value::BigUnsigned(Some(i64::MAX as u64));
        assert!(refuse_unsigned_overflow(DbBackend::Postgres, "orders", "total", &fits).is_ok());
    }

    #[test]
    fn a_comparison_beyond_every_signed_value_is_settled_by_its_operator() {
        let beyond = Value::BigUnsigned(Some(i64::MAX as u64 + 1));
        assert!(beyond_signed(DbBackend::Postgres, &beyond));
        assert!(beyond_signed(DbBackend::Sqlite, &beyond));
        assert!(!beyond_signed(DbBackend::MySql, &beyond));
        let held = Value::BigUnsigned(Some(i64::MAX as u64));
        assert!(!beyond_signed(DbBackend::Postgres, &held));

        for op in ["=", ">", ">="] {
            assert_eq!(Settled::of_operator(op), Some(Settled::Never), "{op}");
        }
        for op in ["!=", "<>", "<", "<="] {
            assert_eq!(Settled::of_operator(op), Some(Settled::Always), "{op}");
        }
        for op in ["LIKE", "NOT LIKE", "ILIKE", "IS", "IS NOT"] {
            assert_eq!(Settled::of_operator(op), None, "{op}");
        }
        // NULL where the column is NULL, so a NOT around it keeps SQL's
        // meaning for that row.
        assert_eq!(Settled::Never.sql("q"), "(q IS NULL AND NULL)");
        assert_eq!(Settled::Always.sql("q"), "(q IS NOT NULL OR NULL)");
    }

    #[test]
    fn only_values_above_i64_max_bind_unsigned() {
        assert_eq!(unsigned_bind(&serde_json::json!(5)), None);
        assert_eq!(
            unsigned_bind(&serde_json::json!(u64::MAX)),
            Some(Value::BigUnsigned(Some(u64::MAX)))
        );
        assert_eq!(unsigned_bind(&serde_json::json!(-1)), None);
    }

    #[test]
    fn the_storage_type_serializes_as_the_number() {
        assert_eq!(
            serde_json::to_value(StoredU64(u64::MAX)).expect("serialize"),
            serde_json::json!(u64::MAX)
        );
        assert_eq!(
            serde_json::from_value::<StoredU64>(serde_json::json!(7)).expect("deserialize"),
            StoredU64(7)
        );
    }

    #[test]
    fn the_storage_type_parses_and_displays_as_the_number() {
        assert_eq!(
            "18446744073709551615".parse::<StoredU64>().ok(),
            Some(StoredU64(u64::MAX))
        );
        for not_a_u64 in ["", "abc", "-1", "18446744073709551616"] {
            assert!(not_a_u64.parse::<StoredU64>().is_err(), "{not_a_u64:?}");
        }
        assert_eq!(StoredU64(42).to_string(), "42");
    }

    #[test]
    fn a_negative_value_does_not_convert() {
        assert!(<StoredU64 as ValueType>::try_from(Value::BigInt(Some(-1))).is_err());
        assert_eq!(
            <StoredU64 as ValueType>::try_from(Value::BigInt(Some(3))).ok(),
            Some(StoredU64(3))
        );
    }
}
