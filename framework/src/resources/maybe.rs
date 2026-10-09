//! Conditional attribute values: omit a field from the rendered
//! `attributes` object based on a runtime flag.
//!
//! This is Suprnova's analogue of Laravel's `MissingValue` / `whenLoaded`
//! / `when` / `unless`. The renderer recognises `Maybe<T>` via its
//! `Serialize` impl: a `Maybe::Missing` produces a namespaced sentinel;
//! the attribute pass then removes the sentinel while preserving explicit nulls.
//!
//! In practice: a hand-rolled resource impl that wants conditional fields
//! either calls `Maybe::present(v)` / `Maybe::missing()` and inserts the
//! result via `insert_maybe(map, "key", maybe)` (provided below), or
//! returns `Maybe<T>` from a field on a `#[derive(Data)]` struct - the
//! generated `resource_attributes` runs every field through the same
//! `Maybe`-aware insert path so a `Missing` value never reaches the
//! envelope.
//!
//! # Examples
//!
//! ```rust,no_run
//! use suprnova::resources::Maybe;
//!
//! # struct User { email: String, phone: String, show_phone: bool }
//! # impl User {
//! // Manual: hand-rolling `IntoJsonResource::resource_attributes`.
//! fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
//!     use suprnova::resources::insert_maybe;
//!     let mut map = serde_json::Map::new();
//!     insert_maybe(&mut map, "email", Maybe::present(&self.email));
//!     insert_maybe(
//!         &mut map,
//!         "phone",
//!         if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
//!     );
//!     serde_json::Value::Object(map)
//! }
//! # }
//! ```

use serde::ser::{Serialize, SerializeStruct, Serializer};
use serde_json::{Map, Value};

/// A conditional attribute value: either present (serialized as the
/// wrapped value) or missing (the field is omitted from the rendered
/// JSON:API `attributes` object).
///
/// Laravel parity: this is `MissingValue` + the `when()` / `whenLoaded()`
/// / `unless()` family from `Illuminate\Http\Resources\ConditionallyLoadsAttributes`,
/// reified as a single sum type with constructors.
#[derive(Debug, Clone, Default)]
pub enum Maybe<T> {
    /// Field is present; the wrapped value is serialized as the attribute value.
    Present(T),
    /// Field is missing - omitted from the rendered `attributes` object.
    #[default]
    Missing,
}

/// Laravel-shape alias for [`Maybe`]. Use whichever name fits your
/// codebase - they are the same type.
pub type MissingValue<T> = Maybe<T>;

impl<T> Maybe<T> {
    /// Construct a present value.
    pub fn present(v: T) -> Self {
        Maybe::Present(v)
    }

    /// Construct a missing value.
    pub fn missing() -> Self {
        Maybe::Missing
    }

    /// Conditional constructor. `when(true, fn)` → present; `when(false, _)` → missing.
    /// Mirrors Laravel's `$this->when($condition, $value)`.
    pub fn when(condition: bool, value: T) -> Self {
        if condition {
            Maybe::Present(value)
        } else {
            Maybe::Missing
        }
    }

    /// Laravel-shape alias: `unless(true, _)` → missing; `unless(false, v)` → present.
    pub fn unless(condition: bool, value: T) -> Self {
        Self::when(!condition, value)
    }

    /// Lazy form: only computes `f()` when present.
    pub fn when_with(condition: bool, f: impl FnOnce() -> T) -> Self {
        if condition {
            Maybe::Present(f())
        } else {
            Maybe::Missing
        }
    }

    /// True if this attribute should be omitted from the envelope.
    pub fn is_missing(&self) -> bool {
        matches!(self, Maybe::Missing)
    }

    /// True if this attribute carries a value.
    pub fn is_present(&self) -> bool {
        matches!(self, Maybe::Present(_))
    }

    /// Map the wrapped value through `f`. Missing values stay missing.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Maybe<U> {
        match self {
            Maybe::Present(v) => Maybe::Present(f(v)),
            Maybe::Missing => Maybe::Missing,
        }
    }

    /// Extract the inner value, returning `None` for missing.
    pub fn into_option(self) -> Option<T> {
        match self {
            Maybe::Present(v) => Some(v),
            Maybe::Missing => None,
        }
    }

    /// Borrow the inner value, returning `None` for missing.
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Maybe::Present(v) => Some(v),
            Maybe::Missing => None,
        }
    }
}

impl<T> From<Option<T>> for Maybe<T> {
    fn from(o: Option<T>) -> Self {
        match o {
            Some(v) => Maybe::Present(v),
            None => Maybe::Missing,
        }
    }
}

impl<T> From<Maybe<T>> for Option<T> {
    fn from(m: Maybe<T>) -> Self {
        m.into_option()
    }
}

impl<'de, T: serde::de::DeserializeOwned> serde::Deserialize<'de> for Maybe<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if is_missing_sentinel(&value) {
            Ok(Self::Missing)
        } else {
            serde_json::from_value(value)
                .map(Self::Present)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// A conditional field group that lets you emit several attributes together.
/// The resource renderer merges its object fields into the surrounding attributes.
#[derive(Debug, Clone)]
pub struct MergeValue<T> {
    fields: Option<T>,
}

/// Group fields under one condition so a resource can include or omit them together.
pub fn merge_when<T>(condition: bool, fields: T) -> MergeValue<T> {
    MergeValue {
        fields: condition.then_some(fields),
    }
}

const MERGE_TAG: &str = "__suprnova_merge_fields__";

impl<T: Serialize> Serialize for MergeValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Some(fields) = &self.fields else {
            return Maybe::<T>::Missing.serialize(serializer);
        };
        let fields = serde_json::to_value(fields).map_err(serde::ser::Error::custom)?;
        if !fields.is_object() {
            return Err(serde::ser::Error::custom(
                "Resource merge fields must be an object",
            ));
        }
        let mut group = serializer.serialize_struct(MERGE_TAG, 1)?;
        group.serialize_field(MERGE_TAG, &fields)?;
        group.end()
    }
}

impl<'de, T: serde::de::DeserializeOwned> serde::Deserialize<'de> for MergeValue<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Value::deserialize(deserializer)?;
        if is_missing_sentinel(&fields) {
            return Ok(Self { fields: None });
        }
        if let Some(group) = fields
            .as_object_mut()
            .filter(|group| group.len() == 1)
            .and_then(|group| group.remove(MERGE_TAG))
        {
            fields = group;
        }
        if !fields.is_object() {
            return Err(serde::de::Error::custom(
                "Resource merge fields must be an object",
            ));
        }
        let fields = serde_json::from_value(fields).map_err(serde::de::Error::custom)?;
        Ok(Self {
            fields: Some(fields),
        })
    }
}

/// Read a loaded existence flag so an unloaded relation omits the resource field.
/// The closure receives the flag, including a loaded `false`, and runs only when loaded.
pub fn when_exists_loaded<M, T>(
    model: &M,
    relation: &str,
    value: impl FnOnce(bool) -> T,
) -> Maybe<T>
where
    M: crate::eloquent::Model + From<<M::Entity as sea_orm::EntityTrait>::Model>,
    <M::Entity as sea_orm::EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as sea_orm::EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<M::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    model
        .__eager_cache()
        .and_then(|cache| cache.get_exists(relation))
        .map(value)
        .into()
}

/// Sentinel object inserted into a `serde_json::Value` to signal "omit
/// this key during the attributes-rendering pass". The wire field key
/// itself is this long, namespaced constant - `serde_json` discards the
/// struct name passed to `serialize_struct`, so the collision-resistance
/// has to live in the field key, not the struct name. A short key like
/// `__missing__` is plausible real user data; nobody serializes
/// `{"__suprnova_maybe_missing__": true}` by accident, so a `Maybe::Missing`
/// can never be confused with genuine attribute data at any nesting depth.
const SUPRNOVA_MAYBE_MISSING_TAG: &str = "__suprnova_maybe_missing__";

impl<T: Serialize> Serialize for Maybe<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Maybe::Present(v) => v.serialize(serializer),
            Maybe::Missing => {
                // Emit a one-field object that `strip_missing_values`
                // recognises and removes. Using a tagged object - not
                // `null` - lets us distinguish "user explicitly stored
                // null" from "value was omitted by design". The single
                // field is keyed on the long namespaced constant so the
                // sentinel can't collide with real user data on the wire.
                let mut st = serializer.serialize_struct(SUPRNOVA_MAYBE_MISSING_TAG, 1)?;
                st.serialize_field(SUPRNOVA_MAYBE_MISSING_TAG, &true)?;
                st.end()
            }
        }
    }
}

/// True if `v` is the sentinel object emitted by `Maybe::Missing`'s
/// `Serialize` impl. Matches on the long namespaced key so genuine user
/// data - even data shaped like `{"__missing__": true}` - is never
/// mistaken for the sentinel.
pub(crate) fn is_missing_sentinel(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.len() == 1 && obj.get(SUPRNOVA_MAYBE_MISSING_TAG).and_then(Value::as_bool) == Some(true)
}

/// Drop every key whose value is a `Maybe::Missing` sentinel, recursing
/// into nested objects and arrays. Called by the resource attributes
/// renderer after serializing each field.
pub fn strip_missing_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let original = std::mem::take(map);
            for (key, mut field) in original {
                strip_missing_values(&mut field);
                if is_missing_sentinel(&field) {
                    continue;
                }
                if let Some(group) = field
                    .as_object()
                    .filter(|group| group.len() == 1)
                    .and_then(|group| group.get(MERGE_TAG))
                    .and_then(Value::as_object)
                {
                    map.extend(group.clone());
                } else {
                    map.insert(key, field);
                }
            }
        }
        Value::Array(items) => {
            items.retain(|item| !is_missing_sentinel(item));
            for item in items {
                strip_missing_values(item);
            }
        }
        _ => {}
    }
}

/// Insert `value` under `key` only if it is `Maybe::Present(v)`. Used
/// by hand-rolled `resource_attributes` implementations as the Laravel
/// `$this->when()`-shape mutator.
pub fn insert_maybe<T: Serialize>(
    map: &mut Map<String, Value>,
    key: impl Into<String>,
    value: Maybe<T>,
) {
    if let Maybe::Present(v) = value {
        match serde_json::to_value(&v) {
            Ok(serialized) => {
                map.insert(key.into(), serialized);
            }
            Err(_) => {
                // Same fail-soft posture as the macro-generated path:
                // if a downstream Serialize impl returns Err, omit the
                // attribute rather than panicking the whole envelope.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_serializes_to_inner() {
        let m: Maybe<i32> = Maybe::present(7);
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v, serde_json::json!(7));
    }

    #[test]
    fn missing_serializes_to_sentinel() {
        let m: Maybe<i32> = Maybe::missing();
        let v = serde_json::to_value(&m).unwrap();
        assert!(is_missing_sentinel(&v));
    }

    #[test]
    fn when_true_is_present() {
        assert!(Maybe::when(true, 1).is_present());
        assert!(Maybe::when(false, 1).is_missing());
    }

    #[test]
    fn unless_inverts_when() {
        assert!(Maybe::unless(false, 1).is_present());
        assert!(Maybe::unless(true, 1).is_missing());
    }

    #[test]
    fn when_with_is_lazy() {
        let mut counter = 0;
        let _: Maybe<i32> = Maybe::when_with(false, || {
            counter += 1;
            1
        });
        assert_eq!(counter, 0, "closure must not run when condition is false");
    }

    #[test]
    fn map_preserves_missing() {
        let m: Maybe<i32> = Maybe::missing();
        let m2 = m.map(|v| v * 2);
        assert!(m2.is_missing());
    }

    #[test]
    fn strip_removes_missing_keys() {
        // Build the sentinel via the actual Serialize impl so the test
        // tracks the wire shape rather than hardcoding it.
        let sentinel = || serde_json::to_value(Maybe::<i32>::missing()).unwrap();
        let mut v = serde_json::json!({
            "kept": 1,
            "dropped": sentinel(),
            "nested": { "kept2": 2, "dropped2": sentinel() }
        });
        strip_missing_values(&mut v);
        assert_eq!(v["kept"], 1);
        assert!(v.get("dropped").is_none());
        assert_eq!(v["nested"]["kept2"], 2);
        assert!(v["nested"].get("dropped2").is_none());
    }

    #[test]
    fn strip_removes_missing_in_arrays() {
        let sentinel = || serde_json::to_value(Maybe::<i32>::missing()).unwrap();
        let mut v = serde_json::json!([sentinel(), { "real": 1 }, sentinel()]);
        strip_missing_values(&mut v);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["real"], 1);
    }

    #[test]
    fn user_data_shaped_like_old_sentinel_survives() {
        // Real attribute data equal to the old short, collidable sentinel
        // shape `{"__missing__": true}` must NOT be treated as a missing
        // marker - at the top level, nested in an object, and inside an
        // array. Only a genuine `Maybe::Missing` is stripped.
        let collidable = || serde_json::json!({ "__missing__": true });
        let real_missing = || serde_json::to_value(Maybe::<i32>::missing()).unwrap();

        let mut v = serde_json::json!({
            "looks_missing": collidable(),
            "nested": { "also_looks_missing": collidable() },
            "list": [collidable(), { "ok": 1 }],
            "really_missing": real_missing(),
        });
        strip_missing_values(&mut v);

        // The user data that merely looks like the old sentinel survives.
        assert_eq!(v["looks_missing"], collidable());
        assert_eq!(v["nested"]["also_looks_missing"], collidable());
        assert_eq!(v["list"].as_array().unwrap().len(), 2);
        assert_eq!(v["list"][0], collidable());
        assert_eq!(v["list"][1]["ok"], 1);

        // The genuine `Maybe::Missing` is still stripped.
        assert!(v.get("really_missing").is_none());

        // And the collidable shape is not classified as the sentinel.
        assert!(!is_missing_sentinel(&collidable()));
        assert!(is_missing_sentinel(&real_missing()));
    }

    #[test]
    fn from_option_round_trip() {
        let m: Maybe<i32> = Some(5).into();
        assert!(m.is_present());
        let m: Maybe<i32> = None::<i32>.into();
        assert!(m.is_missing());
        let o: Option<i32> = Maybe::present(3).into();
        assert_eq!(o, Some(3));
        let o: Option<i32> = Maybe::<i32>::missing().into();
        assert_eq!(o, None);
    }

    #[test]
    fn insert_maybe_skips_missing() {
        let mut m = Map::new();
        insert_maybe(&mut m, "a", Maybe::present(1));
        insert_maybe(&mut m, "b", Maybe::<i32>::missing());
        assert_eq!(m.get("a"), Some(&serde_json::json!(1)));
        assert!(m.get("b").is_none());
    }

    #[test]
    fn missing_value_alias_is_same_type() {
        // Demonstrate the Laravel-shape alias.
        let m: MissingValue<&str> = Maybe::present("hi");
        assert!(m.is_present());
    }

    /// MEM-006: dropping a missing attribute keeps the others in order.
    #[test]
    fn mem_audit_dropping_a_missing_attribute_keeps_the_order() {
        let sentinel = serde_json::to_value(Maybe::<i32>::missing()).unwrap();
        let mut value = serde_json::json!({"a": sentinel, "b": 1, "c": 2});
        strip_missing_values(&mut value);
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["b", "c"]);
    }
}
