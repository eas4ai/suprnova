//! `#[model]` takes the key type from the primary-key field. A `key_type`
//! may repeat it, in any spelling of the same type.

use suprnova::eloquent::EloquentModel;
use suprnova::model;

#[model(table = "unsigned_keys")]
pub struct UnsignedKey {
    pub id: u64,
    pub name: String,
}

#[model(table = "string_keys", primary_key = "code", auto_increment = false)]
pub struct StringKey {
    pub code: String,
    pub name: String,
}

#[model(table = "spelled_keys", key_type = "::core::primitive::u64")]
pub struct SpelledKey {
    pub id: u64,
}

#[model(table = "default_keys")]
pub struct DefaultKey {
    pub id: i64,
}

/// Compiles only while each key type is its primary-key field's type.
pub fn keys(
    unsigned: <UnsignedKey as EloquentModel>::Key,
    string: <StringKey as EloquentModel>::Key,
    spelled: <SpelledKey as EloquentModel>::Key,
    default: <DefaultKey as EloquentModel>::Key,
) -> (u64, String, u64, i64) {
    (unsigned, string, spelled, default)
}
