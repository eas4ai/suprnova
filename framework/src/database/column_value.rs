//! The type a query terminal reads out of a column when the caller names
//! it: `pluck::<T>`, `value::<T>`, `max::<T>`, `DB::scalar::<T>` and the
//! others.
//!
//! The terminals used to decode through SeaORM's `TryGetable`, which reads
//! a `u64` on MySQL only: on Postgres and SQLite `pluck::<u64>` found
//! nothing. [`ColumnValue`] covers every `TryGetable` type unchanged and
//! reads `u64` and `Option<u64>` the way a model's `u64` field does, so
//! callers write the same turbofish on every database.

use std::any::{Any, TypeId};

use bigdecimal::{BigDecimal, ToPrimitive};
use rust_decimal::Decimal;
use sea_orm::{ColIdx, DbErr, QueryResult, TryGetError, TryGetable};

use crate::eloquent::casts::StoredU64;

/// A value a query terminal reads out of a column: any SeaORM
/// [`TryGetable`] type, with `u64` and `Option<u64>` read on every
/// database.
///
/// Every `TryGetable` type implements it; there is nothing to implement.
/// A `u64` reads an unsigned MySQL column as it is, and a signed column,
/// the only kind Postgres and SQLite have, as a non-negative `i64`. A
/// negative value fails the read with an error that names the column.
pub trait ColumnValue: TryGetable + 'static {
    /// Reads the value at `index`, a column name or a position, of `row`.
    fn from_column<I: ColIdx>(row: &QueryResult, index: I) -> Result<Self, TryGetError>;
}

impl<T: TryGetable + 'static> ColumnValue for T {
    /// `u64` and `Option<u64>` cannot get an implementation of their own
    /// beside this one, because both are `TryGetable`, so they are told
    /// apart by type here and read through [`StoredU64`].
    fn from_column<I: ColIdx>(row: &QueryResult, index: I) -> Result<Self, TryGetError> {
        let wanted = TypeId::of::<T>();
        if wanted == TypeId::of::<u64>() {
            let stored = <StoredU64 as TryGetable>::try_get_by(row, index)?;
            return as_wanted::<T, u64>(stored.get());
        }
        if wanted == TypeId::of::<Option<u64>>() {
            let stored = <Option<StoredU64> as TryGetable>::try_get_by(row, index)?;
            return as_wanted::<T, Option<u64>>(stored.map(StoredU64::get));
        }
        T::try_get_by(row, index)
    }
}

/// Hands `value` back as `T`, which the caller has checked is `V`.
fn as_wanted<T: 'static, V: 'static>(value: V) -> Result<T, TryGetError> {
    let mut slot: Option<T> = None;
    match (&mut slot as &mut dyn Any).downcast_mut::<Option<V>>() {
        Some(out) => *out = Some(value),
        None => {
            return Err(TryGetError::DbErr(DbErr::Type(format!(
                "{} read as {}",
                std::any::type_name::<V>(),
                std::any::type_name::<T>()
            ))));
        }
    }
    slot.ok_or_else(|| {
        TryGetError::DbErr(DbErr::Type(format!(
            "no {} was read",
            std::any::type_name::<T>()
        )))
    })
}

/// [`ColumnValue::from_column`] for a terminal that skips a NULL - `pluck`
/// leaves the row out, `value` answers `None` - and fails on any other
/// value it cannot read. A value of the wrong kind, a negative one in a
/// `u64` among them, is an error rather than a missing row.
pub(crate) fn unless_null<T: ColumnValue>(
    row: &QueryResult,
    column: &str,
) -> Result<Option<T>, DbErr> {
    match T::from_column(row, column) {
        Ok(value) => Ok(Some(value)),
        Err(TryGetError::Null(_)) => Ok(None),
        Err(TryGetError::DbErr(error)) => Err(error),
    }
}

/// The number an aggregate answered, in the form the database sent it.
///
/// The database, not the caller, chooses the type of `SUM` and `AVG`:
/// Postgres answers `numeric` for the sum of a `bigint` and the average of
/// any integer, MySQL answers `DECIMAL` for both, and SQLite an integer or
/// a real. None of those decode as the integer or float a caller names, so
/// an aggregate is read as whichever arrived and converted exactly.
#[derive(Debug)]
enum Number {
    /// An integer: SQLite's sum of integers, Postgres's sum of `int`.
    Integer(i128),
    /// A real: any database's sum or average of a floating-point column,
    /// SQLite's average of anything, Postgres's sum of a `real`.
    Real(f64),
    /// An exact decimal: Postgres's `numeric`, MySQL's `DECIMAL`.
    Decimal(BigDecimal),
}

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(n) => write!(f, "{n}"),
            Self::Real(n) => write!(f, "{n}"),
            Self::Decimal(n) => write!(f, "{}", n.normalized()),
        }
    }
}

impl Number {
    /// Reads the number at `column` of `row`, in whichever of the three
    /// forms the database sent it. A NULL is [`TryGetError::Null`].
    fn read(row: &QueryResult, column: &str) -> Result<Self, TryGetError> {
        match <i64 as TryGetable>::try_get_by(row, column) {
            Ok(n) => return Ok(Self::Integer(n.into())),
            Err(TryGetError::Null(name)) => return Err(TryGetError::Null(name)),
            Err(TryGetError::DbErr(_)) => {}
        }
        // Postgres reads each integer width as its own type only: the
        // minimum of an `integer` column is an `int4`.
        if let Ok(n) = <i32 as TryGetable>::try_get_by(row, column) {
            return Ok(Self::Integer(n.into()));
        }
        if let Ok(n) = <i16 as TryGetable>::try_get_by(row, column) {
            return Ok(Self::Integer(n.into()));
        }
        if let Ok(n) = <u64 as TryGetable>::try_get_by(row, column) {
            return Ok(Self::Integer(n.into()));
        }
        if let Ok(n) = <f64 as TryGetable>::try_get_by(row, column) {
            return Ok(Self::Real(n));
        }
        // Postgres sums a `real` column as a `real`.
        if let Ok(n) = <f32 as TryGetable>::try_get_by(row, column) {
            return Ok(Self::Real(n.into()));
        }
        <BigDecimal as TryGetable>::try_get_by(row, column).map(Self::Decimal)
    }

    /// The number as a whole number, or an error when it has a fraction
    /// or is beyond any integer type, so it is never truncated.
    fn whole(&self, wanted: &str) -> Result<i128, DbErr> {
        let whole = match self {
            Self::Integer(n) => Some(*n),
            // Every f64 strictly between -2^127 and 2^127 that has no
            // fraction converts to i128 exactly.
            Self::Real(n) if n.fract() == 0.0 && n.abs() < 2f64.powi(127) => Some(*n as i128),
            Self::Real(_) => None,
            Self::Decimal(n) if n.is_integer() => n.to_i128(),
            Self::Decimal(_) => None,
        };
        whole.ok_or_else(|| {
            DbErr::Type(format!(
                "{self} is not a whole number in range, so it does not read as {wanted}"
            ))
        })
    }

    /// The number as a [`Decimal`], exactly: an integer, a decimal, or the
    /// shortest decimal that reads back as the same `f64`, which is all a
    /// real says. A number a `Decimal` cannot hold exactly (more than 28
    /// fractional digits, or beyond 96 bits) is an error, never rounded.
    fn exact_decimal(&self) -> Result<Decimal, DbErr> {
        let exact = match self {
            Self::Integer(n) => Decimal::try_from_i128_with_scale(*n, 0).ok(),
            Self::Real(n) if n.is_finite() => Decimal::from_str_exact(&n.to_string()).ok(),
            Self::Real(_) => None,
            Self::Decimal(n) => {
                let (digits, exponent) = n.normalized().as_bigint_and_exponent();
                let digits = digits.to_i128();
                match (digits, u32::try_from(exponent)) {
                    (Some(digits), Ok(scale)) => {
                        Decimal::try_from_i128_with_scale(digits, scale).ok()
                    }
                    // A negative exponent: trailing zeros of a whole number.
                    (Some(digits), Err(_)) => u32::try_from(exponent.unsigned_abs())
                        .ok()
                        .and_then(|zeros| 10i128.checked_pow(zeros))
                        .and_then(|power| digits.checked_mul(power))
                        .and_then(|whole| Decimal::try_from_i128_with_scale(whole, 0).ok()),
                    (None, _) => None,
                }
            }
        };
        exact.ok_or_else(|| DbErr::Type(format!("{self} does not fit in a Decimal exactly")))
    }

    /// The number as JSON: an integer exactly when JSON holds it, anything
    /// else as the nearest `f64`, and a NaN or an infinity as `null`.
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Integer(n) => i64::try_from(*n)
                .map(serde_json::Value::from)
                .or_else(|_| u64::try_from(*n).map(serde_json::Value::from))
                .unwrap_or_else(|_| float_json(*n as f64)),
            Self::Real(n) => float_json(*n),
            Self::Decimal(n) => {
                let n = n.normalized();
                match (n.is_integer(), n.to_i64(), n.to_u64()) {
                    (true, Some(whole), _) => serde_json::Value::from(whole),
                    (true, None, Some(whole)) => serde_json::Value::from(whole),
                    _ => float_json(n.to_f64().unwrap_or(f64::NAN)),
                }
            }
        }
    }

    /// The number as the nearest `f64`.
    fn nearest_f64(&self) -> Result<f64, DbErr> {
        match self {
            Self::Integer(n) => Ok(*n as f64),
            Self::Real(n) => Ok(*n),
            Self::Decimal(n) => n
                .to_f64()
                .ok_or_else(|| DbErr::Type(format!("{self} does not read as f64"))),
        }
    }
}

/// An `f64` as JSON, `null` for a NaN or an infinity, which JSON has no
/// number for.
fn float_json(n: f64) -> serde_json::Value {
    serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, serde_json::Value::Number)
}

/// Reads the result of an aggregate (`COUNT`, `SUM`, `AVG`) at `column`
/// as `T` on every database.
///
/// An integer `T` reads the result exactly: one with a fraction, or
/// outside `T`'s range, is an error that quotes it, never truncated or
/// wrapped. A [`Decimal`] reads it exactly too, or fails when it cannot
/// hold it. `f64` and `f32` read the nearest value. Any other `T` decodes
/// as [`ColumnValue`] does.
pub(crate) fn read_aggregate<T: ColumnValue>(row: &QueryResult, column: &str) -> Result<T, DbErr> {
    macro_rules! exactly {
        ($($ty:ty),*) => {$(
            if TypeId::of::<T>() == TypeId::of::<$ty>() {
                let number = Number::read(row, column)?;
                let wanted = stringify!($ty);
                let value = <$ty>::try_from(number.whole(wanted)?).map_err(|_| {
                    DbErr::Type(format!("{number} does not fit in {wanted}"))
                })?;
                return as_wanted::<T, $ty>(value).map_err(DbErr::from);
            }
        )*};
    }
    exactly!(
        i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
    );
    if TypeId::of::<T>() == TypeId::of::<Decimal>() {
        let value = Number::read(row, column)?.exact_decimal()?;
        return as_wanted::<T, Decimal>(value).map_err(DbErr::from);
    }
    if TypeId::of::<T>() == TypeId::of::<f64>() {
        let value = Number::read(row, column)?.nearest_f64()?;
        return as_wanted::<T, f64>(value).map_err(DbErr::from);
    }
    if TypeId::of::<T>() == TypeId::of::<f32>() {
        // Rounding to the nearest f64 and then to f32 can differ from
        // rounding once only in the last bit, which an f32 sum cannot
        // promise anyway.
        let value = Number::read(row, column)?.nearest_f64()? as f32;
        return as_wanted::<T, f32>(value).map_err(DbErr::from);
    }
    T::from_column(row, column).map_err(DbErr::from)
}

/// The type an average reads as: `f64`, or [`rust_decimal::Decimal`] for
/// an exact one.
///
/// The database chooses the type of `AVG`: Postgres answers `numeric`,
/// MySQL `DECIMAL`, and SQLite a real. An `f64` takes the nearest value,
/// which is what a floating-point column holds anyway. A `Decimal` takes
/// an exact one, as a money column's average needs: Postgres's and MySQL's
/// decimals and every integer exactly, a real as the shortest decimal that
/// reads back as the same `f64`, and an average it cannot hold is an error
/// rather than a rounded value.
///
/// Implemented for those two types only.
pub trait AvgValue: ColumnValue + Default + sealed::Sealed {}

impl AvgValue for f64 {}
impl AvgValue for Decimal {}

mod sealed {
    /// Keeps [`super::AvgValue`] to the types the average reader converts.
    pub trait Sealed {}

    impl Sealed for f64 {}
    impl Sealed for rust_decimal::Decimal {}
}

/// A relation aggregate (`with_sum`, `with_avg`, `with_min`, `with_max`) as
/// the code `#[suprnova::model]` generates stores it.
///
/// **Not part of the public API.** It is `pub` because that generated code
/// names it.
#[doc(hidden)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct __RelationAggregate {
    /// The value as the nearest `f64`, when the database answered a number.
    pub number: Option<f64>,
    /// The value as JSON, whatever its type: a number, text, or a date or
    /// a time as the ISO 8601 text serde writes for it. This is what
    /// `<rel>_min_as` and `<rel>_max_as` read, so the minimum or maximum of
    /// a date column is usable as Laravel's `withMax` attribute is.
    pub value: Option<serde_json::Value>,
}

/// Reads a relation aggregate at `column`, whatever type the database
/// answered: every integer width, a real, `numeric` / `DECIMAL`, text, a
/// date, a time or a date-time. A NULL, or a value of a type none of those
/// reads, is the empty aggregate, never an error: `with_min` and `with_max`
/// over a date column answered `None` before they could read it, and a page
/// that shows one renders either way.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::model]` generates for relation aggregates calls it.
#[doc(hidden)]
pub fn __relation_aggregate(row: &QueryResult, column: &str) -> __RelationAggregate {
    match Number::read(row, column) {
        Ok(number) => __RelationAggregate {
            number: number.nearest_f64().ok(),
            value: Some(number.to_json()),
        },
        Err(TryGetError::Null(_)) => __RelationAggregate::default(),
        Err(TryGetError::DbErr(_)) => __RelationAggregate {
            number: None,
            value: non_numeric_json(row, column),
        },
    }
}

/// A value that is not a number, as JSON: text as it is, and a date, a
/// time or a date-time as serde writes it. `None` for a NULL or any other
/// type.
fn non_numeric_json(row: &QueryResult, column: &str) -> Option<serde_json::Value> {
    fn read<T: TryGetable + serde::Serialize>(
        row: &QueryResult,
        column: &str,
    ) -> Option<serde_json::Value> {
        <T as TryGetable>::try_get_by(row, column)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok())
    }
    read::<String>(row, column)
        .or_else(|| read::<chrono::DateTime<chrono::Utc>>(row, column))
        .or_else(|| read::<chrono::NaiveDateTime>(row, column))
        .or_else(|| read::<chrono::NaiveDate>(row, column))
        .or_else(|| read::<chrono::NaiveTime>(row, column))
        .or_else(|| read::<bool>(row, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sum_reads_as_a_whole_number_only_when_it_is_one() {
        let decimal = |text: &str| Number::Decimal(text.parse().expect("a decimal"));
        assert_eq!(decimal("65.000").whole("i64").ok(), Some(65));
        assert_eq!(Number::Real(65.0).whole("i64").ok(), Some(65));
        assert_eq!(Number::Integer(-100).whole("u64").ok(), Some(-100));
        for fraction in [decimal("1.75"), Number::Real(1.75), Number::Real(f64::NAN)] {
            let error = fraction.whole("i64").expect_err("no whole number");
            assert!(error.to_string().contains("i64"), "{error}");
        }
        assert!(decimal("1e40").whole("i128").is_err());
        assert_eq!(decimal("17.5").nearest_f64().ok(), Some(17.5));
        let exact = |number: Number| number.exact_decimal().map(|d| d.to_string()).ok();
        assert_eq!(
            exact(decimal("12345678901234567.900000")).as_deref(),
            Some("12345678901234567.9")
        );
        assert_eq!(exact(decimal("1e3")).as_deref(), Some("1000"));
        assert_eq!(exact(Number::Integer(-100)).as_deref(), Some("-100"));
        assert_eq!(exact(Number::Real(0.375)).as_deref(), Some("0.375"));
        assert_eq!(exact(Number::Real(1e300)), None);
        assert_eq!(exact(Number::Real(f64::INFINITY)), None);
        assert_eq!(exact(decimal("0.12345678901234567890123456789")), None);
        assert_eq!(decimal("1.750").to_string(), "1.75");
    }

    #[test]
    fn a_value_comes_back_as_the_type_it_was_checked_to_be() {
        assert_eq!(as_wanted::<u64, u64>(7).ok(), Some(7));
        assert_eq!(as_wanted::<Option<u64>, Option<u64>>(None).ok(), Some(None));
        assert!(as_wanted::<i64, u64>(7).is_err());
    }
}
