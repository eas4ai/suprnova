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
//! The one exception is a column whose cast stores a new value on every
//! write, such as an encrypted column, whose ciphertext changes with each
//! save. Its stored values always differ, so it is compared by its decoded
//! value instead, as Laravel's `originalIsEquivalent` decrypts before it
//! compares.
//!
//! The lifecycle follows Laravel's `Model::save`. Right after the write,
//! [`record_save`] does what `syncChanges` does: it stores the save's
//! changes, unless the save changed nothing, in which case the previous
//! changes stay. Until the save returns, the original stays the row the save
//! started from, so the `updated` and `saved` observers read the values
//! loaded before it. [`finish_save`] then does what `finishSave` does with
//! `syncOriginal`: the original becomes the saved row.
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

/// Whether `column` holds the same value in two stored rows of one model
/// once decoded, asked only for a column whose stored values differ. It is
/// the model's `Model::__decoded_values_equal`, which the
/// `#[suprnova::model]` macro emits because only the macro can name the row
/// type and the casts.
pub(crate) type DecodedEqual =
    fn(&str, &(dyn Any + Send + Sync), &(dyn Any + Send + Sync)) -> Result<bool, FrameworkError>;

/// A save between its write and its return.
#[derive(Clone)]
struct SaveInProgress {
    /// The row the save started from, which `get_original` reads until the
    /// save returns. `None` when the instance was never read from the
    /// database.
    before: Option<Arc<dyn StoredRow>>,
}

/// One instance's view of its row. Never edited in place: each step builds
/// a new one, so a reader that cloned the old one keeps a consistent view.
#[derive(Clone, Default)]
struct History {
    /// The row as the database held it after the instance's last read or
    /// save: what the next save compares against, and the original once no
    /// save is in progress.
    synced: Option<Arc<dyn StoredRow>>,
    /// Set from a save's write until the save returns.
    in_progress: Option<SaveInProgress>,
    /// The changes of the last save that changed something, or `None`
    /// before any such save.
    changes: Option<Arc<Attrs>>,
}

/// The row history of one model instance.
///
/// Behind a lock because [`Model::save`](crate::eloquent::Model::save) takes
/// `&self` and still has to leave its record on the instance. The lock is
/// held only to clone or replace a few `Arc`s, never across an `.await`.
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
                in_progress: None,
                changes: None,
            }),
        }
    }

    // The critical sections below clone or assign a few `Arc`s and run no
    // user code, so a poisoned lock still holds a whole `History`: recover
    // it rather than lose the record.
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
/// that was saved. Laravel's `syncChanges`, run where `performUpdate` runs
/// it: after the write, before the `updated` event.
///
/// A column is changed when its stored value after the write differs from
/// the value the instance held before it, unless `decoded_equal` finds the
/// two stored values decode to the same value: that is how a column whose
/// cast stores a new value on every write, an encrypted one, counts as
/// changed only when its decoded value changed. When the instance was never
/// read from the database there is nothing to compare with, and every
/// column counts as changed, as in Laravel, where an attribute missing from
/// `$original` is dirty. A save that changed nothing keeps `before`'s
/// changes: Laravel skips such a save's `syncChanges`.
///
/// Until [`finish_save`] runs, `after`'s original is the row the save
/// started from.
///
/// Does nothing when `after` has no row, which is the case only for a
/// model type that does not keep one.
pub(crate) fn record_save(
    before: Option<&RowState>,
    after: Option<&RowState>,
    decoded_equal: DecodedEqual,
) -> Result<(), FrameworkError> {
    let Some(after) = after else {
        return Ok(());
    };
    let Some(after_row) = after.read().synced else {
        return Ok(());
    };
    let before_history = before.map(RowState::read).unwrap_or_default();
    let before_row = before_history.synced;

    let after_json = after_row.to_json()?;
    let before_json = before_row.as_ref().map(|row| row.to_json()).transpose()?;
    let mut changes = Attrs::new();
    for (column, value) in after_json {
        let unchanged = match (&before_row, before_json.as_ref()) {
            (Some(before_row), Some(before_json)) => match before_json.get(&column) {
                Some(old) if *old == value => true,
                Some(_) => same_decoded_value(
                    decoded_equal,
                    &column,
                    before_row.as_any(),
                    after_row.as_any(),
                ),
                None => false,
            },
            _ => false,
        };
        if !unchanged {
            changes.insert(column, value);
        }
    }
    let changes = if changes.is_empty() {
        before_history.changes
    } else {
        Some(Arc::new(changes))
    };

    after.write(History {
        synced: Some(after_row),
        in_progress: Some(SaveInProgress { before: before_row }),
        changes,
    });
    Ok(())
}

/// Whether `column`, whose stored value differs between the two rows,
/// decodes to the same value in both.
///
/// A value that no longer decodes counts as changed, which is what the
/// stored rows say. The save's write has already landed by now, so failing
/// the save over its change record would report a write that happened as
/// one that did not. The failure is logged, because a row that decoded when
/// it was read and no longer does is a defect to find.
fn same_decoded_value(
    decoded_equal: DecodedEqual,
    column: &str,
    before: &(dyn Any + Send + Sync),
    after: &(dyn Any + Send + Sync),
) -> bool {
    match decoded_equal(column, before, after) {
        Ok(same) => same,
        Err(error) => {
            tracing::warn!(
                target: "suprnova::eloquent",
                column,
                error = %error,
                "could not compare a column's decoded values after a save; it is recorded as changed",
            );
            false
        }
    }
}

/// End the save in progress on `state`: its original becomes the saved row.
/// Laravel's `syncOriginal`, run where `finishSave` runs it: after the
/// `saved` event.
pub(crate) fn finish_save(state: Option<&RowState>) {
    let Some(state) = state else {
        return;
    };
    let mut history = state.read();
    if history.in_progress.take().is_some() {
        state.write(history);
    }
}

/// Whether the last save that changed something changed any of
/// `attributes`, or anything at all when `attributes` is empty. `false`
/// before any such save.
pub(crate) fn was_changed_any(state: Option<&RowState>, attributes: &[&str]) -> bool {
    let Some(changes) = state.and_then(|state| state.read().changes) else {
        return false;
    };
    if attributes.is_empty() {
        return !changes.is_empty();
    }
    attributes
        .iter()
        .any(|attribute| changes.contains_key(attribute))
}

/// The columns the last save that changed something changed, each with
/// the value it stored. Empty before any such save.
pub(crate) fn changes(state: Option<&RowState>) -> Attrs {
    state
        .and_then(|state| state.read().changes)
        .map(|changes| changes.as_ref().clone())
        .unwrap_or_default()
}

/// The row `get_original` reads: the row a save in progress started from,
/// otherwise the row as last read or saved. `None` when there is no such
/// row because the instance was never read from the database.
pub(crate) fn original_row(state: Option<&RowState>) -> Option<Arc<dyn StoredRow>> {
    let history = state?.read();
    match history.in_progress {
        Some(save) => save.before,
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

    /// A model whose casts all store deterministically: differing stored
    /// values are a change.
    fn stored_only(
        _column: &str,
        _before: &(dyn Any + Send + Sync),
        _after: &(dyn Any + Send + Sync),
    ) -> Result<bool, FrameworkError> {
        Ok(false)
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
        record_save(Some(&before), Some(&after), stored_only).unwrap();

        assert!(was_changed_any(Some(&after), &["flag"]));
        assert!(!was_changed_any(Some(&after), &["name", "id"]));
        assert!(was_changed_any(Some(&after), &[]));
        let changes = changes(Some(&after));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes.get("flag"), Some(&json!(1)));
    }

    #[test]
    fn the_original_is_the_row_before_the_save_until_the_save_finishes() {
        let before = RowState::loaded(row("a", 0));
        let after = RowState::loaded(row("a", 1));
        record_save(Some(&before), Some(&after), stored_only).unwrap();
        assert_eq!(raw_original(Some(&after), "flag"), Some(json!(0)));

        finish_save(Some(&after));
        assert_eq!(raw_original(Some(&after), "flag"), Some(json!(1)));
        assert!(
            was_changed_any(Some(&after), &["flag"]),
            "finishing keeps the changes"
        );
    }

    #[test]
    fn a_save_that_changes_nothing_keeps_the_previous_changes() {
        let loaded = RowState::loaded(row("a", 0));
        let first = RowState::loaded(row("a", 1));
        record_save(Some(&loaded), Some(&first), stored_only).unwrap();
        finish_save(Some(&first));

        let second = RowState::loaded(row("a", 1));
        record_save(Some(&first), Some(&second), stored_only).unwrap();
        finish_save(Some(&second));

        assert!(was_changed_any(Some(&second), &["flag"]));
        assert_eq!(changes(Some(&second)).len(), 1);
    }

    #[test]
    fn without_a_loaded_row_every_column_is_changed() {
        let after = RowState::loaded(row("a", 1));
        record_save(Some(&RowState::default()), Some(&after), stored_only).unwrap();

        assert_eq!(changes(Some(&after)).len(), 3);
        assert_eq!(raw_original(Some(&after), "flag"), None);
        finish_save(Some(&after));
        assert_eq!(raw_original(Some(&after), "flag"), Some(json!(1)));
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
        record_save(Some(&source), Some(&after), stored_only).unwrap();
        source.adopt(&after);

        assert!(was_changed_any(Some(&source), &["name"]));
        assert!(!was_changed_any(Some(&copy), &[]));
    }
}
