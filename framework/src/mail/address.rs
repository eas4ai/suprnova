//! Address and Attachment types for outgoing mail.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A mail address - email plus an optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    /// The raw email address (e.g. `"alice@example.org"`).
    pub email: String,
    /// Optional display name shown in `"Name <email>"` form.
    pub name: Option<String>,
}

impl Address {
    /// Build an address from an email-only string.
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            name: None,
        }
    }
    /// Attach a display name to the address (rendered as `"Name <email>"`).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl From<&str> for Address {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Address {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<(String, String)> for Address {
    /// `(name, email)`
    fn from(t: (String, String)) -> Self {
        Self {
            name: Some(t.0),
            email: t.1,
        }
    }
}

impl From<(&str, &str)> for Address {
    /// `(name, email)`
    fn from(t: (&str, &str)) -> Self {
        Self {
            name: Some(t.0.into()),
            email: t.1.into(),
        }
    }
}

/// Human-readable `Name <email>` for logs and test output. This is not a wire
/// format: the name is written unquoted, so a comma in it would read as a
/// second address. Transports serialize addresses through `mail::wire`,
/// which quotes the name.
impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(n) => write!(f, "{n} <{}>", self.email),
            None => write!(f, "{}", self.email),
        }
    }
}

/// A binary attachment included on an outgoing mail message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// Filename surfaced to the recipient.
    pub filename: String,
    /// Raw attachment bytes.
    pub content: Vec<u8>,
    /// MIME content type (e.g. `"application/pdf"`).
    pub content_type: String,
}

impl Attachment {
    /// Build an [`Attachment`] from raw bytes plus a filename and
    /// content-type label.
    pub fn new(
        filename: impl Into<String>,
        content: Vec<u8>,
        content_type: impl Into<String>,
    ) -> Self {
        Self {
            filename: filename.into(),
            content,
            content_type: content_type.into(),
        }
    }

    /// Whether `other` has the same name, bytes and content type. Mirrors
    /// Laravel's `Attachment::isEquivalent`, which compares the data, the
    /// `as` name and the `mime` type.
    ///
    /// A test asserts with it that a message carries a file, not merely a
    /// file of that name: see
    /// [`OutgoingMessage::has_equivalent_attachment`](crate::mail::OutgoingMessage::has_equivalent_attachment).
    pub fn is_equivalent(&self, other: &Attachment) -> bool {
        self.is_equivalent_with(other, &other.filename, &other.content_type)
    }

    /// Whether `other` is equivalent once `name` and `content_type` stand in
    /// for its own: the bytes must match, and this attachment must carry
    /// that name and type. Mirrors the `$options` argument of Laravel's
    /// `Attachment::isEquivalent`, whose `as` and `mime` replace the other
    /// attachment's.
    pub fn is_equivalent_with(&self, other: &Attachment, name: &str, content_type: &str) -> bool {
        self.filename == name && self.content_type == content_type && self.content == other.content
    }

    /// Whether `other` has the same name and bytes, whatever its content
    /// type: what makes a later attachment a duplicate of an earlier one,
    /// as Laravel's `attachData` keeps one attachment per name and data.
    fn duplicates(&self, other: &Attachment) -> bool {
        self.filename == other.filename && self.content == other.content
    }
}

/// Append `attachment` unless one with the same name and bytes is already
/// in `list`; the earlier one is kept.
pub(crate) fn push_attachment(list: &mut Vec<Attachment>, attachment: Attachment) {
    if !list.iter().any(|kept| kept.duplicates(&attachment)) {
        list.push(attachment);
    }
}

/// Merge `first` and `then` into one list with one attachment for each
/// distinct name and bytes, in order, the first of each kept. Both the send
/// path and the queue worker merge a mailable's `attachments()` with the
/// builder's through this, so the two paths cannot disagree.
pub(crate) fn merge_attachments(
    first: Vec<Attachment>,
    then: impl IntoIterator<Item = Attachment>,
) -> Vec<Attachment> {
    let mut merged = Vec::with_capacity(first.len());
    for attachment in first.into_iter().chain(then) {
        push_attachment(&mut merged, attachment);
    }
    merged
}
