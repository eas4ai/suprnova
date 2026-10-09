//! Enum cast - `AsEnum<E>` for any `E: FromStr + AsRef<str>`.
//!
//! Stores the variant name (or the user-customised `strum::AsRefStr`
//! string) as a `TEXT` column and parses back via `FromStr`. The two
//! trait bounds together let the cast work cleanly with any enum the
//! user marks with `#[derive(strum::EnumString, strum::AsRefStr)]` -
//! or any enum where the user wrote the impls manually. There is no
//! framework lock-in on `strum`; it's just the most ergonomic way to
//! get the bounds without hand-rolling them.
//!
//! ## Why string storage, not integer
//!
//! Integer-discriminant storage is fragile in two ways:
//!   1. Reordering variants is a silent migration. A `Role::Admin = 0`
//!      that later becomes `Role::Admin = 2` after a re-order would
//!      silently swap which rows are admins.
//!   2. Schema diffs across deployments are noisy. `0` / `1` / `2`
//!      tells you nothing in a DB browser; `"Admin"` is self-describing.
//!
//! Variant name storage is the same convention Laravel uses for its
//! enum casts and is what almost every real-world Eloquent model picks.

use std::marker::PhantomData;
use std::str::FromStr;

use serde::Deserialize;

use super::{Cast, DynCast, IntoDynCast};
use crate::error::FrameworkError;

/// Cast a `FromStr + AsRef<str>` enum ↔ `TEXT`. The enum's variant
/// name (or its `AsRefStr`-customised string) is what hits the column.
pub struct AsEnum<E>(PhantomData<E>);

impl<E> Cast for AsEnum<E>
where
    E: FromStr + AsRef<str> + Send + Sync,
    <E as FromStr>::Err: std::fmt::Display,
{
    type Runtime = E;
    type Storage = String;

    fn to_storage(v: &E) -> Result<String, FrameworkError> {
        Ok(v.as_ref().to_string())
    }

    fn from_storage(s: &String) -> Result<E, FrameworkError> {
        E::from_str(s).map_err(|e| FrameworkError::validation("AsEnum", format!("{e}")))
    }
}

struct AsEnumDyn<E>(PhantomData<E>);

impl<E> DynCast for AsEnumDyn<E>
where
    E: FromStr + AsRef<str> + Send + Sync + 'static,
    <E as FromStr>::Err: std::fmt::Display,
{
    fn from_storage_json(
        &self,
        v: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        // Domain 7 audit D7-A - was `v.as_str().unwrap_or("")` which
        // silently coerced non-strings to the empty string and produced
        // a misleading "AsEnum: '' isn't a valid variant" error instead
        // of "AsEnum: expected JSON string, got <actual>".
        let s = v.as_str().ok_or_else(|| {
            FrameworkError::validation(
                "AsEnum",
                format!("dyn from_storage: expected JSON string, got {v:?}"),
            )
        })?;
        let parsed = AsEnum::<E>::from_storage(&s.to_string())?;
        Ok(serde_json::Value::String(parsed.as_ref().to_string()))
    }

    fn to_storage_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, FrameworkError> {
        Ok(v.clone())
    }
}

impl<E> IntoDynCast for AsEnum<E>
where
    E: FromStr + AsRef<str> + Send + Sync + 'static,
    <E as FromStr>::Err: std::fmt::Display,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsEnumDyn::<E>(PhantomData))
    }
}

/// Store enum lists as a JSON array of each variant's `AsRef<str>` value.
/// You can read Laravel enum collection columns without relying on serde's enum names.
///
/// The storage is `serde_json::Value`, so the column is a native JSON column
/// (`json` or `jsonb` on Postgres). A `String` storage would declare a text
/// column, which Postgres will not read a `json` or `jsonb` value into.
pub struct AsEnumCollection<E>(PhantomData<E>);

impl<E> Cast for AsEnumCollection<E>
where
    E: FromStr + AsRef<str> + Send + Sync,
    E::Err: std::fmt::Display,
{
    type Runtime = Vec<E>;
    type Storage = serde_json::Value;

    fn to_storage(value: &Vec<E>) -> Result<serde_json::Value, FrameworkError> {
        let strings: Vec<&str> = value.iter().map(AsRef::as_ref).collect();
        serde_json::to_value(&strings)
            .map_err(|error| FrameworkError::validation("AsEnumCollection", error.to_string()))
    }

    fn from_storage(stored: &serde_json::Value) -> Result<Vec<E>, FrameworkError> {
        let strings: Vec<String> = Vec::<String>::deserialize(stored)
            .map_err(|error| FrameworkError::validation("AsEnumCollection", error.to_string()))?;
        strings
            .iter()
            .enumerate()
            .map(|(index, value)| {
                AsEnum::<E>::from_storage(value).map_err(|error| {
                    FrameworkError::validation(
                        "AsEnumCollection",
                        format!("element {index}: {error}"),
                    )
                })
            })
            .collect()
    }

    /// A query compares or writes this column as a native JSON parameter, so
    /// Postgres accepts it against a `json` or `jsonb` column. A value that is
    /// not an array of accepted variant strings answers `None`, and the engine
    /// then reports the mismatch.
    fn bind_json(value: &serde_json::Value) -> Option<sea_orm::Value> {
        let accepted = value.as_array()?.iter().all(|element| {
            element
                .as_str()
                .is_some_and(|name| E::from_str(name).is_ok())
        });
        accepted.then(|| sea_orm::Value::Json(Some(Box::new(value.clone()))))
    }
}

struct AsEnumCollectionDyn<E>(PhantomData<E>);

impl<E> DynCast for AsEnumCollectionDyn<E>
where
    E: FromStr + AsRef<str> + serde::Serialize + serde::de::DeserializeOwned + Send + Sync,
    E::Err: std::fmt::Display,
{
    fn from_storage_json(
        &self,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        let runtime = AsEnumCollection::<E>::from_storage(value)?;
        serde_json::to_value(runtime)
            .map_err(|error| FrameworkError::validation("AsEnumCollection", error.to_string()))
    }

    fn to_storage_json(
        &self,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, FrameworkError> {
        let runtime: Vec<E> = serde_json::from_value(value.clone())
            .map_err(|error| FrameworkError::validation("AsEnumCollection", error.to_string()))?;
        AsEnumCollection::<E>::to_storage(&runtime)
    }
}

impl<E> IntoDynCast for AsEnumCollection<E>
where
    E: FromStr
        + AsRef<str>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
    E::Err: std::fmt::Display,
{
    fn into_dyn() -> Box<dyn DynCast> {
        Box::new(AsEnumCollectionDyn::<E>(PhantomData))
    }
}
