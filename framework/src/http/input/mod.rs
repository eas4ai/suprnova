//! Typed input read from a request the way Laravel reads one, failing field
//! by field.
//!
//! serde stops at the first error and words it for a developer: `missing
//! field `title``, `invalid digit found in string`. A form needs every field
//! that failed at once, each under the name the form used and with a catalog
//! message, so that the Inertia validation redirect shows each one under its
//! input, as the multipart extractor does. The deserializers here record a
//! field that failed and stand a placeholder value in for it, so the
//! struct's own `Deserialize` goes on to the next field, and a struct that
//! is missing required fields is asked again until it names no new one.
//!
//! [`parse_form_input`] is the url-encoded reader for bodies and query
//! strings. [`json_field_failures`] reads nothing: it runs only after
//! `serde_json` refused a body, to name the fields that made it fail, so
//! what a JSON body accepts stays exactly what `serde_json` accepts.

mod form;
mod json;
mod placeholder;

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use serde::de::{self, DeserializeOwned};

use crate::error::ValidationErrors;
use crate::http::upload::{FieldFailure, add_field_failure};

pub(crate) use form::parse_form_input;
pub(crate) use json::json_field_failures;

use placeholder::Skeleton;

/// Why a read of typed input failed.
#[derive(Debug)]
pub(crate) enum InputError {
    /// Fields that failed, each under its input name.
    Fields(ValidationErrors),
    /// A failure that belongs to no field, such as an unknown field a
    /// struct denies, as serde worded it.
    Other(String),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fields(errors) => write!(f, "{errors}"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

/// The error the deserializers here return. It says why a read stopped,
/// so a missing field can be named and a field already recorded is not
/// recorded twice.
#[derive(Debug)]
pub(crate) struct FieldError(Stop);

#[derive(Debug)]
enum Stop {
    /// A required field the struct did not receive, by its wire name.
    Missing(&'static str),
    /// A field already recorded in the [`Collector`] ended the read.
    Recorded,
    /// Anything else, as serde worded it.
    Other(String),
}

impl FieldError {
    fn recorded() -> Self {
        Self(Stop::Recorded)
    }
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Stop::Missing(field) => write!(f, "missing field `{field}`"),
            Stop::Recorded => f.write_str("a field failed"),
            Stop::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FieldError {}

impl de::Error for FieldError {
    fn custom<M: fmt::Display>(message: M) -> Self {
        Self(Stop::Other(message.to_string()))
    }

    fn missing_field(field: &'static str) -> Self {
        Self(Stop::Missing(field))
    }
}

/// The failures one read records, shared by every deserializer the read
/// runs.
#[derive(Default)]
struct Collector {
    failures: RefCell<Vec<(String, FieldFailure)>>,
    /// The input names already recorded: a field is reported once, by the
    /// first failure found for it.
    recorded: RefCell<HashSet<String>>,
    /// An error carried across a deserializer whose error type is not
    /// [`FieldError`], `serde_json`'s: set just before that deserializer
    /// is handed a plain error to return, and taken back as it returns it.
    carried: RefCell<Option<FieldError>>,
}

impl Collector {
    fn record(&self, path: &str, failure: FieldFailure) {
        if self.recorded.borrow_mut().insert(path.to_string()) {
            self.failures.borrow_mut().push((path.to_string(), failure));
        }
    }

    fn has_failures(&self) -> bool {
        !self.failures.borrow().is_empty()
    }

    /// File a struct visitor's error at `path`, the path of the struct: a
    /// missing field becomes that field's `validation-required`.
    fn settle_missing(&self, path: &str, error: FieldError) -> FieldError {
        match error.0 {
            Stop::Missing(field) => {
                self.record(&join(path, field), FieldFailure::Required);
                FieldError::recorded()
            }
            stop => FieldError(stop),
        }
    }

    /// Hand `error` across a deserializer whose error type is `E`.
    fn carry<E: de::Error>(&self, error: FieldError) -> E {
        let message = error.to_string();
        *self.carried.borrow_mut() = Some(error);
        E::custom(message)
    }

    /// Take back what [`Collector::carry`] handed across, or word an error
    /// that deserializer raised itself.
    fn take<E: fmt::Display>(&self, error: E) -> FieldError {
        self.carried
            .borrow_mut()
            .take()
            .unwrap_or_else(|| FieldError(Stop::Other(error.to_string())))
    }

    fn into_errors(self) -> ValidationErrors {
        let mut errors = ValidationErrors::new();
        for (path, failure) in self.failures.into_inner() {
            add_field_failure(&mut errors, &path, None, failure);
        }
        errors
    }
}

/// `field` under the input name `path`, joined the way a validation error
/// names a nested field: `address.street`, `items.1`.
fn join(path: &str, field: impl fmt::Display) -> String {
    if path.is_empty() {
        field.to_string()
    } else {
        format!("{path}.{field}")
    }
}

/// Record every required field of `T` that the input left out.
///
/// A derived `Deserialize` names one missing field, the first in
/// declaration order, and stops. So `T` is read again from placeholders
/// alone, one for each field in `present` and each missing field found so
/// far, until it names no new one. Each read touches no input, and there
/// is at most one per field.
fn record_missing_fields<T: DeserializeOwned>(
    collector: &Collector,
    fields: &'static [&'static str],
    mut present: Vec<&'static str>,
) {
    for _ in 0..=fields.len() {
        match T::deserialize(Skeleton { fields: &present }) {
            Err(FieldError(Stop::Missing(field))) if !present.contains(&field) => {
                collector.record(field, FieldFailure::Required);
                present.push(field);
            }
            _ => return,
        }
    }
}

/// The names of `T`'s fields when `T` deserializes as a struct; `None`
/// when it deserializes as anything else, a map or a struct with a
/// flattened field among them, which can take any name.
///
/// A derived `Deserialize` hands its field names to the deserializer
/// before it reads a byte, so a deserializer that keeps the names and
/// stops there reads them without any input.
fn struct_field_names<T: DeserializeOwned>() -> Option<&'static [&'static str]> {
    match T::deserialize(FieldNames) {
        Err(FieldNamesRead(fields)) => fields,
        Ok(_) => None,
    }
}

/// The deserializer [`struct_field_names`] runs: it fails at once,
/// carrying the field names when it was asked for a struct.
struct FieldNames;

/// The "error" that carries what [`FieldNames`] read.
#[derive(Debug)]
struct FieldNamesRead(Option<&'static [&'static str]>);

impl fmt::Display for FieldNamesRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("read the field names of a struct, not a value")
    }
}

impl std::error::Error for FieldNamesRead {}

impl de::Error for FieldNamesRead {
    fn custom<M: fmt::Display>(_message: M) -> Self {
        Self(None)
    }
}

impl<'de> de::Deserializer<'de> for FieldNames {
    type Error = FieldNamesRead;

    fn deserialize_any<V: de::Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(FieldNamesRead(None))
    }

    fn deserialize_struct<V: de::Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(FieldNamesRead(Some(fields)))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Deserialize)]
    struct Profile {
        name: String,
        #[serde(rename = "about", alias = "bio")]
        bio: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct WithExtra {
        name: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Option<String>>,
    }

    #[test]
    fn a_struct_hands_over_its_wire_names_and_aliases_and_a_map_none() {
        let fields = struct_field_names::<Profile>().expect("a struct");
        assert!(fields.contains(&"name"));
        assert!(fields.contains(&"about"));
        assert!(
            fields.contains(&"bio"),
            "an alias is a name the struct reads"
        );
        assert_eq!(struct_field_names::<BTreeMap<String, String>>(), None);
        assert_eq!(struct_field_names::<WithExtra>(), None);

        // The alias reads like the name, and a flattened struct reads.
        let profile: Profile = parse_form_input(b"name=Ada&bio=Hi").expect("a profile");
        assert_eq!(
            (profile.name.as_str(), profile.bio.as_deref()),
            ("Ada", Some("Hi"))
        );
        let extra: WithExtra =
            parse_form_input(b"name=x&name=y&x=1&x=2&blank=").expect("a flattened struct");
        assert_eq!(extra.name, "y");
        assert_eq!(
            extra.extra,
            BTreeMap::from([("blank".into(), None), ("x".into(), Some("2".into()))])
        );
    }

    #[derive(Debug, Deserialize)]
    struct Five {
        a: String,
        b: u32,
        c: Option<bool>,
        #[serde(default)]
        d: bool,
        e: Vec<u8>,
    }

    #[test]
    fn a_skeleton_read_names_every_missing_field_once() {
        let collector = Collector::default();
        record_missing_fields::<Five>(
            &collector,
            struct_field_names::<Five>().expect("a struct"),
            vec!["b"],
        );
        let errors = collector.into_errors();
        let mut keys: Vec<&str> = errors.errors.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["a", "e"]);

        let five: Five = serde_json::from_str(r#"{"a": "x", "b": 1, "e": [2]}"#).expect("all five");
        assert_eq!(
            (five.a.as_str(), five.b, five.c, five.d, five.e),
            ("x", 1, None, false, vec![2])
        );
    }
}
