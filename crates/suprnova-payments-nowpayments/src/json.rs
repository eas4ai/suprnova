//! Strict parsing and NOWPayments' JavaScript-compatible IPN representation.

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::fmt;

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(value.into())))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|v| UniqueValue(Value::Number(v)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.into())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, UniqueValue(value))) =
                    map.next_entry::<String, UniqueValue>()?
                {
                    if values.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(crate) fn parse(body: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice::<UniqueValue>(body).map(|value| value.0)
}

/// JSON.stringify enumerates array-index keys numerically even after the
/// provider's recursive Object.keys(...).sort(). Other keys sort by UTF-16.
fn array_index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

pub(crate) fn canonical(value: &Value) -> Result<Vec<u8>, &'static str> {
    let mut output = Vec::new();
    write(value, &mut output)?;
    Ok(output)
}

fn write(value: &Value, output: &mut Vec<u8>) -> Result<(), &'static str> {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_by(|a, b| match (array_index(a), array_index(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.encode_utf16().cmp(b.encode_utf16()),
            });
            output.push(b'{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key).map_err(|_| "invalid JSON string")?;
                output.push(b':');
                write(&object[*key], output)?;
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    output.push(b',');
                }
                write(value, output)?;
            }
            output.push(b']');
        }
        Value::Number(number) => {
            // Avoid accepting a signed JavaScript-rounded integer while
            // retaining a different exact integer in the stored Rust payload.
            const MAX_SAFE: u64 = 9_007_199_254_740_991;
            if number.as_u64().is_some_and(|n| n > MAX_SAFE)
                || number.as_i64().is_some_and(|n| n.unsigned_abs() > MAX_SAFE)
            {
                return Err("IPN integer exceeds JavaScript's exact range; use a string");
            }
            let float = number
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("invalid JSON number")?;
            if float == 0.0 {
                output.push(b'0');
            } else {
                output.extend_from_slice(ryu_js::Buffer::new().format_finite(float).as_bytes());
            }
        }
        other => serde_json::to_writer(&mut *output, other).map_err(|_| "invalid JSON value")?,
    }
    Ok(())
}
