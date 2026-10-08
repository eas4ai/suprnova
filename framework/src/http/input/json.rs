//! Naming the fields of a JSON body that kept it from deserializing.
//!
//! This runs only after `serde_json` refused a body, so a body it accepts
//! is read by `serde_json` alone and what a JSON body accepts does not
//! change. The body is read again with every deserializer `serde_json`
//! hands out wrapped, so each value knows its input name (`address.street`,
//! `items.1`) and the type it is read as. A value that does not fit its
//! type is recorded and a placeholder stands in, so the read goes on to the
//! next field. `null` where a value is required counts as missing, as
//! Laravel's `required` reads it.

use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, IgnoredAny, MapAccess, SeqAccess, Visitor,
};

use super::form::FormValue;
use super::{Collector, FieldError, Stop, join, record_missing_fields, struct_field_names};
use crate::error::ValidationErrors;
use crate::http::upload::FieldFailure;

/// The fields of the JSON `bytes` that failed to deserialize as `T`, each
/// under its input name; `None` when `T` is not a struct, when `bytes` is
/// not a JSON object, or when no field is to blame.
pub(crate) fn json_field_failures<T: DeserializeOwned>(bytes: &[u8]) -> Option<ValidationErrors> {
    let fields = struct_field_names::<T>()?;
    // A body that is not one whole JSON object is malformed, not a form
    // with bad fields, and the names it does carry are what a missing
    // field is told apart by.
    let present = present_fields(bytes, fields)?;
    let collector = Collector::default();
    let mut body = serde_json::Deserializer::from_slice(bytes);
    let read = T::deserialize(Tracked {
        inner: &mut body,
        path: String::new(),
        collector: &collector,
    });
    if read.is_ok() && !collector.has_failures() {
        return None;
    }
    record_missing_fields::<T>(&collector, fields, present);
    collector.has_failures().then(|| collector.into_errors())
}

/// Which of `fields` the top-level object of `bytes` names; `None` when
/// `bytes` is not exactly one JSON object.
fn present_fields(bytes: &[u8], fields: &'static [&'static str]) -> Option<Vec<&'static str>> {
    let mut body = serde_json::Deserializer::from_slice(bytes);
    let present = de::Deserializer::deserialize_map(&mut body, NamesIn { fields }).ok()?;
    body.end().ok()?;
    Some(present)
}

/// Collects the names of an object that are among `fields`, skipping
/// every value.
struct NamesIn {
    fields: &'static [&'static str],
}

impl<'de> Visitor<'de> for NamesIn {
    type Value = Vec<&'static str>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut present = Vec::new();
        while let Some(field) = map.next_key_seed(FieldName {
            fields: self.fields,
        })? {
            map.next_value::<IgnoredAny>()?;
            if let Some(field) = field
                && !present.contains(&field)
            {
                present.push(field);
            }
        }
        Ok(present)
    }
}

/// An object's name, as the field among `fields` it is, if any.
#[derive(Clone, Copy)]
struct FieldName {
    fields: &'static [&'static str],
}

impl<'de> DeserializeSeed<'de> for FieldName {
    type Value = Option<&'static str>;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for FieldName {
    type Value = Option<&'static str>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a field name")
    }

    fn visit_str<E: de::Error>(self, name: &str) -> Result<Self::Value, E> {
        Ok(self.fields.iter().copied().find(|field| *field == name))
    }
}

/// A `serde_json` deserializer for the value at `path`.
struct Tracked<'c, D> {
    inner: D,
    path: String,
    collector: &'c Collector,
}

/// A deserializer method that hands `serde_json` a [`Pass`] visitor.
macro_rules! through_pass {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            let Tracked { inner, path, collector } = self;
            inner
                .$method($($arg,)* Pass { visitor, path: &path, collector })
                .map_err(|error| collector.take(error))
        }
    )*};
}

/// A deserializer method for a primitive, read through a [`Leaf`].
macro_rules! leaf {
    ($($method:ident => $want:ident;)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let Tracked { inner, path, collector } = self;
            inner
                .deserialize_any(Leaf { want: Want::$want, visitor, path: &path, collector })
                .map_err(|error| collector.take(error))
        }
    )*};
}

impl<'de, D: de::Deserializer<'de>> de::Deserializer<'de> for Tracked<'_, D> {
    type Error = FieldError;

    through_pass! {
        deserialize_any();
        deserialize_bytes();
        deserialize_byte_buf();
        deserialize_option();
        deserialize_unit();
        deserialize_unit_struct(name: &'static str);
        deserialize_newtype_struct(name: &'static str);
        deserialize_seq();
        deserialize_tuple(len: usize);
        deserialize_tuple_struct(name: &'static str, len: usize);
        deserialize_map();
        deserialize_struct(name: &'static str, fields: &'static [&'static str]);
        deserialize_enum(name: &'static str, variants: &'static [&'static str]);
        deserialize_identifier();
        deserialize_ignored_any();
    }

    leaf! {
        deserialize_bool => Bool;
        deserialize_i8 => I8;
        deserialize_i16 => I16;
        deserialize_i32 => I32;
        deserialize_i64 => I64;
        deserialize_i128 => I128;
        deserialize_u8 => U8;
        deserialize_u16 => U16;
        deserialize_u32 => U32;
        deserialize_u64 => U64;
        deserialize_u128 => U128;
        deserialize_f32 => F32;
        deserialize_f64 => F64;
        deserialize_char => Char;
        deserialize_str => Str;
        deserialize_string => Str;
    }
}

/// A visitor that hands every value to `visitor` unchanged, wrapping what
/// it reads further, a list's elements or an object's values, so they
/// know their own input names.
struct Pass<'p, 'c, V> {
    visitor: V,
    path: &'p str,
    collector: &'c Collector,
}

impl<V> Pass<'_, '_, V> {
    /// File a failure the visitor raised for this value under its path, and
    /// hand it back across `serde_json`.
    fn settle<T, E: de::Error>(&self, read: Result<T, E>) -> Result<T, E> {
        if self.path.is_empty() {
            return read;
        }
        read.map_err(|_| {
            self.collector.record(self.path, FieldFailure::Format);
            self.collector.carry(FieldError::recorded())
        })
    }

    /// Hand back across `serde_json` an error a wrapped read returned.
    fn carry<T, E: de::Error>(&self, read: Result<T, FieldError>) -> Result<T, E> {
        read.map_err(|error| {
            let error = match error.0 {
                Stop::Missing(_) => self.collector.settle_missing(self.path, error),
                // The body's own object failing is no field's failure.
                Stop::Other(_) if self.path.is_empty() => error,
                Stop::Other(_) => {
                    self.collector.record(self.path, FieldFailure::Format);
                    FieldError::recorded()
                }
                Stop::Recorded => error,
            };
            self.collector.carry(error)
        })
    }
}

/// Visitor methods that pass a scalar through.
macro_rules! pass_scalar {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method<E: de::Error>(self, $($arg: $ty),*) -> Result<Self::Value, E> {
            let read = self.visitor.$method($($arg),*);
            Pass { visitor: (), path: self.path, collector: self.collector }.settle(read)
        }
    )*};
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Pass<'_, '_, V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.visitor.expecting(f)
    }

    pass_scalar! {
        visit_bool(v: bool);
        visit_i8(v: i8);
        visit_i16(v: i16);
        visit_i32(v: i32);
        visit_i64(v: i64);
        visit_i128(v: i128);
        visit_u8(v: u8);
        visit_u16(v: u16);
        visit_u32(v: u32);
        visit_u64(v: u64);
        visit_u128(v: u128);
        visit_f32(v: f32);
        visit_f64(v: f64);
        visit_char(v: char);
        visit_str(v: &str);
        visit_borrowed_str(v: &'de str);
        visit_string(v: String);
        visit_bytes(v: &[u8]);
        visit_borrowed_bytes(v: &'de [u8]);
        visit_byte_buf(v: Vec<u8>);
        visit_none();
        visit_unit();
    }

    fn visit_some<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        let (path, collector) = (self.path, self.collector);
        let read = self.visitor.visit_some(Tracked {
            inner: deserializer,
            path: path.to_string(),
            collector,
        });
        Pass {
            visitor: (),
            path,
            collector,
        }
        .carry(read)
    }

    fn visit_newtype_struct<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        let (path, collector) = (self.path, self.collector);
        let read = self.visitor.visit_newtype_struct(Tracked {
            inner: deserializer,
            path: path.to_string(),
            collector,
        });
        Pass {
            visitor: (),
            path,
            collector,
        }
        .carry(read)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        let (path, collector) = (self.path, self.collector);
        let read = self.visitor.visit_seq(TrackedSeq {
            inner: seq,
            path,
            index: 0,
            collector,
        });
        Pass {
            visitor: (),
            path,
            collector,
        }
        .carry(read)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let (path, collector) = (self.path, self.collector);
        let read = self.visitor.visit_map(TrackedMap {
            inner: map,
            path,
            name: None,
            collector,
        });
        Pass {
            visitor: (),
            path,
            collector,
        }
        .carry(read)
    }

    /// An enum's content is read by `serde_json` alone: a variant that does
    /// not read fails the enum's own field.
    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let read = self.visitor.visit_enum(data);
        Pass {
            visitor: (),
            path: self.path,
            collector: self.collector,
        }
        .settle(read)
    }
}

/// The elements of a JSON array, each under `path.index`.
struct TrackedSeq<'p, 'c, A> {
    inner: A,
    path: &'p str,
    index: usize,
    collector: &'c Collector,
}

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for TrackedSeq<'_, '_, A> {
    type Error = FieldError;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        let path = join(self.path, self.index);
        self.index += 1;
        self.inner
            .next_element_seed(Within {
                seed,
                path,
                collector: self.collector,
            })
            .map_err(|error| self.collector.take(error))
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

/// The entries of a JSON object, each value under `path.name`.
struct TrackedMap<'p, 'c, A> {
    inner: A,
    path: &'p str,
    /// The name of the entry whose value is next.
    name: Option<String>,
    collector: &'c Collector,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for TrackedMap<'_, '_, A> {
    type Error = FieldError;

    /// The name is read whole, then handed to the struct the way a form's
    /// name is, so a map with number keys reads its keys as `serde_json`
    /// reads them.
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let name = match self.inner.next_key::<String>() {
            Ok(Some(name)) => name,
            Ok(None) => return Ok(None),
            Err(error) => return Err(self.collector.take(error)),
        };
        let key = seed.deserialize(FormValue::key(&name, self.collector));
        self.name = Some(name);
        key.map(Some)
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        let path = join(self.path, self.name.as_deref().unwrap_or_default());
        self.inner
            .next_value_seed(Within {
                seed,
                path,
                collector: self.collector,
            })
            .map_err(|error| self.collector.take(error))
    }

    fn size_hint(&self) -> Option<usize> {
        self.inner.size_hint()
    }
}

/// A seed whose value is read through a [`Tracked`] deserializer at `path`.
struct Within<'c, S> {
    seed: S,
    path: String,
    collector: &'c Collector,
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Within<'_, S> {
    type Value = S::Value;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        let collector = self.collector;
        self.seed
            .deserialize(Tracked {
                inner: deserializer,
                path: self.path,
                collector,
            })
            .map_err(|error| collector.carry(error))
    }
}

/// The primitive a field is read as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    Str,
}

impl Want {
    /// The key a value of another kind is reported with, as the multipart
    /// extractor reports text that does not parse as the field's type.
    fn failure(self) -> FieldFailure {
        match self {
            Self::Bool => FieldFailure::Boolean,
            Self::F32 | Self::F64 => FieldFailure::Numeric,
            Self::Char => FieldFailure::Format,
            Self::Str => FieldFailure::String,
            _ => FieldFailure::Integer,
        }
    }

    fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// Whether the integer `n` is a value of this primitive.
    fn holds(self, n: i128) -> bool {
        match self {
            Self::I8 => i8::try_from(n).is_ok(),
            Self::I16 => i16::try_from(n).is_ok(),
            Self::I32 => i32::try_from(n).is_ok(),
            Self::I64 => i64::try_from(n).is_ok(),
            Self::I128 => true,
            Self::U8 => u8::try_from(n).is_ok(),
            Self::U16 => u16::try_from(n).is_ok(),
            Self::U32 => u32::try_from(n).is_ok(),
            Self::U64 => u64::try_from(n).is_ok(),
            Self::U128 => u128::try_from(n).is_ok(),
            Self::F32 | Self::F64 => true,
            Self::Bool | Self::Char | Self::Str => false,
        }
    }
}

/// The member `serde_json` puts a number in when its `arbitrary_precision`
/// feature is on: the number then arrives as an object holding its digits.
const ARBITRARY_PRECISION_NUMBER: &str = "$serde_json::private::Number";

/// A visitor for one primitive. `serde_json` hands it whatever the value
/// is; a value of the wanted kind goes to `visitor`, and any other is
/// recorded and `want`'s placeholder goes to `visitor` instead.
struct Leaf<'p, 'c, V> {
    want: Want,
    visitor: V,
    path: &'p str,
    collector: &'c Collector,
}

impl<'de, V: Visitor<'de>> Leaf<'_, '_, V> {
    /// Record `failure` and hand the visitor a placeholder of the wanted
    /// kind.
    fn stand_in<E: de::Error>(self, failure: FieldFailure) -> Result<V::Value, E> {
        self.collector.record(self.path, failure);
        let Leaf {
            want,
            visitor,
            path,
            collector,
        } = self;
        let read = match want {
            Want::Bool => visitor.visit_bool(false),
            Want::I8 => visitor.visit_i8(0),
            Want::I16 => visitor.visit_i16(0),
            Want::I32 => visitor.visit_i32(0),
            Want::I64 => visitor.visit_i64(0),
            Want::I128 => visitor.visit_i128(0),
            Want::U8 => visitor.visit_u8(0),
            Want::U16 => visitor.visit_u16(0),
            Want::U32 => visitor.visit_u32(0),
            Want::U64 => visitor.visit_u64(0),
            Want::U128 => visitor.visit_u128(0),
            Want::F32 => visitor.visit_f32(0.0),
            Want::F64 => visitor.visit_f64(0.0),
            Want::Char => visitor.visit_char(' '),
            Want::Str => visitor.visit_str(""),
        };
        Leaf {
            want,
            visitor: (),
            path,
            collector,
        }
        .settle_unit(read)
    }

    fn mismatch<E: de::Error>(self) -> Result<V::Value, E> {
        let failure = self.want.failure();
        self.stand_in(failure)
    }

    fn integer<E: de::Error>(self, n: i128) -> Result<V::Value, E> {
        if !self.want.holds(n) {
            return self.mismatch();
        }
        // In range by `holds`, so each conversion keeps the value.
        let read = match self.want {
            Want::I8 => self.visitor.visit_i8(n as i8),
            Want::I16 => self.visitor.visit_i16(n as i16),
            Want::I32 => self.visitor.visit_i32(n as i32),
            Want::I64 => self.visitor.visit_i64(n as i64),
            Want::I128 => self.visitor.visit_i128(n),
            Want::U8 => self.visitor.visit_u8(n as u8),
            Want::U16 => self.visitor.visit_u16(n as u16),
            Want::U32 => self.visitor.visit_u32(n as u32),
            Want::U64 => self.visitor.visit_u64(n as u64),
            Want::U128 => self.visitor.visit_u128(n as u128),
            Want::F32 => self.visitor.visit_f32(n as f32),
            _ => self.visitor.visit_f64(n as f64),
        };
        Leaf {
            want: self.want,
            visitor: (),
            path: self.path,
            collector: self.collector,
        }
        .settle_unit(read)
    }

    fn float<E: de::Error>(self, x: f64) -> Result<V::Value, E> {
        let read = match self.want {
            Want::F32 => self.visitor.visit_f32(x as f32),
            Want::F64 => self.visitor.visit_f64(x),
            _ => return self.mismatch(),
        };
        Leaf {
            want: self.want,
            visitor: (),
            path: self.path,
            collector: self.collector,
        }
        .settle_unit(read)
    }
}

impl Leaf<'_, '_, ()> {
    /// Hand `read`, the visitor's answer, back across `serde_json` once
    /// the visitor is spent: a failure the visitor raised is this field's.
    fn settle_unit<T, E: de::Error>(&self, read: Result<T, E>) -> Result<T, E> {
        read.map_err(|_| {
            self.collector.record(self.path, self.want.failure());
            self.collector.carry(FieldError::recorded())
        })
    }
}

impl<'de, V: Visitor<'de>> Visitor<'de> for Leaf<'_, '_, V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.visitor.expecting(f)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        if self.want != Want::Bool {
            return self.mismatch();
        }
        let read = self.visitor.visit_bool(v);
        Leaf {
            want: self.want,
            visitor: (),
            path: self.path,
            collector: self.collector,
        }
        .settle_unit(read)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        self.integer(i128::from(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        self.integer(i128::from(v))
    }

    fn visit_i128<E: de::Error>(self, v: i128) -> Result<Self::Value, E> {
        self.integer(v)
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> Result<Self::Value, E> {
        match i128::try_from(v) {
            Ok(n) => self.integer(n),
            Err(_) if self.want == Want::U128 => {
                let read = self.visitor.visit_u128(v);
                Leaf {
                    want: self.want,
                    visitor: (),
                    path: self.path,
                    collector: self.collector,
                }
                .settle_unit(read)
            }
            Err(_) if self.want.is_float() => self.float(v as f64),
            Err(_) => self.mismatch(),
        }
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        self.float(v)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        let read = match self.want {
            Want::Str => self.visitor.visit_str(v),
            Want::Char => {
                let mut chars = v.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => self.visitor.visit_char(c),
                    _ => return self.mismatch(),
                }
            }
            _ => return self.mismatch(),
        };
        Leaf {
            want: self.want,
            visitor: (),
            path: self.path,
            collector: self.collector,
        }
        .settle_unit(read)
    }

    /// `null` where a value is required: the field is missing.
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.stand_in(FieldFailure::Required)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.stand_in(FieldFailure::Required)
    }

    fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<Self::Value, E> {
        self.mismatch()
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        self.mismatch()
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let Some(name) = map.next_key::<String>()? else {
            return self.mismatch();
        };
        if name == ARBITRARY_PRECISION_NUMBER {
            let digits: String = map.next_value()?;
            if let Ok(n) = digits.parse::<i128>() {
                return self.integer(n);
            }
            if let Ok(x) = digits.parse::<f64>() {
                return self.float(x);
            }
            return self.mismatch();
        }
        map.next_value::<IgnoredAny>()?;
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        self.mismatch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Order {
        id: u8,
        price: f32,
        code: char,
        lines: Vec<Line>,
        state: State,
        note: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct Line {
        sku: String,
        qty: u16,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    enum State {
        Open,
        Closed,
    }

    fn failures(body: &str) -> Vec<(String, String)> {
        assert!(
            serde_json::from_slice::<Order>(body.as_bytes()).is_err(),
            "`{body}` is meant to fail"
        );
        let errors = json_field_failures::<Order>(body.as_bytes())
            .unwrap_or_else(|| panic!("`{body}`: no field to blame"));
        let mut found: Vec<(String, String)> = errors
            .errors
            .iter()
            .map(|(field, messages)| (field.clone(), messages[0].key.to_string()))
            .collect();
        found.sort();
        found
    }

    fn pair(field: &str, key: &str) -> (String, String) {
        (field.to_string(), format!("validation-{key}"))
    }

    #[test]
    fn a_body_that_reads_names_no_field() {
        let body = r#"{"id": 1, "price": 2, "code": "x", "state": "Open",
                       "lines": [{"sku": "a", "qty": 2}]}"#;
        let order: Order = serde_json::from_str(body).expect("an order");
        assert_eq!((order.id, order.code, order.state), (1, 'x', State::Open));
        assert!(order.price > 1.0 && order.note.is_none());
        assert_eq!((order.lines[0].sku.as_str(), order.lines[0].qty), ("a", 2));
        assert!(json_field_failures::<Order>(body.as_bytes()).is_none());
    }

    #[test]
    fn every_value_of_the_wrong_kind_is_named_at_its_depth() {
        assert_eq!(
            failures(
                r#"{"id": 300, "price": true, "code": "xy", "note": [1],
                    "lines": [{"sku": 5, "qty": -1}, {"sku": "a", "qty": 2}],
                    "state": "Shipped"}"#
            ),
            [
                pair("code", "format"),
                pair("id", "integer"),
                pair("lines.0.qty", "integer"),
                pair("lines.0.sku", "string"),
                pair("note", "string"),
                pair("price", "numeric"),
                pair("state", "format"),
            ]
        );
    }

    #[test]
    fn a_missing_field_is_named_at_its_depth_and_null_is_missing() {
        assert_eq!(
            failures(r#"{"id": null, "lines": [{"qty": 1}], "state": "Open"}"#),
            [
                pair("code", "required"),
                pair("id", "required"),
                pair("lines.0.sku", "required"),
                pair("price", "required"),
            ]
        );
    }

    #[test]
    fn a_body_that_is_not_one_object_is_no_fields_failure() {
        for body in [r#"{"id": "#, r#"[1, 2]"#, r#"{"id": 1} {"#, "nope"] {
            assert!(
                json_field_failures::<Order>(body.as_bytes()).is_none(),
                "`{body}`"
            );
        }
    }
}
