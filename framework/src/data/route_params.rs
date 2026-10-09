//! Per-type coercers used by `#[data(from_route_param)]`-generated extract code.
//!
//! Each function converts a raw route-param string into a `serde_json::Value`
//! of the matching JSON kind. The macro picks the right helper based on the
//! field's `syn::Type` at compile time, so no runtime type guessing is needed.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{DeserializeSeed, Deserializer, Error as DeError, Visitor};
use serde_json::{Number, Value};

use crate::FrameworkError;
use crate::data::Field;

fn bad(name: &str, raw: &str, ty: &str) -> FrameworkError {
    FrameworkError::bad_request(format!(
        "route param `{}` = {:?} is not a valid {}",
        name, raw, ty
    ))
}

/// Coerce a route param to `i64` (JSON number).
pub fn parse_i64(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    raw.parse::<i64>()
        .map(|n| Value::Number(n.into()))
        .map_err(|_| bad(name, raw, "i64"))
}

/// Coerce a route param to `u64` (JSON number).
pub fn parse_u64(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    raw.parse::<u64>()
        .map(|n| Value::Number(n.into()))
        .map_err(|_| bad(name, raw, "u64"))
}

/// Coerce a route param to `i32` (JSON number).
pub fn parse_i32(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    raw.parse::<i32>()
        .map(|n| Value::Number(n.into()))
        .map_err(|_| bad(name, raw, "i32"))
}

/// Coerce a route param to `u32` (JSON number).
pub fn parse_u32(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    raw.parse::<u32>()
        .map(|n| Value::Number(n.into()))
        .map_err(|_| bad(name, raw, "u32"))
}

/// Coerce a route param to `i128`.
///
/// A JSON number carries at most 64 bits, so the validated value is
/// returned as its decimal text rather than truncated. The `Data` derive
/// reads it back into the field through [`WideInteger`].
pub fn parse_i128(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    let _ = raw.parse::<i128>().map_err(|_| bad(name, raw, "i128"))?;
    Ok(Value::String(raw.to_string()))
}

/// Coerce a route param to `u128`.
///
/// Like `i128`, returned as its decimal text and read back through
/// [`WideInteger`].
pub fn parse_u128(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    let _ = raw.parse::<u128>().map_err(|_| bad(name, raw, "u128"))?;
    Ok(Value::String(raw.to_string()))
}

/// A field type a 128-bit route parameter can fill: `i128`, `u128`, or an
/// `Option` of either.
///
/// Implemented here only; it exists so [`WideInteger`] can read each of
/// those shapes.
#[doc(hidden)]
pub trait WideIntegerField: Sized {
    /// Read the field from a JSON number or from its decimal text.
    fn deserialize_wide<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// The seed the `Data` derive reads a `#[data(from_route_param)]` field of
/// a 128-bit integer type with.
///
/// [`parse_i128`] and [`parse_u128`] hand the value over as decimal text
/// because a JSON number cannot hold it. A plain `i128` field rejects text,
/// so without this seed every such route parameter, small ones included,
/// failed extraction with a 422.
#[doc(hidden)]
pub struct WideInteger<T>(PhantomData<fn() -> T>);

impl<T> WideInteger<T> {
    /// A seed for one field.
    #[must_use]
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for WideInteger<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de, T: WideIntegerField> DeserializeSeed<'de> for WideInteger<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        T::deserialize_wide(deserializer)
    }
}

/// Reads one 128-bit integer from any integer the format offers or from
/// decimal text.
struct WideIntegerVisitor<T>(PhantomData<fn() -> T>);

impl<'de> Visitor<'de> for WideIntegerVisitor<i128> {
    type Value = i128;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an i128 or its decimal text")
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<i128, E> {
        Ok(i128::from(value))
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<i128, E> {
        Ok(i128::from(value))
    }

    fn visit_i128<E: DeError>(self, value: i128) -> Result<i128, E> {
        Ok(value)
    }

    fn visit_u128<E: DeError>(self, value: u128) -> Result<i128, E> {
        i128::try_from(value).map_err(|_| E::custom("out of range for i128"))
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<i128, E> {
        value.parse().map_err(|_| E::custom("not a valid i128"))
    }
}

impl<'de> Visitor<'de> for WideIntegerVisitor<u128> {
    type Value = u128;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a u128 or its decimal text")
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<u128, E> {
        u128::try_from(value).map_err(|_| E::custom("out of range for u128"))
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<u128, E> {
        Ok(u128::from(value))
    }

    fn visit_i128<E: DeError>(self, value: i128) -> Result<u128, E> {
        u128::try_from(value).map_err(|_| E::custom("out of range for u128"))
    }

    fn visit_u128<E: DeError>(self, value: u128) -> Result<u128, E> {
        Ok(value)
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<u128, E> {
        value.parse().map_err(|_| E::custom("not a valid u128"))
    }
}

impl WideIntegerField for i128 {
    fn deserialize_wide<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(WideIntegerVisitor::<i128>(PhantomData))
    }
}

impl WideIntegerField for u128 {
    fn deserialize_wide<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(WideIntegerVisitor::<u128>(PhantomData))
    }
}

/// Reads `null` as `None` and anything else as the inner integer.
struct OptionalWideIntegerVisitor<T>(PhantomData<fn() -> T>);

impl<'de, T: WideIntegerField> Visitor<'de> for OptionalWideIntegerVisitor<T> {
    type Value = Option<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null, an integer, or its decimal text")
    }

    fn visit_none<E: DeError>(self) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_unit<E: DeError>(self) -> Result<Option<T>, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Option<T>, D::Error> {
        T::deserialize_wide(deserializer).map(Some)
    }
}

impl<T: WideIntegerField> WideIntegerField for Option<T> {
    fn deserialize_wide<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_option(OptionalWideIntegerVisitor::<T>(PhantomData))
    }
}

/// `Field` reads like `Option`: `null` is [`Field::Null`], a value is
/// [`Field::Value`]. An absent key never reaches the seed and stays
/// [`Field::Absent`].
impl<T: WideIntegerField> WideIntegerField for Field<T> {
    fn deserialize_wide<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize_wide(deserializer).map(|value| match value {
            Some(value) => Field::Value(value),
            None => Field::Null,
        })
    }
}

/// Coerce a route param to `f64` (JSON number). Rejects non-finite values.
pub fn parse_f64(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    let n = raw.parse::<f64>().map_err(|_| bad(name, raw, "f64"))?;
    Number::from_f64(n)
        .map(Value::Number)
        .ok_or_else(|| bad(name, raw, "finite f64"))
}

/// Coerce a route param to `f32` (JSON number). Rejects non-finite values.
pub fn parse_f32(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    let n = raw.parse::<f32>().map_err(|_| bad(name, raw, "f32"))?;
    Number::from_f64(n as f64)
        .map(Value::Number)
        .ok_or_else(|| bad(name, raw, "finite f32"))
}

/// Coerce a route param to `bool`. Accepts only `"true"` and `"false"`.
pub fn parse_bool(name: &str, raw: &str) -> Result<Value, FrameworkError> {
    match raw {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => Err(bad(name, raw, "bool")),
    }
}

/// Pass the raw route param through as a JSON string (for `String`, `&str`, UUID, etc.).
pub fn pass_string(_name: &str, raw: &str) -> Result<Value, FrameworkError> {
    Ok(Value::String(raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i64_happy() {
        assert_eq!(parse_i64("id", "42").unwrap(), Value::Number(42.into()));
    }

    #[test]
    fn i64_negative() {
        assert_eq!(
            parse_i64("id", "-7").unwrap(),
            Value::Number((-7i64).into())
        );
    }

    #[test]
    fn i64_rejects_non_numeric() {
        assert!(parse_i64("id", "abc").is_err());
    }

    #[test]
    fn u64_happy() {
        let val = parse_u64("count", "100").unwrap();
        assert_eq!(val, Value::Number(100u64.into()));
    }

    #[test]
    fn u64_rejects_negative() {
        assert!(parse_u64("count", "-1").is_err());
    }

    #[test]
    fn i32_happy() {
        assert_eq!(parse_i32("n", "7").unwrap(), Value::Number(7.into()));
    }

    #[test]
    fn u32_happy() {
        assert_eq!(parse_u32("n", "7").unwrap(), Value::Number(7u32.into()));
    }

    #[test]
    fn f64_happy() {
        let v = parse_f64("ratio", "2.5").unwrap();
        if let Value::Number(n) = v {
            let f = n.as_f64().unwrap();
            assert!((f - 2.5f64).abs() < 1e-9);
        } else {
            panic!("expected number");
        }
    }

    #[test]
    fn f64_rejects_nan() {
        // NaN is not a finite f64 in JSON.
        assert!(parse_f64("ratio", "NaN").is_err());
    }

    #[test]
    fn bool_true() {
        assert_eq!(parse_bool("active", "true").unwrap(), Value::Bool(true));
    }

    #[test]
    fn bool_false() {
        assert_eq!(parse_bool("active", "false").unwrap(), Value::Bool(false));
    }

    #[test]
    fn bool_rejects_unknown() {
        assert!(parse_bool("active", "yes").is_err());
    }

    #[test]
    fn pass_string_keeps_raw() {
        assert_eq!(
            pass_string("slug", "hello-world").unwrap(),
            Value::String("hello-world".into())
        );
    }

    #[test]
    fn i128_returns_string_for_large_value() {
        let large = "170141183460469231731687303715884105727"; // i128::MAX
        let v = parse_i128("n", large).unwrap();
        assert_eq!(v, Value::String(large.to_string()));
    }

    #[test]
    fn i128_rejects_non_numeric() {
        assert!(parse_i128("n", "abc").is_err());
    }
}
