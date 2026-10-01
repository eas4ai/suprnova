//! Duplicate-aware, resource-bounded canonical value parser.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use super::{CanonicalError, CanonicalErrorKind, CanonicalNumber, CanonicalValue};
use crate::limits::InputLimits;

struct ParseBudget<'limits> {
    limits: &'limits InputLimits,
    entries: Cell<usize>,
    error: Cell<Option<CanonicalErrorKind>>,
}

impl ParseBudget<'_> {
    fn fail<E: de::Error>(&self, kind: CanonicalErrorKind) -> E {
        if self.error.get().is_none() {
            self.error.set(Some(kind));
        }
        E::custom(kind.as_str())
    }

    fn count_entry<E: de::Error>(&self) -> Result<(), E> {
        let next = self.entries.get().saturating_add(1);
        if next > self.limits.max_entries() {
            return Err(self.fail(CanonicalErrorKind::TooManyEntries));
        }
        self.entries.set(next);
        Ok(())
    }

    fn validate_string<E: de::Error>(&self, value: &str) -> Result<(), E> {
        if value.len() > self.limits.max_string_bytes() {
            return Err(self.fail(CanonicalErrorKind::StringTooLong));
        }
        Ok(())
    }
}

struct ValueSeed<'budget, 'limits> {
    budget: &'budget ParseBudget<'limits>,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for ValueSeed<'_, '_> {
    type Value = CanonicalValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct ValueVisitor<'budget, 'limits> {
    budget: &'budget ParseBudget<'limits>,
    depth: usize,
}

impl<'de> Visitor<'de> for ValueVisitor<'_, '_> {
    type Value = CanonicalValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded canonical JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CanonicalNumber::from_i64(value)
            .map(CanonicalValue::Number)
            .map_err(|error| self.budget.fail(error.kind()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CanonicalNumber::from_u64(value)
            .map(CanonicalValue::Number)
            .map_err(|error| self.budget.fail(error.kind()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        CanonicalNumber::new(value)
            .map(CanonicalValue::Number)
            .map_err(|error| self.budget.fail(error.kind()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.budget.validate_string(value)?;
        Ok(CanonicalValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.budget.validate_string(&value)?;
        Ok(CanonicalValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.depth >= self.budget.limits.max_depth() {
            return Err(self.budget.fail(CanonicalErrorKind::TooDeep));
        }
        if sequence.size_hint().is_some_and(|hint| {
            hint > self
                .budget
                .limits
                .max_entries()
                .saturating_sub(self.budget.entries.get())
        }) {
            return Err(self.budget.fail(CanonicalErrorKind::TooManyEntries));
        }

        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(ValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            self.budget.count_entry()?;
            values.push(value);
        }
        Ok(CanonicalValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.depth >= self.budget.limits.max_depth() {
            return Err(self.budget.fail(CanonicalErrorKind::TooDeep));
        }
        if map.size_hint().is_some_and(|hint| {
            hint > self
                .budget
                .limits
                .max_entries()
                .saturating_sub(self.budget.entries.get())
        }) {
            return Err(self.budget.fail(CanonicalErrorKind::TooManyEntries));
        }

        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            self.budget.validate_string(&key)?;
            self.budget.count_entry()?;
            if values.contains_key(&key) {
                return Err(self.budget.fail(CanonicalErrorKind::DuplicateKey));
            }
            let value = map.next_value_seed(ValueSeed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(CanonicalValue::Object(values))
    }
}

/// Parses exactly one bounded JSON value while rejecting duplicate keys.
pub fn parse_canonical_value(
    input: &[u8],
    limits: &InputLimits,
) -> Result<CanonicalValue, CanonicalError> {
    if input.len() > limits.max_bytes() {
        return Err(CanonicalError::new(CanonicalErrorKind::TooLarge));
    }
    if std::str::from_utf8(input).is_err() {
        return Err(CanonicalError::new(CanonicalErrorKind::InvalidUtf8));
    }

    let budget = ParseBudget {
        limits,
        entries: Cell::new(0),
        error: Cell::new(None),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(input);
    let result = ValueSeed {
        budget: &budget,
        depth: 0,
    }
    .deserialize(&mut deserializer);

    match result {
        Ok(value) => deserializer.end().map(|()| value).map_err(|_| {
            CanonicalError::new(
                budget
                    .error
                    .get()
                    .unwrap_or(CanonicalErrorKind::InvalidJson),
            )
        }),
        Err(_) => Err(CanonicalError::new(
            budget
                .error
                .get()
                .unwrap_or(CanonicalErrorKind::InvalidJson),
        )),
    }
}
