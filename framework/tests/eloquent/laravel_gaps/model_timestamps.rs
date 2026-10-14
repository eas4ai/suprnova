//! An explicit `timestamps = true` (or the bare `timestamps` flag) on a
//! model with neither timestamp column fails the build and names both
//! columns, as `#[suprnova::document]` does: the model asked for managed
//! timestamps, so silently managing none would hide the missing fields. A
//! model with both columns, with `timestamps = false`, or with the default
//! compiles as before.

use chrono::{DateTime, Utc};
use suprnova::{EloquentModel, model};

/// Asks for timestamps and has both columns: managed.
#[model(table = "gap_stamped_rows", timestamps = true)]
pub struct GapStampedRow {
    /// The database assigns the key.
    pub id: i64,
    /// Set when the row is created.
    pub created_at: DateTime<Utc>,
    /// Set when the row is written.
    pub updated_at: DateTime<Utc>,
}

/// Opts out and has neither column: unmanaged, no error.
#[model(table = "gap_unstamped_rows", timestamps = false)]
pub struct GapUnstampedRow {
    /// The database assigns the key.
    pub id: i64,
    /// A label gives the row content.
    pub name: String,
}

/// Says nothing and has neither column: the default turns timestamps off.
#[model(table = "gap_plain_rows")]
pub struct GapPlainRow {
    /// The database assigns the key.
    pub id: i64,
    /// A label gives the row content.
    pub name: String,
}

#[test]
fn explicit_timestamps_without_their_columns_fail_to_compile_naming_both() {
    trybuild::TestCases::new().compile_fail("tests/eloquent/compile_fail/*.rs");
}

#[test]
fn a_model_with_the_columns_with_false_or_with_the_default_compiles_as_before() {
    // Which of the three manage their timestamps, read as one value so the
    // comparison states the whole expectation.
    let managed = [
        <GapStampedRow as EloquentModel>::HAS_TIMESTAMPS,
        <GapUnstampedRow as EloquentModel>::HAS_TIMESTAMPS,
        <GapPlainRow as EloquentModel>::HAS_TIMESTAMPS,
    ];
    assert_eq!(managed, [true, false, false]);
}
