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

/// Refuses a `u64` that `column` cannot store on `backend`: above
/// `i64::MAX` on Postgres and SQLite, which store a `u64` in a signed
/// `BIGINT`. MySQL's unsigned columns take every `u64`. `table` qualifies
/// the column in the message; `""` leaves it bare.
///
/// The error names the column and the value. It is raised before the
/// statement is built, because sea-query-sqlx's Postgres and SQLite
/// binders panic on the conversion rather than return an error.
pub(crate) fn refuse_unsigned_overflow(
    backend: DbBackend,
    table: &str,
    column: &str,
    value: &Value,
) -> Result<(), String> {
    match value {
        Value::BigUnsigned(Some(n)) if *n > i64::MAX as u64 && backend != DbBackend::MySql => {
            let column = if table.is_empty() {
                format!("`{column}`")
            } else {
                format!("`{table}.{column}`")
            };
            Err(format!(
                "{column}: {n} is above {}, the largest value a {backend:?} BIGINT \
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
    fn a_negative_value_does_not_convert() {
        assert!(<StoredU64 as ValueType>::try_from(Value::BigInt(Some(-1))).is_err());
        assert_eq!(
            <StoredU64 as ValueType>::try_from(Value::BigInt(Some(3))).ok(),
            Some(StoredU64(3))
        );
    }
}
