//! Structured casts - `Vec`, `HashMap`-shaped struct, `Collection`,
//! `serde_json::Value`, and `IndexMap<String, T>`.
//!
//! All five casts serialise the runtime value to JSON text and store
//! it in a `TEXT` column. The storage shape is intentionally
//! backend-agnostic: SQLite has no native JSON type so we pick TEXT
//! as the lowest-common-denominator that every backend round-trips
//! cleanly through SeaORM's `Value::String` boundary.
//!
//! Text is all they read and write, so they need a text column. Postgres
//! refuses a text parameter for a `jsonb` or `json` column, and MySQL's
//! driver refuses to read its `JSON` type as text. For a native JSON
//! column use [`AsNativeJson`] (or [`AsOptionalNativeJson`]), which
//! stores the value as JSON. Any of the shapes below fits as its type
//! parameter: a struct, a `Vec<T>`, an `IndexMap<String, T>` or a
//! `serde_json::Value`.
//!
//! ## `AsArrayObject` vs `AsObject`
//!
//! `AsObject<T>` is the right cast when the runtime shape is a fixed
//! struct (e.g. `Prefs { theme: String, ... }`). `AsArrayObject<T>`
//! is the right cast when the runtime shape is an associative map
//! with insertion-order semantics (`IndexMap<String, T>`). The two
//! casts produce equivalent JSON on disk; the choice is about which
//! Rust type the user wants at the field.
//!
//! ## `AsJson<T>`
//!
//! A pass-through cast for any `T: Serialize + DeserializeOwned`.
//! Useful when the field is a `serde_json::Value` or a user-defined
//! struct that's already fully describable in serde terms.
//!
//! ## Nullable columns
//!
//! The five casts above store non-null text. Each has a nullable form
//! for an `Option<_>` field over a nullable column: [`AsOptionalArray`],
//! [`AsOptionalObject`], [`AsOptionalCollection`], [`AsOptionalJson`]
//! and [`AsOptionalArrayObject`]. `None` stores SQL `NULL` and a `NULL`
//! column reads as `None`; a value is stored and read by the sibling.

use std::marker::PhantomData;

use serde::{Serialize, de::DeserializeOwned};

use super::{Cast, DynCast, IntoDynCast};
use crate::error::FrameworkError;

// ---- AsArray<T> -----------------------------------------------------------

/// Cast `Vec<T>` ↔ JSON-encoded `TEXT`. The element type `T` must be
/// `Serialize + DeserializeOwned`.
pub struct AsArray<T>(PhantomData<T>);

impl<T> Cast for AsArray<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Vec<T>;
    type Storage = String;

    fn to_storage(v: &Vec<T>) -> Result<String, FrameworkError> {
        serde_json::to_string(v).map_err(|e| FrameworkError::validation("AsArray", format!("{e}")))
    }

    fn from_storage(s: &String) -> Result<Vec<T>, FrameworkError> {
        serde_json::from_str(s).map_err(|e| FrameworkError::validation("AsArray", format!("{e}")))
    }
}

struct AsArrayDyn<T>(PhantomData<T>);

impl<T> DynCast for AsArrayDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // The TEXT column round-trips as a JSON string - `v.as_str()`
        // gives us the encoded payload; parse it back into `Vec<T>`
        // then re-emit as a JSON array so downstream deserialization
        // into the user model's `Vec<T>` field succeeds.
        //
        // Domain 7 audit D7-A - was `v.as_str().unwrap_or("[]")` which
        // silently treated non-string storage as an empty array. Now
        // strict so a misconfigured column produces an explicit type
        // mismatch rather than a silent empty Vec<T>.
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsArray",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed: Vec<T> = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsArray", format!("dyn parse: {e}")))?;
        serde_json::to_value(parsed)
            .map_err(|e| FrameworkError::internal(format!("AsArray: re-serialize failed: {e}")))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(serde_json::Value::String(v.to_string()))
    }
}

impl<T> IntoDynCast for AsArray<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsArrayDyn::<T>(PhantomData))
    }
}

// ---- AsObject<T> ----------------------------------------------------------

/// Cast a `Serialize + DeserializeOwned` struct ↔ JSON-encoded `TEXT`.
/// Use for fixed-shape associative data where the keys are statically
/// known (e.g. a `Prefs { theme, notifications }` config struct). Use
/// [`AsArrayObject`] when the runtime shape is a dynamic map.
pub struct AsObject<T>(PhantomData<T>);

impl<T> Cast for AsObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = T;
    type Storage = String;

    fn to_storage(v: &T) -> Result<String, FrameworkError> {
        serde_json::to_string(v).map_err(|e| FrameworkError::validation("AsObject", format!("{e}")))
    }

    fn from_storage(s: &String) -> Result<T, FrameworkError> {
        serde_json::from_str(s).map_err(|e| FrameworkError::validation("AsObject", format!("{e}")))
    }
}

struct AsObjectDyn<T>(PhantomData<T>);

impl<T> DynCast for AsObjectDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - strict-validate input shape.
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsObject",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed: T = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsObject", format!("dyn parse: {e}")))?;
        serde_json::to_value(parsed)
            .map_err(|e| FrameworkError::internal(format!("AsObject: re-serialize failed: {e}")))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(serde_json::Value::String(v.to_string()))
    }
}

impl<T> IntoDynCast for AsObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsObjectDyn::<T>(PhantomData))
    }
}

// ---- AsCollection<T> ------------------------------------------------------

/// Cast `Collection<T>` ↔ JSON-encoded `TEXT`. Stores as a JSON array
/// of `T` and decodes back into the [`Collection`] wrapper. The
/// runtime type is the framework's [`Collection<T>`] (a thin
/// `Vec<T>` newtype) so the user gets slice-style indexing /
/// iteration plus the Eloquent-style methods Phase 10C adds.
///
/// [`Collection`]: crate::eloquent::Collection
/// [`Collection<T>`]: crate::eloquent::Collection
pub struct AsCollection<T>(PhantomData<T>);

impl<T> Cast for AsCollection<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + Clone,
{
    type Runtime = crate::eloquent::Collection<T>;
    type Storage = String;

    fn to_storage(v: &crate::eloquent::Collection<T>) -> Result<String, FrameworkError> {
        serde_json::to_string(v.as_ref())
            .map_err(|e| FrameworkError::validation("AsCollection", format!("{e}")))
    }

    fn from_storage(s: &String) -> Result<crate::eloquent::Collection<T>, FrameworkError> {
        let v: Vec<T> = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsCollection", format!("{e}")))?;
        Ok(crate::eloquent::Collection::from(v))
    }
}

struct AsCollectionDyn<T>(PhantomData<T>);

impl<T> DynCast for AsCollectionDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - strict-validate input shape.
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsCollection",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed: Vec<T> = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsCollection", format!("dyn parse: {e}")))?;
        serde_json::to_value(parsed).map_err(|e| {
            FrameworkError::internal(format!("AsCollection: re-serialize failed: {e}"))
        })
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(serde_json::Value::String(v.to_string()))
    }
}

impl<T> IntoDynCast for AsCollection<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsCollectionDyn::<T>(PhantomData))
    }
}

// ---- AsJson<T> ------------------------------------------------------------

/// Cast any `Serialize + DeserializeOwned` type ↔ JSON-encoded `TEXT`.
/// Pass-through both directions - the cast exists so the storage shape
/// is uniform (TEXT) across backends. Use when the field is a
/// `serde_json::Value` or a user-defined struct that's already
/// serde-describable.
pub struct AsJson<T>(PhantomData<T>);

impl<T> Cast for AsJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = T;
    type Storage = String;

    fn to_storage(v: &T) -> Result<String, FrameworkError> {
        serde_json::to_string(v).map_err(|e| FrameworkError::validation("AsJson", format!("{e}")))
    }

    fn from_storage(s: &String) -> Result<T, FrameworkError> {
        serde_json::from_str(s).map_err(|e| FrameworkError::validation("AsJson", format!("{e}")))
    }
}

struct AsJsonDyn<T>(PhantomData<T>);

impl<T> DynCast for AsJsonDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - strict-validate input shape.
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsJson",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed: T = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsJson", format!("dyn parse: {e}")))?;
        serde_json::to_value(parsed)
            .map_err(|e| FrameworkError::internal(format!("AsJson: re-serialize failed: {e}")))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(serde_json::Value::String(v.to_string()))
    }
}

impl<T> IntoDynCast for AsJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsJsonDyn::<T>(PhantomData))
    }
}

// ---- AsNativeJson<T> ------------------------------------------------------

/// Cast any `Serialize + DeserializeOwned` type ↔ a native JSON column:
/// `jsonb` or `json` on Postgres, `JSON` on MySQL and MariaDB, text on
/// SQLite.
///
/// The other structured casts store text, which Postgres refuses to bind
/// to a `jsonb` or `json` column and MySQL's driver refuses to read from
/// a `JSON` column. This one stores the value as JSON, so the column can
/// be native and the database's JSON operators and indexes work on it.
/// Queries bind through it too: an `update_all` or a `filter` on the
/// column sends a JSON parameter.
pub struct AsNativeJson<T>(PhantomData<T>);

impl<T> Cast for AsNativeJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = T;
    type Storage = serde_json::Value;

    fn to_storage(v: &T) -> Result<serde_json::Value, FrameworkError> {
        serde_json::to_value(v)
            .map_err(|e| FrameworkError::validation("AsNativeJson", format!("{e}")))
    }

    fn from_storage(s: &serde_json::Value) -> Result<T, FrameworkError> {
        T::deserialize(s).map_err(|e| FrameworkError::validation("AsNativeJson", format!("{e}")))
    }

    fn bind_json(value: &serde_json::Value) -> Option<sea_orm::Value> {
        native_json_bind(value)
    }
}

/// A value compared with or written to a native JSON column, as a JSON
/// parameter. `null` is left to the caller, which writes SQL `NULL`.
fn native_json_bind(value: &serde_json::Value) -> Option<sea_orm::Value> {
    (!value.is_null()).then(|| sea_orm::Value::Json(Some(Box::new(value.clone()))))
}

/// The erased form of [`AsNativeJson`]. The stored value is the JSON
/// itself, not text, so it reads it as `T` directly.
struct AsNativeJsonDyn<T>(PhantomData<T>);

impl<T> DynCast for AsNativeJsonDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        let parsed = AsNativeJson::<T>::from_storage(v)?;
        serde_json::to_value(parsed).map_err(|e| {
            FrameworkError::internal(format!("AsNativeJson: re-serialize failed: {e}"))
        })
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl<T> IntoDynCast for AsNativeJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsNativeJsonDyn::<T>(PhantomData))
    }
}

/// The nullable form of [`AsNativeJson`]: `Option<T>` ↔ a nullable native
/// JSON column. `None` is SQL `NULL`, not the JSON `null`.
pub struct AsOptionalNativeJson<T>(PhantomData<T>);

impl<T> Cast for AsOptionalNativeJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Option<T>;
    type Storage = Option<serde_json::Value>;

    fn to_storage(v: &Option<T>) -> Result<Option<serde_json::Value>, FrameworkError> {
        v.as_ref().map(AsNativeJson::<T>::to_storage).transpose()
    }

    fn from_storage(s: &Option<serde_json::Value>) -> Result<Option<T>, FrameworkError> {
        s.as_ref().map(AsNativeJson::<T>::from_storage).transpose()
    }

    fn bind_json(value: &serde_json::Value) -> Option<sea_orm::Value> {
        native_json_bind(value)
    }
}

impl<T> IntoDynCast for AsOptionalNativeJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsNativeJson::<T>::into_dyn()))
    }
}

// ---- AsArrayObject<T> -----------------------------------------------------

/// Cast `IndexMap<String, T>` ↔ JSON-encoded `TEXT`. Use when the
/// runtime shape is a dynamic-key map and the order of keys is
/// significant (e.g. a UI ordering of labels). For fixed-shape
/// records, use [`AsObject`].
///
/// `IndexMap` over `HashMap` is intentional: serde's JSON
/// serialisation preserves insertion order through `IndexMap` (Rust's
/// `HashMap` randomises bucket order), and the framework's
/// `serde_json` is already configured with `preserve_order` for the
/// same reason.
pub struct AsArrayObject<T>(PhantomData<T>);

impl<T> Cast for AsArrayObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = indexmap::IndexMap<String, T>;
    type Storage = String;

    fn to_storage(v: &indexmap::IndexMap<String, T>) -> Result<String, FrameworkError> {
        serde_json::to_string(v)
            .map_err(|e| FrameworkError::validation("AsArrayObject", format!("{e}")))
    }

    fn from_storage(s: &String) -> Result<indexmap::IndexMap<String, T>, FrameworkError> {
        serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsArrayObject", format!("{e}")))
    }
}

struct AsArrayObjectDyn<T>(PhantomData<T>);

impl<T> DynCast for AsArrayObjectDyn<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - strict-validate input shape.
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsArrayObject",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed: indexmap::IndexMap<String, T> = serde_json::from_str(s)
            .map_err(|e| FrameworkError::validation("AsArrayObject", format!("dyn parse: {e}")))?;
        serde_json::to_value(parsed).map_err(|e| {
            FrameworkError::internal(format!("AsArrayObject: re-serialize failed: {e}"))
        })
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(serde_json::Value::String(v.to_string()))
    }
}

impl<T> IntoDynCast for AsArrayObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsArrayObjectDyn::<T>(PhantomData))
    }
}

// ---- Nullable siblings ----------------------------------------------------
//
// Each cast above stores a non-null `String`, so it cannot serve a nullable
// column: `AsJson<Option<T>>` writes the text `null`, and a `NULL` column
// fails to decode. The `AsOptional*` forms store `Option<String>`, which
// makes the column nullable, map `None` to SQL `NULL` and back, and hand
// every value to the sibling so a value is stored and read exactly as the
// sibling does it, errors included.

/// `None` stores `NULL`; `Some` stores what the sibling cast `C` stores.
fn optional_to_storage<C>(value: &Option<C::Runtime>) -> Result<Option<String>, FrameworkError>
where
    C: Cast<Storage = String>,
{
    value.as_ref().map(C::to_storage).transpose()
}

/// `NULL` reads as `None`; text reads as the sibling cast `C` reads it.
fn optional_from_storage<C>(stored: &Option<String>) -> Result<Option<C::Runtime>, FrameworkError>
where
    C: Cast<Storage = String>,
{
    stored.as_ref().map(C::from_storage).transpose()
}

/// The erased form of the `AsOptional*` casts: `null` stays `null` in
/// both directions, and any other value goes through the sibling's erased
/// cast unchanged.
struct OptionalStructuredDyn(Box<dyn DynCast>);

impl DynCast for OptionalStructuredDyn {
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        match v {
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            other => self.0.from_storage_json(other),
        }
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        match v {
            serde_json::Value::Null => Ok(serde_json::Value::Null),
            other => self.0.to_storage_json(other),
        }
    }
}

/// The nullable form of [`AsArray`]: `Option<Vec<T>>` ↔ a nullable
/// JSON-encoded `TEXT` column. `None` is SQL `NULL`.
pub struct AsOptionalArray<T>(PhantomData<T>);

impl<T> Cast for AsOptionalArray<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Option<Vec<T>>;
    type Storage = Option<String>;

    fn to_storage(v: &Option<Vec<T>>) -> Result<Option<String>, FrameworkError> {
        optional_to_storage::<AsArray<T>>(v)
    }

    fn from_storage(s: &Option<String>) -> Result<Option<Vec<T>>, FrameworkError> {
        optional_from_storage::<AsArray<T>>(s)
    }
}

impl<T> IntoDynCast for AsOptionalArray<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsArray::<T>::into_dyn()))
    }
}

/// The nullable form of [`AsObject`]: `Option<T>` ↔ a nullable
/// JSON-encoded `TEXT` column. `None` is SQL `NULL`.
pub struct AsOptionalObject<T>(PhantomData<T>);

impl<T> Cast for AsOptionalObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Option<T>;
    type Storage = Option<String>;

    fn to_storage(v: &Option<T>) -> Result<Option<String>, FrameworkError> {
        optional_to_storage::<AsObject<T>>(v)
    }

    fn from_storage(s: &Option<String>) -> Result<Option<T>, FrameworkError> {
        optional_from_storage::<AsObject<T>>(s)
    }
}

impl<T> IntoDynCast for AsOptionalObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsObject::<T>::into_dyn()))
    }
}

/// The nullable form of [`AsCollection`]: an optional [`Collection<T>`]
/// ↔ a nullable JSON-encoded `TEXT` column. `None` is SQL `NULL`.
///
/// [`Collection<T>`]: crate::eloquent::Collection
pub struct AsOptionalCollection<T>(PhantomData<T>);

impl<T> Cast for AsOptionalCollection<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + Clone,
{
    type Runtime = Option<crate::eloquent::Collection<T>>;
    type Storage = Option<String>;

    fn to_storage(
        v: &Option<crate::eloquent::Collection<T>>,
    ) -> Result<Option<String>, FrameworkError> {
        optional_to_storage::<AsCollection<T>>(v)
    }

    fn from_storage(
        s: &Option<String>,
    ) -> Result<Option<crate::eloquent::Collection<T>>, FrameworkError> {
        optional_from_storage::<AsCollection<T>>(s)
    }
}

impl<T> IntoDynCast for AsOptionalCollection<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsCollection::<T>::into_dyn()))
    }
}

/// The nullable form of [`AsJson`]: `Option<T>` ↔ a nullable
/// JSON-encoded `TEXT` column. `None` is SQL `NULL`, where
/// `AsJson<Option<T>>` would store the text `null`.
pub struct AsOptionalJson<T>(PhantomData<T>);

impl<T> Cast for AsOptionalJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Option<T>;
    type Storage = Option<String>;

    fn to_storage(v: &Option<T>) -> Result<Option<String>, FrameworkError> {
        optional_to_storage::<AsJson<T>>(v)
    }

    fn from_storage(s: &Option<String>) -> Result<Option<T>, FrameworkError> {
        optional_from_storage::<AsJson<T>>(s)
    }
}

impl<T> IntoDynCast for AsOptionalJson<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsJson::<T>::into_dyn()))
    }
}

/// The nullable form of [`AsArrayObject`]: `Option<IndexMap<String, T>>`
/// ↔ a nullable JSON-encoded `TEXT` column. `None` is SQL `NULL`.
pub struct AsOptionalArrayObject<T>(PhantomData<T>);

impl<T> Cast for AsOptionalArrayObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Runtime = Option<indexmap::IndexMap<String, T>>;
    type Storage = Option<String>;

    fn to_storage(
        v: &Option<indexmap::IndexMap<String, T>>,
    ) -> Result<Option<String>, FrameworkError> {
        optional_to_storage::<AsArrayObject<T>>(v)
    }

    fn from_storage(
        s: &Option<String>,
    ) -> Result<Option<indexmap::IndexMap<String, T>>, FrameworkError> {
        optional_from_storage::<AsArrayObject<T>>(s)
    }
}

impl<T> IntoDynCast for AsOptionalArrayObject<T>
where
    T: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(OptionalStructuredDyn(AsArrayObject::<T>::into_dyn()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AsArray, AsArrayObject, AsCollection, AsJson, AsNativeJson, AsObject, AsOptionalArray,
        AsOptionalArrayObject, AsOptionalCollection, AsOptionalJson, AsOptionalNativeJson,
        AsOptionalObject, Cast, IntoDynCast,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Prefs {
        theme: String,
    }

    /// A cast name, its optional erased cast, the non-optional sibling's
    /// erased cast, and a stored JSON text the pair accepts.
    type Twin = (
        &'static str,
        Box<dyn super::DynCast>,
        Box<dyn super::DynCast>,
        &'static str,
    );

    /// Each optional twin next to its non-optional sibling.
    fn twins() -> Vec<Twin> {
        vec![
            (
                "AsOptionalArray",
                AsOptionalArray::<String>::into_dyn(),
                AsArray::<String>::into_dyn(),
                r#"["a","b"]"#,
            ),
            (
                "AsOptionalObject",
                AsOptionalObject::<Prefs>::into_dyn(),
                AsObject::<Prefs>::into_dyn(),
                r#"{"theme":"dark"}"#,
            ),
            (
                "AsOptionalCollection",
                AsOptionalCollection::<String>::into_dyn(),
                AsCollection::<String>::into_dyn(),
                r#"["a","b"]"#,
            ),
            (
                "AsOptionalJson",
                AsOptionalJson::<serde_json::Value>::into_dyn(),
                AsJson::<serde_json::Value>::into_dyn(),
                r#"{"count":42}"#,
            ),
            (
                "AsOptionalArrayObject",
                AsOptionalArrayObject::<String>::into_dyn(),
                AsArrayObject::<String>::into_dyn(),
                r#"{"z":"last","a":"first"}"#,
            ),
        ]
    }

    #[test]
    fn the_optional_dyn_casts_keep_null_as_null_in_both_directions() {
        for (name, optional, _, _) in twins() {
            assert_eq!(
                optional.from_storage_json(&json!(null)).unwrap(),
                json!(null),
                "{name} reads NULL as null"
            );
            assert_eq!(
                optional.to_storage_json(&json!(null)).unwrap(),
                json!(null),
                "{name} writes null as NULL, not as the text `null`"
            );
        }
    }

    #[test]
    fn the_optional_dyn_casts_match_their_sibling_for_non_null_values() {
        for (name, optional, plain, stored) in twins() {
            let stored = json!(stored);
            let read = optional.from_storage_json(&stored).unwrap();
            assert_eq!(
                read,
                plain.from_storage_json(&stored).unwrap(),
                "{name} reads like its sibling"
            );
            assert_eq!(
                optional.to_storage_json(&read).unwrap(),
                plain.to_storage_json(&read).unwrap(),
                "{name} writes like its sibling"
            );
            for bad in [json!(1), json!("{not json")] {
                assert_eq!(
                    optional.from_storage_json(&bad).unwrap_err().to_string(),
                    plain.from_storage_json(&bad).unwrap_err().to_string(),
                    "{name} fails like its sibling on {bad}"
                );
            }
        }
    }

    #[test]
    fn the_optional_casts_store_none_as_null_and_delegate_some() {
        assert_eq!(
            AsOptionalJson::<serde_json::Value>::to_storage(&None).unwrap(),
            None
        );
        assert_eq!(
            AsOptionalJson::<serde_json::Value>::from_storage(&None).unwrap(),
            None
        );
        let value = json!({ "count": 42 });
        let stored = AsOptionalJson::<serde_json::Value>::to_storage(&Some(value.clone()))
            .unwrap()
            .expect("Some stores text");
        assert_eq!(
            stored,
            AsJson::<serde_json::Value>::to_storage(&value).unwrap()
        );
        assert_eq!(
            AsOptionalJson::<serde_json::Value>::from_storage(&Some(stored)).unwrap(),
            Some(value)
        );
        assert_eq!(
            AsOptionalJson::<serde_json::Value>::from_storage(&Some("{not json".into()))
                .unwrap_err()
                .to_string(),
            AsJson::<serde_json::Value>::from_storage(&"{not json".into())
                .unwrap_err()
                .to_string(),
        );

        assert_eq!(AsOptionalArray::<String>::to_storage(&None).unwrap(), None);
        assert_eq!(
            AsOptionalObject::<Prefs>::from_storage(&None).unwrap(),
            None
        );
        assert!(
            AsOptionalCollection::<String>::from_storage(&None)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            AsOptionalArrayObject::<String>::to_storage(&None).unwrap(),
            None
        );
    }

    /// The native JSON cast stores the value as JSON, not as text, and
    /// binds a JSON parameter, which a Postgres `jsonb` column needs.
    #[test]
    fn the_native_json_cast_stores_and_binds_json() {
        let prefs = Prefs {
            theme: "dark".into(),
        };
        let stored = AsNativeJson::<Prefs>::to_storage(&prefs).unwrap();
        assert_eq!(stored, json!({ "theme": "dark" }));
        assert_eq!(AsNativeJson::<Prefs>::from_storage(&stored).unwrap(), prefs);
        assert!(AsNativeJson::<Prefs>::from_storage(&json!([1, 2])).is_err());
        assert_eq!(
            <AsNativeJson<Prefs> as Cast>::bind_json(&json!({ "theme": "dark" })),
            Some(sea_orm::Value::Json(Some(Box::new(
                json!({ "theme": "dark" })
            ))))
        );
        assert_eq!(
            <AsNativeJson<Prefs> as Cast>::bind_json(&serde_json::Value::Null),
            None,
            "null is left to the caller, which writes SQL NULL"
        );

        let erased = AsNativeJson::<Prefs>::into_dyn();
        assert_eq!(
            erased
                .from_storage_json(&json!({ "theme": "dark" }))
                .unwrap(),
            json!({ "theme": "dark" })
        );
        assert!(erased.from_storage_json(&json!("not prefs")).is_err());
        assert_eq!(
            erased.to_storage_json(&json!({ "theme": "dark" })).unwrap(),
            json!({ "theme": "dark" })
        );
    }

    #[test]
    fn the_optional_native_json_cast_stores_none_as_sql_null() {
        assert_eq!(
            AsOptionalNativeJson::<Vec<String>>::to_storage(&None).unwrap(),
            None
        );
        assert_eq!(
            AsOptionalNativeJson::<Vec<String>>::to_storage(&Some(vec!["a".into()])).unwrap(),
            Some(json!(["a"]))
        );
        assert_eq!(
            AsOptionalNativeJson::<Vec<String>>::from_storage(&None).unwrap(),
            None
        );
        let erased = AsOptionalNativeJson::<Vec<String>>::into_dyn();
        assert_eq!(
            erased.from_storage_json(&serde_json::Value::Null).unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            erased.from_storage_json(&json!(["a"])).unwrap(),
            json!(["a"])
        );
    }
}
