//! `Stringable`: a value that chains the [`Str`] helpers, as Laravel's
//! fluent `Str::of($value)` does.

use std::fmt;

use super::Str;
use crate::content::{ContentResult, MarkdownRenderer};
use crate::crypto::{Crypt, CryptPurpose};
use crate::error::FrameworkError;

/// A string value with the [`Str`] helpers as chainable methods, made by
/// [`Str::of`]. It chains the subset of Laravel's `Stringable` that `Str`
/// keeps, so `Str::of(title).slug("-")` reads as Laravel's
/// `Str::of($title)->slug('-')` and answers what `Str::slug(title, "-")`
/// answers.
///
/// ```
/// use suprnova::Str;
///
/// let slug = Str::of("Child").plural(2).slug("-");
/// assert_eq!(slug.to_string(), "children");
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Stringable(String);

impl Stringable {
    /// Wrap `value`. [`Str::of`] reads better at a call site.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Take the value out.
    pub fn into_string(self) -> String {
        self.0
    }

    /// [`Str::slug`] of the value.
    pub fn slug(self, separator: &str) -> Self {
        Self(Str::slug(&self.0, separator))
    }

    /// [`Str::slug_in`] of the value.
    pub fn slug_in(self, separator: &str, language: &str) -> Self {
        Self(Str::slug_in(&self.0, separator, language))
    }

    /// [`Str::mask`] of the value.
    pub fn mask(self, character: char, index: isize, length: Option<isize>) -> Self {
        Self(Str::mask(&self.0, character, index, length))
    }

    /// [`Str::limit`] of the value.
    pub fn limit(self, limit: usize, end: &str) -> Self {
        Self(Str::limit(&self.0, limit, end))
    }

    /// [`Str::words`] of the value.
    pub fn words(self, words: usize, end: &str) -> Self {
        Self(Str::words(&self.0, words, end))
    }

    /// [`Str::limit_words`] of the value.
    pub fn limit_words(self, limit: usize, end: &str) -> Self {
        Self(Str::limit_words(&self.0, limit, end))
    }

    /// [`Str::excerpt`] of the value: `None` when the phrase is not there,
    /// as Laravel's `Stringable::excerpt` answers `null`.
    pub fn excerpt(self, phrase: &str, radius: usize, omission: &str) -> Option<Self> {
        Str::excerpt(&self.0, phrase, radius, omission).map(Self)
    }

    /// [`Str::plural`] of the value.
    pub fn plural(self, count: i64) -> Self {
        Self(Str::plural(&self.0, count))
    }

    /// [`Str::plural_with_count`] of the value.
    pub fn plural_with_count(self, count: i64) -> Self {
        Self(Str::plural_with_count(&self.0, count))
    }

    /// [`Str::plural_studly`] of the value.
    pub fn plural_studly(self, count: i64) -> Self {
        Self(Str::plural_studly(&self.0, count))
    }

    /// [`Str::plural_pascal`] of the value.
    pub fn plural_pascal(self, count: i64) -> Self {
        Self(Str::plural_pascal(&self.0, count))
    }

    /// [`Str::singular`] of the value.
    pub fn singular(self) -> Self {
        Self(Str::singular(&self.0))
    }

    /// [`Str::markdown`] of the value: the value rendered as a Markdown
    /// document by `renderer`.
    pub fn markdown(self, renderer: &MarkdownRenderer) -> ContentResult<Self> {
        Str::markdown(&self.0, renderer).map(Self)
    }

    /// [`Str::inline_markdown`] of the value: its inline Markdown rendered
    /// by `renderer`, with no paragraph around it.
    pub fn inline_markdown(self, renderer: &MarkdownRenderer) -> ContentResult<Self> {
        Str::inline_markdown(&self.0, renderer).map(Self)
    }

    /// The value encrypted under `purpose` by
    /// [`Crypt::encrypt_string`]. Laravel's `Stringable::encrypt` takes no
    /// purpose; here every encryption names one, so a value sealed for
    /// one use cannot be opened as another.
    ///
    /// # Errors
    ///
    /// [`CryptPurpose::Cookie`] is refused, because a cookie value is bound
    /// to its cookie's name: use [`encrypt_for`](Self::encrypt_for) with
    /// the name. Otherwise, when `Crypt` is not initialized or the cipher
    /// refuses.
    pub fn encrypt(self, purpose: CryptPurpose) -> Result<Self, FrameworkError> {
        Crypt::encrypt_string(purpose, &self.0).map(Self)
    }

    /// The value encrypted under `purpose` and `context` by
    /// [`Crypt::encrypt_string_for`]: only
    /// [`decrypt_for`](Self::decrypt_for) with the same context opens it.
    ///
    /// # Errors
    ///
    /// When `Crypt` is not initialized or the cipher refuses.
    pub fn encrypt_for(self, purpose: CryptPurpose, context: &str) -> Result<Self, FrameworkError> {
        Crypt::encrypt_string_for(purpose, context, &self.0).map(Self)
    }

    /// The value decrypted under `purpose` by [`Crypt::decrypt_string`].
    ///
    /// # Errors
    ///
    /// When the value was encrypted under another purpose, is not a value
    /// `Crypt` wrote, `Crypt` is not initialized, or `purpose` is
    /// [`CryptPurpose::Cookie`].
    pub fn decrypt(self, purpose: CryptPurpose) -> Result<Self, FrameworkError> {
        Crypt::decrypt_string(purpose, &self.0).map(Self)
    }

    /// The value decrypted under `purpose` and `context` by
    /// [`Crypt::decrypt_string_for`].
    ///
    /// # Errors
    ///
    /// When the value was encrypted under another purpose or context, is
    /// not a value `Crypt` wrote, or `Crypt` is not initialized.
    pub fn decrypt_for(self, purpose: CryptPurpose, context: &str) -> Result<Self, FrameworkError> {
        Crypt::decrypt_string_for(purpose, context, &self.0).map(Self)
    }
}

impl fmt::Display for Stringable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Stringable {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Stringable {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<Stringable> for String {
    fn from(value: Stringable) -> Self {
        value.0
    }
}

impl AsRef<str> for Stringable {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Stringable {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Stringable {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Stringable {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}
