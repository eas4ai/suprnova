//! What a model instance remembers about its row, so it can answer
//! [`Model::was_changed`](crate::eloquent::Model::was_changed),
//! [`Model::get_changes`](crate::eloquent::Model::get_changes),
//! [`Model::get_original`](crate::eloquent::Model::get_original) and
//! [`Model::get_raw_original`](crate::eloquent::Model::get_raw_original).
//!
//! Laravel keeps an `$original` array beside a model's attributes and
//! compares the two when it saves. A Suprnova model is a plain struct whose
//! fields the caller writes directly, so by the time `save` runs the loaded
//! values are gone from the fields. The row is kept instead, as the database
//! returned it, when the `#[suprnova::model]` macro hydrates the model, and
//! a save compares that row with the row the database returns after the
//! write.
//!
//! Comparing stored rows rather than fields means a column a `Saving`
//! listener rewrote, or a timestamp the framework set, counts like a column
//! the caller changed: the record says what the database holds now that it
//! did not hold before.
//!
//! The kept row lives in the model's
//! [`EagerLoadCache`](crate::EagerLoadCache), the per-instance runtime state
//! the macro already injects, so no new field appears on user structs.

use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::eloquent::attrs::Attrs;
use crate::error::FrameworkError;

/// A stored row with its concrete type erased, so the per-model cache can
/// hold it without being generic over the model.
pub(crate) trait StoredRow: Send + Sync {
    /// The row as a JSON object keyed by column: what the comparison and
    /// `get_raw_original` read.
    fn to_json(&self) -> Result<Map<String, Value>, FrameworkError>;

    /// The row itself, for the macro-emitted hook that turns it back into
    /// a model to read a cast value.
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

impl<R> StoredRow for R
where
    R: Serialize + Send + Sync + 'static,
{
    fn to_json(&self) -> Result<Map<String, Value>, FrameworkError> {
        match serde_json::to_value(self) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(FrameworkError::internal(
                "a stored model row did not serialize to a JSON object",
            )),
            Err(error) => Err(FrameworkError::internal(format!(
                "could not serialize a stored model row: {error}"
            ))),
        }
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}

/// The record one save leaves.
struct LastSave {
    /// The row as the instance knew it before the save. `None` when the
    /// instance was never read from the database.
    before: Option<Arc<dyn StoredRow>>,
    /// Each column the save changed, with the value it stored.
    changes: Attrs,
}

/// One instance's view of its row. Never edited in place: a save builds a
/// new one, so a reader that cloned the old one keeps a consistent view.
#[derive(Clone, Default)]
struct History {
    /// The row as the database held it after the instance's last read or
    /// save: what the next save compares against.
    synced: Option<Arc<dyn StoredRow>>,
    /// The last save, or `None` until one succeeds.
    last_save: Option<Arc<LastSave>>,
}

/// The row history of one model instance.
///
/// Behind a lock because [`Model::save`](crate::eloquent::Model::save) takes
/// `&self` and still has to leave its record on the instance. The lock is
/// held only to clone or replace two `Arc`s, never across an `.await`.
#[derive(Default)]
pub(crate) struct RowState {
    history: Mutex<History>,
}

impl RowState {
    /// The state of an instance just read from the database: `row` is both
    /// the base of the next comparison and the original, and nothing has
    /// been saved.
    pub(crate) fn loaded<R>(row: R) -> Self
    where
        R: Serialize + Send + Sync + 'static,
    {
        Self {
            history: Mutex::new(History {
                synced: Some(Arc::new(row)),
                last_save: None,
            }),
        }
    }

    // The critical sections below clone or assign a pair of `Arc`s and run
    // no user code, so a poisoned lock still holds a whole `History`:
    // recover it rather than lose the record.
    fn read(&self) -> History {
        self.history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn write(&self, history: History) {
        *self.history.lock().unwrap_or_else(PoisonError::into_inner) = history;
    }

    /// Take over `other`'s history. `save` uses it to leave on the caller's
    /// instance the record it built on the row the write returned.
    pub(crate) fn adopt(&self, other: &RowState) {
        self.write(other.read());
    }
}

impl Clone for RowState {
    /// A clone keeps the history, as cloning a Laravel model keeps its
    /// original and its changes.
    fn clone(&self) -> Self {
        Self {
            history: Mutex::new(self.read()),
        }
    }
}

/// Record a save on `after`, the state of the model hydrated from the row
/// the write returned, comparing it with `before`, the state of the model
/// that was saved.
///
/// A column is changed when its stored value after the write differs from
/// the value the instance held before it. When the instance was never read
/// from the database there is nothing to compare with, and every column
/// counts as changed, as in Laravel, where an attribute missing from
/// `$original` is dirty.
///
/// Does nothing when `after` has no row, which is the case only for a
/// model type that does not keep one.
pub(crate) fn record_save(
    before: Option<&RowState>,
    after: Option<&RowState>,
) -> Result<(), FrameworkError> {
    let Some(after) = after else {
        return Ok(());
    };
    let Some(after_row) = after.read().synced else {
        return Ok(());
    };
    let before_row = before.and_then(|state| state.read().synced);

    let after_json = after_row.to_json()?;
    let before_json = before_row.as_ref().map(|row| row.to_json()).transpose()?;
    let mut changes = Attrs::new();
    for (column, value) in after_json {
        let unchanged = before_json
            .as_ref()
            .and_then(|before| before.get(&column))
            .is_some_and(|old| *old == value);
        if !unchanged {
            changes.insert(column, value);
        }
    }

    after.write(History {
        synced: Some(after_row),
        last_save: Some(Arc::new(LastSave {
            before: before_row,
            changes,
        })),
    });
    Ok(())
}

/// Whether the last save changed any of `attributes`, or anything at all
/// when `attributes` is empty. `false` before any save.
pub(crate) fn was_changed_any(state: Option<&RowState>, attributes: &[&str]) -> bool {
    let Some(save) = state.and_then(|state| state.read().last_save) else {
        return false;
    };
    if attributes.is_empty() {
        return !save.changes.is_empty();
    }
    attributes
        .iter()
        .any(|attribute| save.changes.contains_key(attribute))
}

/// The columns the last save changed, each with the value it stored. Empty
/// before any save.
pub(crate) fn changes(state: Option<&RowState>) -> Attrs {
    state
        .and_then(|state| state.read().last_save)
        .map(|save| save.changes.clone())
        .unwrap_or_default()
}

/// The row as it was before the last save, or as it was loaded when there
/// has been no save. `None` when the instance was never read from the
/// database.
pub(crate) fn original_row(state: Option<&RowState>) -> Option<Arc<dyn StoredRow>> {
    let history = state?.read();
    match history.last_save {
        Some(save) => save.before.clone(),
        None => history.synced,
    }
}

/// The stored value of `attribute` in [`original_row`]. `None` when there
/// is no such row or no such column.
///
/// A row that cannot be represented as JSON reads as `None` too, the rule
/// [`Model::field_value`](crate::eloquent::Model::field_value) follows. It
/// is logged, because a model row that serde cannot serialize is a defect
/// to find, not an absent value.
pub(crate) fn raw_original(state: Option<&RowState>, attribute: &str) -> Option<Value> {
    let row = original_row(state)?;
    match row.to_json() {
        Ok(mut map) => map.remove(attribute),
        Err(error) => {
            tracing::warn!(
                target: "suprnova::eloquent",
                error = %error,
                "get_raw_original could not read the kept row",
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Clone, Serialize)]
    struct Row {
        id: i64,
        name: String,
        flag: i64,
    }

    fn row(name: &str, flag: i64) -> Row {
        Row {
            id: 1,
            name: name.to_string(),
            flag,
        }
    }

    #[test]
    fn a_save_records_only_the_columns_whose_stored_value_differs() {
        let before = RowState::loaded(row("a", 0));
        let after = RowState::loaded(row("a", 1));
        record_save(Some(&before), Some(&after)).unwrap();

        assert!(was_changed_any(Some(&after), &["flag"]));
        assert!(!was_changed_any(Some(&after), &["name", "id"]));
        assert!(was_changed_any(Some(&after), &[]));
        let changes = changes(Some(&after));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes.get("flag"), Some(&json!(1)));
        assert_eq!(raw_original(Some(&after), "flag"), Some(json!(0)));
    }

    #[test]
    fn without_a_loaded_row_every_column_is_changed() {
        let after = RowState::loaded(row("a", 1));
        record_save(Some(&RowState::default()), Some(&after)).unwrap();

        assert_eq!(changes(Some(&after)).len(), 3);
        assert_eq!(raw_original(Some(&after), "flag"), None);
    }

    #[test]
    fn before_a_save_nothing_changed_and_the_original_is_the_loaded_row() {
        let state = RowState::loaded(row("a", 0));
        assert!(!was_changed_any(Some(&state), &[]));
        assert!(changes(Some(&state)).is_empty());
        assert_eq!(raw_original(Some(&state), "name"), Some(json!("a")));
        assert_eq!(raw_original(None, "name"), None);
    }

    #[test]
    fn a_clone_is_independent_of_its_source() {
        let source = RowState::loaded(row("a", 0));
        let copy = source.clone();
        let after = RowState::loaded(row("b", 0));
        record_save(Some(&source), Some(&after)).unwrap();
        source.adopt(&after);

        assert!(was_changed_any(Some(&source), &["name"]));
        assert!(!was_changed_any(Some(&copy), &[]));
    }
}
