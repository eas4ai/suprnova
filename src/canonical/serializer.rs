//! Canonical RFC 8785-compatible serialization.

use std::io::{self, Write};

use super::{CanonicalError, CanonicalErrorKind, CanonicalValue};
use crate::limits::InputLimits;

fn validate_value(
    value: &CanonicalValue,
    limits: &InputLimits,
    depth: usize,
    entries: &mut usize,
) -> Result<(), CanonicalError> {
    match value {
        CanonicalValue::String(value) => {
            if value.len() > limits.max_string_bytes() {
                return Err(CanonicalError::new(CanonicalErrorKind::StringTooLong));
            }
        }
        CanonicalValue::Array(values) => {
            validate_container(limits, depth)?;
            for value in values {
                count_entry(limits, entries)?;
                validate_value(value, limits, depth + 1, entries)?;
            }
        }
        CanonicalValue::Object(values) => {
            validate_container(limits, depth)?;
            for (key, value) in values {
                if key.len() > limits.max_string_bytes() {
                    return Err(CanonicalError::new(CanonicalErrorKind::StringTooLong));
                }
                count_entry(limits, entries)?;
                validate_value(value, limits, depth + 1, entries)?;
            }
        }
        CanonicalValue::Null | CanonicalValue::Bool(_) | CanonicalValue::Number(_) => {}
    }
    Ok(())
}

fn validate_container(limits: &InputLimits, depth: usize) -> Result<(), CanonicalError> {
    if depth >= limits.max_depth() {
        return Err(CanonicalError::new(CanonicalErrorKind::TooDeep));
    }
    Ok(())
}

fn count_entry(limits: &InputLimits, entries: &mut usize) -> Result<(), CanonicalError> {
    *entries = entries.saturating_add(1);
    if *entries > limits.max_entries() {
        return Err(CanonicalError::new(CanonicalErrorKind::TooManyEntries));
    }
    Ok(())
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(1_024)),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("canonical_output_limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serializes a validated value to deterministic RFC 8785-compatible UTF-8.
pub fn to_canonical_bytes(
    value: &CanonicalValue,
    limits: &InputLimits,
) -> Result<Vec<u8>, CanonicalError> {
    validate_value(value, limits, 0, &mut 0)?;
    let mut writer = BoundedWriter::new(limits.max_bytes());
    if serde_json_canonicalizer::to_writer(value, &mut writer).is_err() {
        let kind = if writer.exceeded {
            CanonicalErrorKind::TooLarge
        } else {
            CanonicalErrorKind::SerializationFailed
        };
        return Err(CanonicalError::new(kind));
    }
    Ok(writer.bytes)
}
