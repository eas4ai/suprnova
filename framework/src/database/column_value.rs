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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_comes_back_as_the_type_it_was_checked_to_be() {
        assert_eq!(as_wanted::<u64, u64>(7).ok(), Some(7));
        assert_eq!(as_wanted::<Option<u64>, Option<u64>>(None).ok(), Some(None));
        assert!(as_wanted::<i64, u64>(7).is_err());
    }
}
