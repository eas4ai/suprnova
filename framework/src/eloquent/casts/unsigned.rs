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
//! A read by such a value is not refused. On Postgres no row of a signed
//! integer column holds it, so a lookup by key finds nothing and a
//! comparison is answered without the value, true or false for every row.
//! SQLite compares it by its digits, as it does a literal: an INTEGER
//! column there can hold a REAL above `i64::MAX`, and its own comparison
//! is the true answer. A column whose type the query does not know takes
//! the value as the number the engine compares it as.

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

/// Whether `value` is a `u64` above `i64::MAX` that `backend` stores in a
/// signed integer column, Postgres's and SQLite's only kind: no integer
/// write of it fits, and the drivers there cannot bind it. MySQL's
/// unsigned columns hold every `u64`.
///
/// It says nothing about what such a column already holds. On Postgres a
/// signed integer column holds only integers, so no row matches the
/// value. On SQLite an INTEGER column can hold a REAL above `i64::MAX`,
/// from raw SQL or an older write, so there a comparison binds the value's
/// digits instead and SQLite answers it.
pub(crate) fn beyond_signed(backend: DbBackend, value: &Value) -> bool {
    matches!(value, Value::BigUnsigned(Some(n)) if *n > i64::MAX as u64)
        && backend != DbBackend::MySql
}

/// How a `u64` above `i64::MAX` binds when the type of the column it meets
/// is unknown - a `DB::table` column, a joined table's column, a raw
/// fragment - so that the engine compares or stores it as the number it
/// is, and the answer is the engine's own for whatever the column holds.
///
/// - MySQL takes it as an unsigned integer.
/// - Postgres takes it as `numeric`: a `bigint` or a `double precision`
///   column compares with a `numeric` exactly, and a `numeric` or `text`
///   column stores it exactly. Postgres refuses to store it in a `bigint`.
/// - SQLite takes its digits, to which it applies the column's affinity as
///   it does to a literal: an INTEGER, REAL or NUMERIC column compares it
///   as a number, a TEXT column as text.
pub(crate) fn exact_unsigned(backend: DbBackend, n: u64) -> Value {
    match backend {
        DbBackend::Postgres => Value::Decimal(Some(rust_decimal::Decimal::from(n))),
        DbBackend::Sqlite => Value::String(Some(n.to_string())),
        _ => Value::BigUnsigned(Some(n)),
    }
}

/// How each `u64` above `i64::MAX` in `large`, written to a column of
/// `table` whose type the write does not know, binds, by column.
///
/// MySQL and Postgres take the exact number (see [`exact_unsigned`]):
/// MySQL stores it in an unsigned column and refuses it for a signed one,
/// Postgres stores it in a `numeric` or `text` column and refuses it for a
/// `bigint`. Neither stores a rounded value in an integer column.
///
/// SQLite would: it stores digits too large for an integer as a REAL in a
/// column of INTEGER or NUMERIC affinity, silently. So on SQLite the
/// columns' affinities are read, only when such a value is written, and a
/// write to one of those columns is refused before anything is sent, as a
/// database error naming the column. A TEXT, BLOB or REAL column takes the
/// digits: as text, or as the nearest REAL a floating-point column holds
/// for any number.
pub(crate) async fn bind_large_unsigned<C: sea_orm::ConnectionTrait>(
    conn: &C,
    table: &str,
    large: &[(String, u64)],
) -> Result<std::collections::HashMap<String, Value>, FrameworkError> {
    let backend = conn.get_database_backend();
    let mut bound = std::collections::HashMap::new();
    if large.is_empty() {
        return Ok(bound);
    }
    let affinities = if backend == DbBackend::Sqlite {
        sqlite_affinities(conn, table).await?
    } else {
        std::collections::HashMap::new()
    };
    for (column, n) in large {
        if backend == DbBackend::Sqlite
            && let Some(affinity @ ("INTEGER" | "NUMERIC")) =
                affinities.get(column).map(String::as_str)
        {
            return Err(FrameworkError::database(format!(
                "`{table}.{column}`: {n} is above {}, the largest integer SQLite \
                 stores exactly; a column of {affinity} affinity would store it as a \
                 rounded REAL",
                i64::MAX,
            )));
        }
        bound.insert(column.clone(), exact_unsigned(backend, *n));
    }
    Ok(bound)
}

/// The affinity SQLite gives each column of `table`, from its declared
/// type, by the rules of section 3.1 of SQLite's datatype documentation.
async fn sqlite_affinities<C: sea_orm::ConnectionTrait>(
    conn: &C,
    table: &str,
) -> Result<std::collections::HashMap<String, String>, FrameworkError> {
    let rows = conn
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT name, type FROM pragma_table_info(?)",
            [Value::from(table)],
        ))
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;
    let mut affinities = std::collections::HashMap::new();
    for row in rows {
        let name: String = row
            .try_get("", "name")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let declared: String = row
            .try_get::<Option<String>>("", "type")
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .unwrap_or_default()
            .to_lowercase();
        let affinity = if declared.contains("int") {
            "INTEGER"
        } else if ["char", "clob", "text"]
            .iter()
            .any(|t| declared.contains(t))
        {
            "TEXT"
        } else if declared.is_empty() || declared.contains("blob") {
            "BLOB"
        } else if ["real", "floa", "doub"]
            .iter()
            .any(|t| declared.contains(t))
        {
            "REAL"
        } else {
            "NUMERIC"
        };
        affinities.insert(name, affinity.to_owned());
    }
    Ok(affinities)
}

/// The columns of `attrs` that hold a `u64` above `i64::MAX`, with it.
pub(crate) fn large_unsigned_attrs<'a>(
    attrs: impl IntoIterator<Item = (&'a str, &'a serde_json::Value)>,
) -> Vec<(String, u64)> {
    attrs
        .into_iter()
        .filter_map(|(column, value)| {
            value
                .as_u64()
                .filter(|n| *n > i64::MAX as u64)
                .map(|n| (column.to_owned(), n))
        })
        .collect()
}

/// How a value compared with, or written to, a text field without a cast
/// binds: a `u64` above `i64::MAX` as its digits, which is what a text
/// column holds; anything else as it is (`None`). Without it such a value
/// would bind as the number it is, which Postgres refuses to compare with
/// text.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::model]` generates for a model's column binder calls it.
#[doc(hidden)]
pub fn __bind_text(value: &serde_json::Value) -> Option<Value> {
    value
        .as_u64()
        .filter(|n| *n > i64::MAX as u64)
        .map(|n| Value::String(Some(n.to_string())))
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
