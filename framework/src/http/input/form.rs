//! The url-encoded reader: a form body or a query string into a typed value.
//!
//! It reads the pairs as Laravel's request holds them once PHP has parsed
//! them and `ConvertEmptyStringsToNull` has run. A bracketed or indexed name
//! is nested data, read as [`super::nested`] describes: `user[name]` is the
//! member `name` of `user`, and `tags[]` and `photos[0]` are elements of a
//! list, in index order. An empty value is `null`: the name is still there,
//! holding `null`, so a map or a `serde_json::Value` sees a cleared field
//! apart from one never sent, an `Option` reads `None`, and a field that
//! cannot hold `null` is missing. A name sent more than once keeps its last
//! value. A value is read as `serde_urlencoded` reads one, through the field
//! type's `FromStr`, except a `bool`, which reads what forms send, and a
//! value that does not parse is recorded under its input name, joined with
//! dots (`user.name`, `tags.1`), with the key for the field's type, as the
//! multipart extractor files it.

use std::borrow::Cow;
use std::str::FromStr;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, Visitor};

use super::multipart::{FormFile, FormNotUtf8, UPLOADED_FILE, offer_placeholder};
use super::nested::{Array, Key, Nested, Node};
use super::placeholder::Placeholder;
use super::{
    Collector, FieldError, InputError, Stop, join, record_missing_fields, struct_field_names,
};
use crate::http::upload::{FieldFailure, parse_form_bool};

/// Read url-encoded `bytes` into `T`, failing field by field.
pub(crate) fn parse_form_input<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InputError> {
    let fields = struct_field_names::<T>();
    read_nested(Nested::from_urlencoded(bytes, fields), fields)
}

/// Read nested form data into `T`, failing field by field. `fields` are
/// the names `form` was read for, `T`'s field names.
pub(super) fn read_nested<T: DeserializeOwned>(
    form: Nested<'_>,
    fields: Option<&'static [&'static str]>,
) -> Result<T, InputError> {
    let present = fields.map(|fields| form.present(fields));
    let collector = Collector::default();
    let read = T::deserialize(FormInput {
        form,
        collector: &collector,
    });
    if let Some(error) = collector.take_failed() {
        return Err(InputError::Failed(error));
    }
    let error = match read {
        Ok(value) if !collector.has_failures() => return Ok(value),
        Ok(_) => None,
        Err(error) => Some(error),
    };
    if let (Some(fields), Some(present)) = (fields, present) {
        record_missing_fields::<T>(&collector, fields, present);
    }
    if collector.has_failures() {
        return Err(InputError::Fields(collector.into_errors()));
    }
    Err(InputError::Other(error.map_or_else(
        || "the input did not read".to_string(),
        |error| error.to_string(),
    )))
}

/// The top-level deserializer of a form read.
struct FormInput<'a, 'c> {
    form: Nested<'a>,
    collector: &'c Collector,
}

impl<'a, 'c> FormInput<'a, 'c> {
    fn entries(self) -> Entries<'a, 'c> {
        Entries {
            names: self.form.names.into_iter(),
            untracked: self.form.untracked,
            collector: self.collector,
            pending: None,
        }
    }
}

impl<'de> de::Deserializer<'de> for FormInput<'_, '_> {
    type Error = FieldError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let collector = self.collector;
        visitor
            .visit_map(self.entries())
            .map_err(|error| collector.settle_missing("", error))
    }

    /// A list of `(name, value)` pairs, as `serde_urlencoded` reads one.
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_seq(Pairs(self.entries()))
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.entries().next_entry() {
            None => visitor.visit_unit(),
            Some(_) => Err(de::Error::invalid_type(de::Unexpected::Map, &visitor)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit_struct tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

/// The top-level entries of a read, each name once, in the order it was
/// first sent, then the first name the read does not use.
struct Entries<'a, 'c> {
    names: indexmap::map::IntoIter<Cow<'a, str>, Node<'a>>,
    untracked: Option<Cow<'a, str>>,
    collector: &'c Collector,
    /// The entry whose key was read and whose value is next.
    pending: Option<(Cow<'a, str>, Node<'a>)>,
}

impl<'a> Entries<'a, '_> {
    fn next_entry(&mut self) -> Option<(Cow<'a, str>, Node<'a>)> {
        // A name the read does not use is only ever ignored or refused by
        // its name, so its value is never read.
        self.names.next().or_else(|| {
            self.untracked
                .take()
                .map(|name| (name, Node::Text(Cow::Borrowed(""))))
        })
    }
}

impl<'de> de::MapAccess<'de> for Entries<'_, '_> {
    type Error = FieldError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((name, node)) = self.next_entry() else {
            return Ok(None);
        };
        let key = seed.deserialize(FormValue::key(&name, self.collector));
        self.pending = Some((name, node));
        key.map(Some)
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        let Some((name, node)) = self.pending.take() else {
            return Err(de::Error::custom("a value was read before its name"));
        };
        read_node(seed, node, &name, self.collector)
    }
}

/// Read `node`, the value at the input name `path`, with `seed`.
fn read_node<'de, S: DeserializeSeed<'de>>(
    seed: S,
    node: Node<'_>,
    path: &str,
    collector: &Collector,
) -> Result<S::Value, FieldError> {
    match node {
        Node::Text(text) if text.is_empty() => seed.deserialize(FormNull { path, collector }),
        Node::Text(text) => seed.deserialize(FormValue {
            text: &text,
            path: Some(path),
            collector,
        }),
        Node::NotUtf8 => seed.deserialize(FormNotUtf8 { path, collector }),
        Node::File(file) => seed.deserialize(FormFile {
            file: *file,
            path,
            collector,
        }),
        Node::Array(array) => seed.deserialize(FormArray {
            array: *array,
            path,
            collector,
        }),
    }
}

/// The entries of a read as `(name, value)` pairs.
struct Pairs<'a, 'c>(Entries<'a, 'c>);

impl<'de> de::SeqAccess<'de> for Pairs<'_, '_> {
    type Error = FieldError;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        let Some((name, node)) = self.0.next_entry() else {
            return Ok(None);
        };
        self.0.pending = Some((name, node));
        seed.deserialize(Pair {
            entries: &mut self.0,
            read: 0,
        })
        .map(Some)
    }
}

/// One `(name, value)` pair: the pending entry of `entries`.
struct Pair<'e, 'a, 'c> {
    entries: &'e mut Entries<'a, 'c>,
    /// How many of the pair's two elements were read.
    read: u8,
}

impl<'de> de::Deserializer<'de> for Pair<'_, '_, '_> {
    type Error = FieldError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_seq(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de> de::SeqAccess<'de> for Pair<'_, '_, '_> {
    type Error = FieldError;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        self.read += 1;
        match self.read {
            1 => {
                let Some((name, _)) = self.entries.pending.as_ref() else {
                    return Ok(None);
                };
                seed.deserialize(FormValue::key(name, self.entries.collector))
                    .map(Some)
            }
            2 => de::MapAccess::next_value_seed(self.entries, seed).map(Some),
            _ => Ok(None),
        }
    }
}

/// One url-encoded value, read as `serde_urlencoded` reads one.
///
/// A field's value files its failures under `path`. A name has no path,
/// and its failure is no field's: it stops the read, as a name
/// `serde_urlencoded` cannot read stops it.
#[derive(Clone, Copy)]
pub(super) struct FormValue<'v, 'c> {
    text: &'v str,
    path: Option<&'v str>,
    collector: &'c Collector,
}

impl<'v, 'c> FormValue<'v, 'c> {
    /// A name read as a map key or a struct's field name.
    pub(super) fn key(text: &'v str, collector: &'c Collector) -> Self {
        Self {
            text,
            path: None,
            collector,
        }
    }

    /// File a failure the visitor raised for this value under its path.
    fn settle<T>(
        &self,
        failure: FieldFailure,
        read: Result<T, FieldError>,
    ) -> Result<T, FieldError> {
        match (read, self.path) {
            (Err(FieldError(Stop::Other(_))), Some(path)) => {
                self.collector.record(path, failure);
                Err(FieldError::recorded())
            }
            (read, _) => read,
        }
    }

    /// Record that this value cannot be read as asked, when it is a
    /// field's; `false` for a name, which has no field to record against.
    fn record(&self, failure: FieldFailure) -> bool {
        match self.path {
            Some(path) => {
                self.collector.record(path, failure);
                true
            }
            None => false,
        }
    }
}

/// A primitive read through its `FromStr`: one that does not parse is
/// recorded, and its zero stands in.
macro_rules! parsed {
    ($($method:ident: $ty:ty => $visit:ident($zero:expr), $failure:ident;)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match <$ty>::from_str(self.text) {
                Ok(value) => self.settle(FieldFailure::$failure, visitor.$visit(value)),
                Err(error) if !self.record(FieldFailure::$failure) => Err(de::Error::custom(error)),
                Err(_) => self.settle(FieldFailure::$failure, visitor.$visit($zero)),
            }
        }
    )*};
}

/// A shape one text value cannot have: a field's is recorded and a
/// placeholder stands in; a name's is read as text, which the visitor
/// refuses.
macro_rules! shapeless {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            if self.record(FieldFailure::Format) {
                de::Deserializer::$method(Placeholder, $($arg,)* visitor)
            } else {
                de::Deserializer::deserialize_any(self, visitor)
            }
        }
    )*};
}

impl<'de> de::Deserializer<'de> for FormValue<'_, '_> {
    type Error = FieldError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.settle(FieldFailure::Format, visitor.visit_str(self.text))
    }

    /// A field's `bool` reads what forms send: `1` and `0`, `true` and
    /// `false`, `on` and `off`, as the multipart extractor reads one. A name
    /// is read as `serde_urlencoded` reads it, `true` or `false`.
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let parsed = match self.path {
            Some(_) => parse_form_bool(self.text),
            None => self.text.parse().ok(),
        };
        match parsed {
            Some(value) => self.settle(FieldFailure::Boolean, visitor.visit_bool(value)),
            None if !self.record(FieldFailure::Boolean) => Err(de::Error::invalid_value(
                de::Unexpected::Str(self.text),
                &"`true` or `false`",
            )),
            None => self.settle(FieldFailure::Boolean, visitor.visit_bool(false)),
        }
    }

    parsed! {
        deserialize_i8: i8 => visit_i8(0), Integer;
        deserialize_i16: i16 => visit_i16(0), Integer;
        deserialize_i32: i32 => visit_i32(0), Integer;
        deserialize_i64: i64 => visit_i64(0), Integer;
        deserialize_i128: i128 => visit_i128(0), Integer;
        deserialize_u8: u8 => visit_u8(0), Integer;
        deserialize_u16: u16 => visit_u16(0), Integer;
        deserialize_u32: u32 => visit_u32(0), Integer;
        deserialize_u64: u64 => visit_u64(0), Integer;
        deserialize_u128: u128 => visit_u128(0), Integer;
        deserialize_f32: f32 => visit_f32(0.0), Numeric;
        deserialize_f64: f64 => visit_f64(0.0), Numeric;
        deserialize_char: char => visit_char(' '), Format;
    }

    shapeless! {
        deserialize_seq();
        deserialize_tuple(len: usize);
        deserialize_tuple_struct(name: &'static str, len: usize);
        deserialize_map();
        deserialize_struct(name: &'static str, fields: &'static [&'static str]);
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    /// Text where an `UploadedFile` belongs is a field's failure
    /// (`validation-file`); a name is never a file.
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name != UPLOADED_FILE {
            return visitor.visit_newtype_struct(self);
        }
        if self.record(FieldFailure::File) {
            offer_placeholder(visitor)
        } else {
            Err(de::Error::custom("a name is not a file"))
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.settle(
            FieldFailure::Format,
            visitor.visit_enum(UnitVariant(self.text)),
        )
    }

    serde::forward_to_deserialize_any! {
        str string bytes byte_buf unit unit_struct identifier ignored_any
    }
}

/// An enum read from a text value: the text names a variant that holds
/// nothing, as `serde_urlencoded` reads an enum.
struct UnitVariant<'v>(&'v str);

/// The variant [`UnitVariant`] names, which holds nothing.
struct UnitOnly;

impl<'de> de::EnumAccess<'de> for UnitVariant<'_> {
    type Error = FieldError;
    type Variant = UnitOnly;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self::Variant), Self::Error> {
        let variant =
            seed.deserialize(IntoDeserializer::<FieldError>::into_deserializer(self.0))?;
        Ok((variant, UnitOnly))
    }
}

impl<'de> de::VariantAccess<'de> for UnitOnly {
    type Error = FieldError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, _: S) -> Result<S::Value, Self::Error> {
        Err(de::Error::custom("expected unit variant"))
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("expected unit variant"))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("expected unit variant"))
    }
}

/// A value sent empty, which Laravel reads as `null`.
///
/// A type that can hold `null` gets it: an `Option` is `None`, and a
/// `serde_json::Value` or a map's value is `null`. A type that cannot is a
/// missing value, recorded as `validation-required` under `path`, and a
/// placeholder stands in.
struct FormNull<'v, 'c> {
    path: &'v str,
    collector: &'c Collector,
}

/// A type that cannot hold `null`: its field is missing.
macro_rules! required {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            self.collector.record(self.path, FieldFailure::Required);
            de::Deserializer::$method(Placeholder, $($arg,)* visitor)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for FormNull<'_, '_> {
    type Error = FieldError;

    /// A type read from whatever arrives, such as a `serde_json::Value`,
    /// gets `null`; one that refuses it is missing.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor
            .visit_unit()
            .map_err(|error: FieldError| match error.0 {
                Stop::Other(_) => {
                    self.collector.record(self.path, FieldFailure::Required);
                    FieldError::recorded()
                }
                _ => error,
            })
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_none()
    }

    /// An `UploadedFile` sent empty, as Inertia sends a `null` file, is
    /// missing.
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name == UPLOADED_FILE {
            self.collector.record(self.path, FieldFailure::Required);
            offer_placeholder(visitor)
        } else {
            visitor.visit_newtype_struct(self)
        }
    }

    required! {
        deserialize_bool();
        deserialize_i8();
        deserialize_i16();
        deserialize_i32();
        deserialize_i64();
        deserialize_i128();
        deserialize_u8();
        deserialize_u16();
        deserialize_u32();
        deserialize_u64();
        deserialize_u128();
        deserialize_f32();
        deserialize_f64();
        deserialize_char();
        deserialize_str();
        deserialize_string();
        deserialize_bytes();
        deserialize_byte_buf();
        deserialize_seq();
        deserialize_tuple(len: usize);
        deserialize_tuple_struct(name: &'static str, len: usize);
        deserialize_map();
        deserialize_struct(name: &'static str, fields: &'static [&'static str]);
        deserialize_enum(name: &'static str, variants: &'static [&'static str]);
        deserialize_identifier();
    }

    serde::forward_to_deserialize_any! {
        unit unit_struct ignored_any
    }
}

/// The members of bracketed names at the input name `path`.
///
/// An array whose keys are all indexes is a list, read in index order; any
/// array also reads as a map or a struct, by its keys. A list is read as a
/// list wherever the type asks for any value, as a `serde_json::Value`
/// does, and an array with another key as a map.
struct FormArray<'a, 'p, 'c> {
    array: Array<'a>,
    path: &'p str,
    collector: &'c Collector,
}

/// An array where one value belongs: recorded under the array's name, with
/// the key for the type asked for, and a placeholder stands in.
macro_rules! not_a_value {
    ($($method:ident($($arg:ident: $ty:ty),*) => $failure:ident;)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            self.collector.record(self.path, FieldFailure::$failure);
            de::Deserializer::$method(Placeholder, $($arg,)* visitor)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for FormArray<'_, '_, '_> {
    type Error = FieldError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if self.array.is_list() {
            de::Deserializer::deserialize_seq(self, visitor)
        } else {
            de::Deserializer::deserialize_map(self, visitor)
        }
    }

    /// The elements in index order. An array with a key that is no index
    /// is no list: recorded as a format failure.
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let (path, collector) = (self.path, self.collector);
        let Some(items) = self.array.into_list() else {
            collector.record(path, FieldFailure::Format);
            return de::Deserializer::deserialize_seq(Placeholder, visitor);
        };
        visitor
            .visit_seq(Items {
                items: items.into_iter(),
                path,
                collector,
            })
            .map_err(|error| match error.0 {
                Stop::Other(_) => {
                    collector.record(path, FieldFailure::Format);
                    FieldError::recorded()
                }
                _ => error,
            })
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        de::Deserializer::deserialize_seq(self, visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        de::Deserializer::deserialize_seq(self, visitor)
    }

    /// The members by key. A struct that misses a required member records
    /// it under `path`, `user.name` for `name` in `user`.
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let (path, collector) = (self.path, self.collector);
        visitor
            .visit_map(Members {
                members: self.array.into_members(),
                path,
                collector,
                pending: None,
            })
            .map_err(|error| collector.settle_missing(path, error))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        de::Deserializer::deserialize_map(self, visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    /// An array where an `UploadedFile` belongs is a field's failure
    /// (`validation-file`).
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name == UPLOADED_FILE {
            self.collector.record(self.path, FieldFailure::File);
            offer_placeholder(visitor)
        } else {
            visitor.visit_newtype_struct(self)
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    not_a_value! {
        deserialize_bool() => Boolean;
        deserialize_i8() => Integer;
        deserialize_i16() => Integer;
        deserialize_i32() => Integer;
        deserialize_i64() => Integer;
        deserialize_i128() => Integer;
        deserialize_u8() => Integer;
        deserialize_u16() => Integer;
        deserialize_u32() => Integer;
        deserialize_u64() => Integer;
        deserialize_u128() => Integer;
        deserialize_f32() => Numeric;
        deserialize_f64() => Numeric;
        deserialize_char() => String;
        deserialize_str() => String;
        deserialize_string() => String;
        deserialize_bytes() => Format;
        deserialize_byte_buf() => Format;
        deserialize_unit() => Format;
        deserialize_unit_struct(name: &'static str) => Format;
        deserialize_enum(name: &'static str, variants: &'static [&'static str]) => Format;
        deserialize_identifier() => Format;
    }
}

/// The elements of a list, each a field's value under `path.index`.
struct Items<'a, 'p, 'c> {
    items: std::vec::IntoIter<(i64, Node<'a>)>,
    path: &'p str,
    collector: &'c Collector,
}

impl<'de> de::SeqAccess<'de> for Items<'_, '_, '_> {
    type Error = FieldError;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        let Some((index, node)) = self.items.next() else {
            return Ok(None);
        };
        read_node(seed, node, &join(self.path, index), self.collector).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

/// The members of an array read as a map, each a field's value under
/// `path.key`.
struct Members<'a, 'p, 'c> {
    members: indexmap::map::IntoIter<Key<'a>, Node<'a>>,
    path: &'p str,
    collector: &'c Collector,
    /// The member whose key was read and whose value is next.
    pending: Option<(Cow<'a, str>, Node<'a>)>,
}

impl<'de> de::MapAccess<'de> for Members<'_, '_, '_> {
    type Error = FieldError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, node)) = self.members.next() else {
            return Ok(None);
        };
        let name = match key {
            Key::Index(index) => Cow::Owned(index.to_string()),
            Key::Name(name) => name,
        };
        let key = seed.deserialize(FormValue::key(&name, self.collector));
        self.pending = Some((name, node));
        key.map(Some)
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        let Some((name, node)) = self.pending.take() else {
            return Err(de::Error::custom("a value was read before its name"));
        };
        read_node(seed, node, &join(self.path, &name), self.collector)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.members.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Profile {
        name: String,
        #[serde(rename = "about")]
        bio: Option<String>,
        #[serde(default)]
        tags: Vec<u32>,
    }

    fn read<T: DeserializeOwned>(body: &str) -> Result<T, InputError> {
        parse_form_input(body.as_bytes())
    }

    fn failed_fields<T: DeserializeOwned + std::fmt::Debug>(body: &str) -> Vec<(String, String)> {
        match read::<T>(body) {
            Err(InputError::Fields(errors)) => {
                let mut fields: Vec<(String, String)> = errors
                    .errors
                    .iter()
                    .map(|(field, messages)| (field.clone(), messages[0].key.to_string()))
                    .collect();
                fields.sort();
                fields
            }
            other => panic!("`{body}`: expected field failures, got {other:?}"),
        }
    }

    #[test]
    fn a_struct_reads_empty_values_repeats_and_lists_as_laravel_does() {
        let profile: Profile =
            read("name=&name=Ada&about=x&about=&tags[]=1&tags%5B%5D=3").expect("a profile");
        assert_eq!(
            profile,
            Profile {
                name: "Ada".into(),
                bio: None,
                tags: vec![1, 3],
            }
        );
    }

    #[test]
    fn every_field_that_fails_is_named_with_its_key() {
        assert_eq!(
            failed_fields::<Profile>("about=Hi&tags[]=1&tags[]=two&tags[]=&tags[]=x"),
            [
                ("name".to_string(), "validation-required".to_string()),
                ("tags.1".to_string(), "validation-integer".to_string()),
                ("tags.2".to_string(), "validation-required".to_string()),
                ("tags.3".to_string(), "validation-integer".to_string()),
            ]
        );
        // One value where a list belongs, and a list where one value does.
        assert_eq!(
            failed_fields::<Profile>("name[]=Ada&tags=1"),
            [
                ("name".to_string(), "validation-string".to_string()),
                ("tags".to_string(), "validation-format".to_string()),
            ]
        );
    }

    #[test]
    fn a_name_that_is_no_field_passes_through_as_sent() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Strict {
            name: String,
        }
        match read::<Strict>("name=Ada&other=1&other=2") {
            Err(InputError::Other(message)) => {
                assert!(message.contains("unknown field `other`"), "{message}")
            }
            other => panic!("expected the unknown field refused, got {other:?}"),
        }
        let strict: Strict = read("name=Ada").expect("a strict struct");
        assert_eq!(strict.name, "Ada");
    }

    #[test]
    fn a_map_and_a_list_of_pairs_read_every_name() {
        let map: BTreeMap<String, Option<String>> = read("a=1&a=2&b=&c=3").expect("a map");
        assert_eq!(
            map,
            BTreeMap::from([
                ("a".into(), Some("2".into())),
                ("b".into(), None),
                ("c".into(), Some("3".into())),
            ])
        );

        let value: serde_json::Value =
            read("q=rust&page=&tags[]=a&tags[]=&tags[]=b").expect("a value");
        assert_eq!(
            value,
            serde_json::json!({ "q": "rust", "page": null, "tags": ["a", null, "b"] })
        );

        // A map whose values cannot be null names the cleared one.
        assert_eq!(
            failed_fields::<BTreeMap<String, String>>("a=1&b="),
            [("b".to_string(), "validation-required".to_string())]
        );

        let pairs: Vec<(String, u32)> = read("a=1&b=2").expect("pairs");
        assert_eq!(pairs, [("a".to_string(), 1), ("b".to_string(), 2)]);

        let none: () = read("").expect("nothing");
        assert_eq!(none, ());
    }

    #[test]
    fn an_enum_reads_a_unit_variant_and_a_map_key_parses() {
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(rename_all = "lowercase")]
        enum Sort {
            Name,
            Date,
        }
        #[derive(Debug, Deserialize)]
        struct Listing {
            sort: Sort,
        }
        let listing: Listing = read("sort=date").expect("a listing");
        assert_eq!(listing.sort, Sort::Date);
        assert_eq!(
            failed_fields::<Listing>("sort=size"),
            [("sort".to_string(), "validation-format".to_string())]
        );

        let scores: BTreeMap<u32, String> = read("1=a&2=b").expect("numeric keys");
        assert_eq!(scores[&2], "b");
        assert!(matches!(
            read::<BTreeMap<u32, String>>("x=a"),
            Err(InputError::Other(_))
        ));
    }

    #[test]
    fn an_empty_value_is_a_key_holding_null() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Patch {
            #[serde(default, deserialize_with = "cleared")]
            bio: Option<Option<String>>,
            #[serde(default, deserialize_with = "cleared")]
            name: Option<Option<String>>,
            nickname: Option<String>,
        }
        // A field told apart as cleared (`Some(None)`) or never sent (`None`).
        fn cleared<'de, D: de::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Option<String>>, D::Error> {
            Option::<String>::deserialize(deserializer).map(Some)
        }
        let patch: Patch = read("bio=&nickname=").expect("a patch");
        assert_eq!(
            patch,
            Patch {
                bio: Some(None),
                name: None,
                nickname: None,
            }
        );
    }

    #[test]
    fn a_bool_reads_what_forms_send() {
        #[derive(Debug, Deserialize)]
        struct Consent {
            terms: bool,
        }
        for (sent, value) in [
            ("1", true),
            ("0", false),
            ("On", true),
            ("off", false),
            ("TRUE", true),
        ] {
            let consent: Consent = read(&format!("terms={sent}")).expect("a bool");
            assert_eq!(consent.terms, value, "{sent}");
        }
        assert_eq!(
            failed_fields::<Consent>("terms=yes"),
            [("terms".to_string(), "validation-boolean".to_string())]
        );
    }

    #[test]
    fn an_encoded_value_reads_decoded() {
        let map: BTreeMap<String, String> = read("q=a+b%26c%3Dd&q2=%C3%A9").expect("a map");
        assert_eq!(map["q"], "a b&c=d");
        assert_eq!(map["q2"], "\u{e9}");
    }
}
