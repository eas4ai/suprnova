//! The multipart reader: a `multipart/form-data` body into a typed value.
//!
//! The parts are read as nested form data, as a url-encoded body is: the
//! Inertia client sends a form with a file as multipart, under bracketed
//! and indexed names (`user[name]`, `photos[0]`), and Laravel reads the
//! parts as PHP nests them. A text part reads as a url-encoded value reads.
//! A part with a file name is a file, which only an [`UploadedFile`] field
//! takes: a `serde_json::Value` holds `null` in its place, as Laravel's
//! `input()` reads a file field, and a field of any other type fails with
//! `validation-string`, as the multipart extractor files it.
//!
//! serde has no way to hand a deserializer's own value to a type, so the
//! file goes through a slot. An `UploadedFile` asks for a newtype struct
//! named [`UPLOADED_FILE`]; the reader puts the part in a thread-local
//! slot and the `UploadedFile` takes it from there, runs its validators on
//! it, and leaves a refusal in the slot for the reader to record under the
//! field's path. Any other deserializer leaves the slot empty, so an
//! `UploadedFile` read from JSON fails. A deserialization runs on one
//! thread without yielding, and the slot is emptied when each hand-off
//! ends, so a file never reaches any read but the one that parsed it.

use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;

use bytes::Bytes;
use serde::de::{self, DeserializeOwned, IgnoredAny, Visitor};
use serde::{Deserialize, Deserializer};

use super::form::read_nested;
use super::nested::{FilePart, Nested};
use super::placeholder::Placeholder;
use super::{Collector, FieldError, InputError, struct_field_names};
use crate::error::FrameworkError;
use crate::http::upload::validators::{ReceivedPart, UploadValidator};
use crate::http::upload::{FieldFailure, MultipartPayload, UploadedFile, UploadedFileBacking};
use crate::validation::message::ValidationMessage;

/// The name an [`UploadedFile`] asks a deserializer for, so a multipart
/// read knows to hand it a file. No other deserializer gives it a meaning.
pub(super) const UPLOADED_FILE: &str = "$suprnova::http::upload::UploadedFile";

/// Read the parts of a multipart body into `T`, failing field by field.
pub(crate) fn parse_multipart_input<T: DeserializeOwned>(
    payload: MultipartPayload,
) -> Result<T, InputError> {
    let fields = struct_field_names::<T>();
    read_nested(Nested::from_multipart(payload, fields), fields)
}

/// What a read hands the [`UploadedFile`] it is building.
enum Handoff {
    /// Nothing: the deserializer is no multipart read.
    Empty,
    /// The file a part carries.
    File(FilePart),
    /// No file: a failure is recorded for the field, and a placeholder
    /// stands in so the read goes on to the next field.
    Placeholder,
    /// The file's validators refused it with this message.
    Refused(ValidationMessage),
    /// A validator failed with an error that is no field's.
    Failed(FrameworkError),
}

thread_local! {
    static HANDOFF: RefCell<Handoff> = const { RefCell::new(Handoff::Empty) };
}

/// The slot, filled for one hand-off and emptied when it ends, a panic
/// included.
struct Slot;

impl Slot {
    fn fill(handoff: Handoff) -> Self {
        HANDOFF.set(handoff);
        Self
    }

    fn take(&self) -> Handoff {
        HANDOFF.replace(Handoff::Empty)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        HANDOFF.set(Handoff::Empty);
    }
}

/// Hand an `UploadedFile` being read a placeholder: its field failed, and
/// the failure is recorded.
pub(super) fn offer_placeholder<'de, V: Visitor<'de>>(visitor: V) -> Result<V::Value, FieldError> {
    let _slot = Slot::fill(Handoff::Placeholder);
    visitor.visit_newtype_struct(Placeholder)
}

/// Hand an `UploadedFile` being read the file of the part at `path`, and
/// record what its validators said.
fn offer_file<'de, V: Visitor<'de>>(
    file: FilePart,
    path: &str,
    collector: &Collector,
    visitor: V,
) -> Result<V::Value, FieldError> {
    let slot = Slot::fill(Handoff::File(file));
    let read = visitor.visit_newtype_struct(Placeholder);
    match slot.take() {
        Handoff::Refused(message) => collector.record_message(path, message),
        Handoff::Failed(error) => {
            collector.fail(error);
            return Err(FieldError::recorded());
        }
        Handoff::Empty | Handoff::File(_) | Handoff::Placeholder => {}
    }
    read
}

/// An uploaded file is read from the part of a multipart body that carries
/// it, so a form request can hold the files of the form it reads, as a
/// Laravel form request does.
///
/// The file's validators `V` run on it once the body is read: a file they
/// refuse fails under its input name (`photos.1`) with their message.
/// Read from anything but a multipart body, such as JSON, an
/// `UploadedFile` fails.
impl<'de, V: UploadValidator> Deserialize<'de> for UploadedFile<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(UPLOADED_FILE, FileVisitor(PhantomData))
    }
}

/// Builds an [`UploadedFile`] from what the slot holds.
struct FileVisitor<V>(PhantomData<V>);

impl<'de, V: UploadValidator> Visitor<'de> for FileVisitor<V> {
    type Value = UploadedFile<V>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a file part of a multipart body")
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        IgnoredAny::deserialize(deserializer)?;
        HANDOFF.with_borrow_mut(|slot| match std::mem::replace(slot, Handoff::Empty) {
            Handoff::File(file) => checked(file, slot),
            Handoff::Placeholder => Ok(placeholder_file()),
            other => {
                *slot = other;
                Err(de::Error::custom(
                    "a file is read only from a multipart body",
                ))
            }
        })
    }
}

/// `file` as an `UploadedFile<V>` once `V` accepts it. A refusal is left in
/// `slot` for the read to record, and a placeholder stands in.
fn checked<V: UploadValidator, E: de::Error>(
    file: FilePart,
    slot: &mut Handoff,
) -> Result<UploadedFile<V>, E> {
    let validator = V::default();
    // The whole part is read, so the size check a validator runs per chunk
    // runs once, on the final size, before the check of the whole file.
    let verdict = validator
        .validate_chunk(&file.sniff, file.size)
        .and_then(|()| {
            validator.validate_received(&ReceivedPart::new(
                &file.sniff,
                file.size,
                file.content_type.as_deref(),
                file.image_size,
            ))
        });
    match verdict {
        Ok(()) => Ok(match file.backing {
            UploadedFileBacking::Memory(bytes) => UploadedFile::from_memory(
                bytes,
                file.file_name,
                file.content_type,
                file.inferred_extension,
            ),
            UploadedFileBacking::Disk(temp) => UploadedFile::from_disk(
                temp,
                file.size,
                file.file_name,
                file.content_type,
                file.inferred_extension,
            ),
        }),
        Err(FrameworkError::InvalidUpload(message)) => {
            *slot = Handoff::Refused(*message);
            Ok(placeholder_file())
        }
        Err(error) => {
            let message = error.to_string();
            *slot = Handoff::Failed(error);
            Err(E::custom(message))
        }
    }
}

/// The file that stands in for one a read could not use. A read that made
/// one has recorded a failure, so it never reaches a handler.
fn placeholder_file<V: UploadValidator>() -> UploadedFile<V> {
    UploadedFile::from_memory(Bytes::new(), None, None, None)
}

/// A type a part cannot fill: recorded under the part's path with the key
/// for the type, and a placeholder stands in.
macro_rules! failing {
    ($($method:ident($($arg:ident: $ty:ty),*) => $failure:ident;)*) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            self.collector.record(self.path, FieldFailure::$failure);
            de::Deserializer::$method(Placeholder, $($arg,)* visitor)
        }
    )*};
}

/// A part that carries a file, at the input name `path`.
pub(super) struct FormFile<'p, 'c> {
    pub(super) file: FilePart,
    pub(super) path: &'p str,
    pub(super) collector: &'c Collector,
}

impl<'de> de::Deserializer<'de> for FormFile<'_, '_> {
    type Error = FieldError;

    /// A file holds no text: a type that takes any value, such as a
    /// `serde_json::Value`, reads `null`.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if name == UPLOADED_FILE {
            offer_file(self.file, self.path, self.collector, visitor)
        } else {
            visitor.visit_newtype_struct(self)
        }
    }

    failing! {
        deserialize_bool() => String;
        deserialize_i8() => String;
        deserialize_i16() => String;
        deserialize_i32() => String;
        deserialize_i64() => String;
        deserialize_i128() => String;
        deserialize_u8() => String;
        deserialize_u16() => String;
        deserialize_u32() => String;
        deserialize_u64() => String;
        deserialize_u128() => String;
        deserialize_f32() => String;
        deserialize_f64() => String;
        deserialize_char() => String;
        deserialize_str() => String;
        deserialize_string() => String;
        deserialize_bytes() => String;
        deserialize_byte_buf() => String;
        deserialize_identifier() => String;
        deserialize_unit() => Format;
        deserialize_unit_struct(name: &'static str) => Format;
        deserialize_seq() => Format;
        deserialize_tuple(len: usize) => Format;
        deserialize_tuple_struct(name: &'static str, len: usize) => Format;
        deserialize_map() => Format;
        deserialize_struct(name: &'static str, fields: &'static [&'static str]) => Format;
        deserialize_enum(name: &'static str, variants: &'static [&'static str]) => Format;
    }
}

/// A text part whose bytes are not UTF-8, at the input name `path`. It
/// parses as no type, so it fails with the key for the type asked for, as
/// the multipart extractor files it.
pub(super) struct FormNotUtf8<'p, 'c> {
    pub(super) path: &'p str,
    pub(super) collector: &'c Collector,
}

impl<'de> de::Deserializer<'de> for FormNotUtf8<'_, '_> {
    type Error = FieldError;

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

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

    failing! {
        deserialize_any() => String;
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
        deserialize_bytes() => String;
        deserialize_byte_buf() => String;
        deserialize_identifier() => String;
        deserialize_unit() => Format;
        deserialize_unit_struct(name: &'static str) => Format;
        deserialize_seq() => Format;
        deserialize_tuple(len: usize) => Format;
        deserialize_tuple_struct(name: &'static str, len: usize) => Format;
        deserialize_map() => Format;
        deserialize_struct(name: &'static str, fields: &'static [&'static str]) => Format;
        deserialize_enum(name: &'static str, variants: &'static [&'static str]) => Format;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::upload::validators::MaxSize;

    fn part(bytes: &'static [u8]) -> FilePart {
        FilePart {
            backing: UploadedFileBacking::Memory(Bytes::from_static(bytes)),
            size: bytes.len() as u64,
            file_name: Some("a.txt".to_string()),
            content_type: None,
            inferred_extension: None,
            sniff: bytes.to_vec(),
            image_size: None,
        }
    }

    #[test]
    fn inp_a_file_reaches_only_the_read_that_offered_it() {
        let collector = Collector::default();
        let file: UploadedFile<MaxSize<8>> = offer_file(
            part(b"small"),
            "photo",
            &collector,
            FileVisitor(PhantomData),
        )
        .expect("the file");
        assert_eq!(file.size, 5);
        assert!(!collector.has_failures());

        // The slot is empty once the hand-off ends, so JSON finds no file.
        let from_json = serde_json::from_str::<UploadedFile>("\"a\"");
        assert!(from_json.is_err());

        // A refusal is recorded under the path and leaves the slot empty.
        let refused: UploadedFile<MaxSize<2>> = offer_file(
            part(b"too big"),
            "photos.1",
            &collector,
            FileVisitor(PhantomData),
        )
        .expect("a placeholder");
        assert_eq!(refused.size, 0);
        let errors = collector.into_errors();
        assert_eq!(
            errors.errors.keys().map(String::as_str).collect::<Vec<_>>(),
            ["photos.1"]
        );
        assert!(matches!(HANDOFF.replace(Handoff::Empty), Handoff::Empty));
    }
}
