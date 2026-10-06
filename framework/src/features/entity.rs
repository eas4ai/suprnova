//! Suprnova model for the `features` table, in laravel/pennant's layout.
//!
//! The columns are the ones Pennant's migration creates:
//!
//! * `name` + `scope` form a UNIQUE composite key. `scope` is Pennant's
//!   serialized scope: `__laravel_null` for a global flag,
//!   `App\Models\User|42` for a model, or a plain string.
//! * `value` is the flag's JSON value. Pennant reads any value other than
//!   `false` as active; the framework writes `true` or `false`.
//! * `created_at` and `updated_at` are nullable.
//!
//! The evaluator and the admin facade read and write the table through
//! [`crate::features::store`], which maps Pennant's scopes to the
//! framework's `scope_key`s. This model is for an application that wants
//! the rows themselves. The admin facade's description and actor live in
//! `suprnova_feature_details`.
//!
//! Schema lives in [`crate::features::migrations::CreateFeaturesTable`].

use chrono::{DateTime, Utc};

/// Row in the `features` table.
///
/// The timestamps use the naive casts, which read Laravel's `TIMESTAMP`
/// columns on MySQL, `timestamp` on Postgres and the text SQLite stores.
#[suprnova::model(
    table = "features",
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
)]
pub struct Feature {
    /// Primary key.
    pub id: u64,
    /// Flag identifier (e.g. `"checkout.v2"`).
    pub name: String,
    /// Pennant's serialized scope: `__laravel_null` for the global flag.
    pub scope: String,
    /// The flag's JSON value; anything but `false` is active.
    pub value: String,
    /// Timestamp at which the row was inserted.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp at which the row was last mutated.
    pub updated_at: Option<DateTime<Utc>>,
}

// Re-export the SeaORM types the macro emits inside the per-struct inner
// module, as the rest of the framework names them.
pub use feature::Model;
pub use feature::{ActiveModel, Column, Entity};
