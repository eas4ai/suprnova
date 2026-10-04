//! The url-encoded reader: a form body or a query string into a typed value.
//!
//! It reads the pairs as Laravel's request holds them once PHP has parsed
//! them and `ConvertEmptyStringsToNull` has run. An empty value is `null`,
//! so it is left out. A name sent more than once keeps its last value. A
//! name that ends in `[]` is a list, read under the name without the
//! brackets, with its empty elements left out. A value is read as
//! `serde_urlencoded` reads one, through the field type's `FromStr`, and a
//! value that does not parse is recorded under its input name with the key
//! for the field's type, as the multipart extractor files it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, Visitor};
use url::form_urlencoded;

use super::placeholder::Placeholder;
use super::{
    Collector, FieldError, InputError, Stop, join, record_missing_fields, struct_field_names,
};
use crate::http::upload::FieldFailure;

/// Read url-encoded `bytes` into `T`, failing field by field.
pub(crate) fn parse_form_input<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, InputError> {
    let fields = struct_field_names::<T>();
    let form = Form::index(bytes, fields);
    let present = fields.map(|fields| form.present(fields));
    let collector = Collector::default();
    let error = match T::deserialize(FormInput {
        form,
        collector: &collector,
    }) {
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

/// Url-encoded input indexed for a read: where each name the read uses was
/// last sent, and the elements of each list.
///
/// Only the names a struct reads are indexed, so the index holds one entry
/// per field however many names the client sends. Any other name a struct
/// ignores, or refuses at once when it denies unknown fields, so those pass
/// through as they were sent. A target with no fixed names, such as a map,
/// indexes every name.
struct Form<'a> {
    bytes: &'a [u8],
    /// The names a read uses, or `None` for every name.
    tracked: Option<HashSet<&'static str>>,
    /// For each indexed name that is not a list, the position of its last
    /// pair and whether that pair has a value.
    last: HashMap<Cow<'a, str>, (usize, bool)>,
    /// Each indexed list, by its name without the brackets.
    lists: HashMap<Cow<'a, str>, List<'a>>,
}

/// The pairs of one list name.
struct List<'a> {
    /// The position of the list's first pair, where the read meets it.
    first: usize,
    /// How many pairs the list has, null elements included.
    count: usize,
    /// The elements that are not null, each with its index among all the
    /// list's pairs, which names its error as the multipart extractor
    /// names a part's.
    items: Vec<(usize, Cow<'a, str>)>,
}

/// What one read entry holds.
enum Entry<'a> {
    One(Cow<'a, str>),
    List(Vec<(usize, Cow<'a, str>)>),
}

impl<'a> Form<'a> {
    fn index(bytes: &'a [u8], fields: Option<&'static [&'static str]>) -> Self {
        let mut form = Form {
            bytes,
            tracked: fields.map(|fields| fields.iter().copied().collect()),
            last: HashMap::new(),
            lists: HashMap::new(),
        };
        for (at, (name, value)) in form_urlencoded::parse(bytes).enumerate() {
            match list_name(name) {
                Ok(base) => {
                    if form.tracks(&base) {
                        let list = form.lists.entry(base).or_insert_with(|| List {
                            first: at,
                            count: 0,
                            items: Vec::new(),
                        });
                        if !value.is_empty() {
                            list.items.push((list.count, value));
                        }
                        list.count += 1;
                    }
                }
                Err(name) => {
                    if form.tracks(&name) {
                        form.last.insert(name, (at, !value.is_empty()));
                    }
                }
            }
        }
        form
    }

    fn tracks(&self, name: &str) -> bool {
        self.tracked
            .as_ref()
            .is_none_or(|tracked| tracked.contains(name))
    }

    /// Which of `fields` the read hands the struct a value for.
    fn present(&self, fields: &'static [&'static str]) -> Vec<&'static str> {
        fields
            .iter()
            .copied()
            .filter(|field| {
                self.lists.contains_key(*field)
                    || self
                        .last
                        .get(*field)
                        .is_some_and(|&(_, has_value)| has_value)
            })
            .collect()
    }
}

/// `Ok` with `name` less its trailing `[]`, PHP's mark for a list, or
/// `Err(name)` for a name that is not a list.
fn list_name(name: Cow<'_, str>) -> Result<Cow<'_, str>, Cow<'_, str>> {
    if !name.ends_with("[]") {
        return Err(name);
    }
    Ok(match name {
        Cow::Borrowed(name) => Cow::Borrowed(&name[..name.len() - 2]),
        Cow::Owned(mut name) => {
            name.truncate(name.len() - 2);
            Cow::Owned(name)
        }
    })
}

/// The top-level deserializer of a url-encoded read.
struct FormInput<'a, 'c> {
    form: Form<'a>,
    collector: &'c Collector,
}

impl<'a, 'c> FormInput<'a, 'c> {
    fn entries(self) -> Entries<'a, 'c> {
        Entries {
            pairs: form_urlencoded::parse(self.form.bytes).enumerate(),
            form: self.form,
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

/// The entries of a read, in the order their names were first or last
/// sent: a name that is not a list where its last pair is, a list where
/// its first pair is.
struct Entries<'a, 'c> {
    form: Form<'a>,
    pairs: std::iter::Enumerate<form_urlencoded::Parse<'a>>,
    collector: &'c Collector,
    /// The entry whose key was read and whose value is next.
    pending: Option<(Cow<'a, str>, Entry<'a>)>,
}

impl<'a> Entries<'a, '_> {
    fn next_entry(&mut self) -> Option<(Cow<'a, str>, Entry<'a>)> {
        for (at, (name, value)) in self.pairs.by_ref() {
            let list = name
                .strip_suffix("[]")
                .and_then(|base| self.form.lists.get_mut(base));
            if let Some(list) = list {
                if list.first == at {
                    let items = std::mem::take(&mut list.items);
                    if let Ok(base) = list_name(name) {
                        return Some((base, Entry::List(items)));
                    }
                }
                continue;
            }
            let read_as_sent = name.ends_with("[]") || !self.form.tracks(&name);
            let last = !read_as_sent && self.form.last.get(name.as_ref()) == Some(&(at, true));
            if (read_as_sent && !value.is_empty()) || last {
                return Some((name, Entry::One(value)));
            }
        }
        None
    }
}

impl<'de> de::MapAccess<'de> for Entries<'_, '_> {
    type Error = FieldError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        let Some((name, entry)) = self.next_entry() else {
            return Ok(None);
        };
        let key = seed.deserialize(FormValue::key(&name, self.collector));
        self.pending = Some((name, entry));
        key.map(Some)
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        let Some((name, entry)) = self.pending.take() else {
            return Err(de::Error::custom("a value was read before its name"));
        };
        match entry {
            Entry::One(text) => seed.deserialize(FormValue {
                text: &text,
                path: Some(&name),
                collector: self.collector,
            }),
            Entry::List(items) => seed.deserialize(FormList {
                items: &items,
                path: &name,
                collector: self.collector,
            }),
        }
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
        let Some((name, entry)) = self.0.next_entry() else {
            return Ok(None);
        };
        self.0.pending = Some((name, entry));
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

    parsed! {
        deserialize_bool: bool => visit_bool(false), Boolean;
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

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
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

/// The elements of a list name, read as a sequence of text values.
struct FormList<'v, 'c> {
    items: &'v [(usize, Cow<'v, str>)],
    path: &'v str,
    collector: &'c Collector,
}

/// A list where one value belongs: recorded under the list's name, with
/// the key for the type asked for, and a placeholder stands in.
macro_rules! not_a_list {
    ($($method:ident($($arg:ident: $ty:ty),*) => $failure:ident;)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            self.collector.record(self.path, FieldFailure::$failure);
            de::Deserializer::$method(Placeholder, $($arg,)* visitor)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for FormList<'_, '_> {
    type Error = FieldError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let (path, collector) = (self.path, self.collector);
        visitor
            .visit_seq(Items {
                items: self.items.iter(),
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

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    not_a_list! {
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
        deserialize_map() => Format;
        deserialize_struct(name: &'static str, fields: &'static [&'static str]) => Format;
        deserialize_enum(name: &'static str, variants: &'static [&'static str]) => Format;
        deserialize_identifier() => Format;
    }

    serde::forward_to_deserialize_any! {
        seq tuple tuple_struct
    }
}

/// The elements of a [`FormList`], each a field's value under
/// `name.index`.
struct Items<'i, 'v, 'c> {
    items: std::slice::Iter<'i, (usize, Cow<'v, str>)>,
    path: &'i str,
    collector: &'c Collector,
}

impl<'de> de::SeqAccess<'de> for Items<'_, '_, '_> {
    type Error = FieldError;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        let Some((index, text)) = self.items.next() else {
            return Ok(None);
        };
        let path = join(self.path, index);
        seed.deserialize(FormValue {
            text,
            path: Some(&path),
            collector: self.collector,
        })
        .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
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
            read("name=&name=Ada&about=x&about=&tags[]=1&tags[]=&tags%5B%5D=3").expect("a profile");
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
        let map: BTreeMap<String, String> = read("a=1&a=2&b=&c=3").expect("a map");
        assert_eq!(
            map,
            BTreeMap::from([("a".into(), "2".into()), ("c".into(), "3".into())])
        );

        let value: serde_json::Value = read("q=rust&tags[]=a&tags[]=b").expect("a value");
        assert_eq!(
            value,
            serde_json::json!({ "q": "rust", "tags": ["a", "b"] })
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
    fn an_encoded_value_reads_decoded() {
        let map: BTreeMap<String, String> = read("q=a+b%26c%3Dd&q2=%C3%A9").expect("a map");
        assert_eq!(map["q"], "a b&c=d");
        assert_eq!(map["q2"], "\u{e9}");
    }
}
