//! Compiles only while the manifest's tables apply: the key comes from the
//! `id` field, and every date-time field stores through the native cast.

use chrono::{DateTime, Utc};
use suprnova::eloquent::EloquentModel;
use suprnova::model;

#[model(table = "orders", soft_deletes = true)]
pub struct Order {
    pub id: u64,
    pub user_id: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// The key is `u64` with no `key_type`.
pub fn key(id: <Order as EloquentModel>::Key) -> u64 {
    id
}

/// The stored columns are native date-times, not RFC 3339 text.
pub fn storage(row: order::Model) -> (DateTime<Utc>, DateTime<Utc>, Option<DateTime<Utc>>) {
    (row.created_at, row.updated_at, row.deleted_at)
}

#[suprnova::main(flavor = "current_thread")]
async fn main() {
    assert!(
        suprnova::boot::unsigned_ids(),
        "`unsigned_ids = true` reaches the schema builder"
    );
}
