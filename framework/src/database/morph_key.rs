//! Binding a polymorphic key in the type of the column that holds it.
//!
//! Laravel's `morphs` makes `*_id` a big integer, `uuidMorphs` a UUID and
//! `ulidMorphs` 26 characters; spatie/laravel-permission's `model_id` is a
//! big integer unless the application changes it. The framework carries
//! such a key as text (`Authenticatable::get_auth_identifier`, a
//! notification's route). Postgres compares no text with an integer or a
//! `uuid`, so the key binds as the type its own shape names: a number as an
//! integer, a UUID as Postgres's `uuid`, anything else as text. MySQL and
//! SQLite convert in the comparison either way.

use sea_orm::{DatabaseBackend, Value};

/// `raw` as the value a polymorphic key column takes.
pub(crate) fn morph_key_value(backend: DatabaseBackend, raw: &str) -> Value {
    if let Ok(number) = raw.parse::<i64>() {
        return Value::BigInt(Some(number));
    }
    uuid_value(backend, raw)
}

/// `raw` as a UUID column takes it: Postgres's native `uuid` when it is
/// one, text otherwise (`CHAR(36)` on MySQL, text on SQLite).
pub(crate) fn uuid_value(backend: DatabaseBackend, raw: &str) -> Value {
    if backend == DatabaseBackend::Postgres
        && let Ok(uuid) = uuid::Uuid::parse_str(raw)
    {
        return Value::Uuid(Some(uuid));
    }
    Value::String(Some(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_binds_as_its_shape() {
        assert_eq!(
            morph_key_value(DatabaseBackend::Postgres, "42"),
            Value::BigInt(Some(42))
        );
        let uuid = "6f1c3d1e-6f53-4b8e-9d55-1d6f1c3d1e6f";
        assert!(matches!(
            morph_key_value(DatabaseBackend::Postgres, uuid),
            Value::Uuid(Some(_))
        ));
        assert_eq!(
            morph_key_value(DatabaseBackend::MySql, uuid),
            Value::String(Some(uuid.to_owned()))
        );
        assert_eq!(
            morph_key_value(DatabaseBackend::Postgres, "01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            Value::String(Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned()))
        );
    }
}
