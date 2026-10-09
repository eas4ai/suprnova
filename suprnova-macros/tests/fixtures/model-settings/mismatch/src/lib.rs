//! A `key_type` that disagrees with the primary-key field.

use suprnova::model;

#[model(table = "mismatched_keys", key_type = "i64")]
pub struct MismatchedKey {
    pub id: u64,
}
