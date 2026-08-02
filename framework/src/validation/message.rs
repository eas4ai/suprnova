//! Structured validation messages — the seam between rule objects and
//! the localization module.
//!
//! A [`ValidationMessage`] carries three things: a stable message key
//! (`validation-min`), the arguments the message needs (`min: 3`), and a
//! pre-rendered English fallback. Translation happens once, at the
//! serialization boundary (`ValidationErrors::to_json`), never inside
//! rules — so rules stay pure and the `localization` feature can be
//! compiled out without touching them.

use serde_json::Value;
use std::borrow::Cow;
use std::fmt;

/// Ordered argument map handed to the translator. Values are
/// `serde_json::Value` because Fluent arguments are strings and numbers,
/// JSON covers both, and the `validator` crate's error params already
/// use this type.
pub type TranslateArgs = crate::indexmap::IndexMap<String, Value>;

/// A validation failure message: key + args + English fallback, plus an
/// optional context prefix.
///
/// Keyless messages (built by the `From<String>` / `From<&str>` impls)
/// skip translation entirely and render their text as-is — which is what
/// keeps user-written custom rules returning `Err("...".into())`
/// compiling and behaving unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationMessage {
    /// Catalog key (`validation-min`), or `""` for keyless messages.
    pub key: Cow<'static, str>,
    /// Arguments the catalog message interpolates.
    pub args: TranslateArgs,
    /// English text rendered when translation is unavailable: feature
    /// off, key missing from every catalog, or keyless message.
    pub fallback: String,
    /// Caller-supplied context (`FrameworkError::context("registration")`),
    /// rendered as `"<prefix>: <message>"` *after* translation. Kept
    /// beside the key rather than folded into the text so adding context
    /// never costs the message its translation.
    pub prefix: Option<String>,
}

impl ValidationMessage {
    /// Start a keyed message. Chain [`Self::arg`] and finish with
    /// [`Self::fallback`].
    pub fn keyed(key: impl Into<Cow<'static, str>>) -> Self {
        Self {
            key: key.into(),
            args: TranslateArgs::new(),
            fallback: String::new(),
            prefix: None,
        }
    }

    /// Attach one named argument.
    pub fn arg(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.args.insert(name.into(), value.into());
        self
    }

    /// Set the English fallback text and finish the builder.
    pub fn fallback(mut self, text: impl Into<String>) -> Self {
        self.fallback = text.into();
        self
    }

    /// Prepend context, keeping the key and arguments intact.
    ///
    /// Repeated calls nest outermost-first — `.prefix("a").prefix("b")`
    /// renders `"b: a: <message>"`, matching how
    /// [`FrameworkError::context`](crate::FrameworkError::context)
    /// chains on every other error variant.
    pub fn prefix(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.prefix = Some(match self.prefix {
            Some(existing) => format!("{text}: {existing}"),
            None => text,
        });
        self
    }

    /// True when this message can be looked up in a catalog.
    pub fn is_keyed(&self) -> bool {
        !self.key.is_empty()
    }
}

impl From<String> for ValidationMessage {
    fn from(text: String) -> Self {
        Self {
            key: Cow::Borrowed(""),
            args: TranslateArgs::new(),
            fallback: text,
            prefix: None,
        }
    }
}

impl From<&str> for ValidationMessage {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

impl fmt::Display for ValidationMessage {
    /// The English fallback, with any context prefix applied — the same
    /// text `ValidationErrors::to_json` produces when no translation is
    /// available.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prefix {
            Some(prefix) => write!(f, "{prefix}: {}", self.fallback),
            None => f.write_str(&self.fallback),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_builder_carries_key_args_and_fallback() {
        let m = ValidationMessage::keyed("validation-min")
            .arg("min", 3)
            .fallback("must be at least 3 characters");
        assert!(m.is_keyed());
        assert_eq!(m.key, "validation-min");
        assert_eq!(m.args.get("min"), Some(&serde_json::json!(3)));
        assert_eq!(m.to_string(), "must be at least 3 characters");
    }

    #[test]
    fn from_str_is_keyless_and_displays_verbatim() {
        let m: ValidationMessage = "custom failure".into();
        assert!(!m.is_keyed());
        assert!(m.prefix.is_none());
        assert_eq!(m.to_string(), "custom failure");
    }

    #[test]
    fn prefix_keeps_the_key_and_nests_outermost_first() {
        let m = ValidationMessage::keyed("validation-required")
            .fallback("required")
            .prefix("registration")
            .prefix("signup");
        // The key survives contexting — that is the whole point.
        assert_eq!(m.key, "validation-required");
        assert_eq!(m.prefix.as_deref(), Some("signup: registration"));
        // Display matches the pre-localization flattened string exactly.
        assert_eq!(m.to_string(), "signup: registration: required");
    }
}
