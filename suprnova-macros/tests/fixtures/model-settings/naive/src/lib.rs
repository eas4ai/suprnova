//! Pins inherited naive casts and the explicit overrides in a package that declares the setting.

use suprnova::StoredDateTime;
use suprnova::chrono::{DateTime, Utc};
use suprnova::model;

/// A model whose storage types prove the package setting reaches managed and ordinary fields.
#[model(table = "naive_records", timestamps, soft_deletes, casts = {
    explicit_at = suprnova::AsDateTime,
    explicit_optional = suprnova::AsOptionalDateTime,
})]
pub struct NaiveRecord {
    /// The key uses the ordinary unsigned Rust type.
    pub id: u64,
    /// An ordinary field inherits the package cast.
    pub event_at: DateTime<Utc>,
    /// Its optional counterpart inherits the optional cast.
    pub optional_at: Option<DateTime<Utc>>,
    /// An explicit text cast overrides the package setting.
    pub explicit_at: DateTime<Utc>,
    /// An optional explicit cast also overrides the setting.
    pub explicit_optional: Option<DateTime<Utc>>,
    /// The managed creation timestamp inherits the package cast.
    pub created_at: DateTime<Utc>,
    /// The managed update timestamp inherits the package cast.
    pub updated_at: DateTime<Utc>,
    /// The managed soft-delete timestamp inherits the optional cast.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Compiles only when each inherited and overridden cast has its promised storage type.
pub fn storage(row: naive_record::Model) {
    let _: StoredDateTime = row.event_at;
    let _: Option<StoredDateTime> = row.optional_at;
    let _: StoredDateTime = row.created_at;
    let _: StoredDateTime = row.updated_at;
    let _: Option<StoredDateTime> = row.deleted_at;
    let _: String = row.explicit_at;
    let _: Option<String> = row.explicit_optional;
}
