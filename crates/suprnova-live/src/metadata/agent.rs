//! What a Live action declares as an agent tool, and what a tool result may
//! show an agent of the component's state (AGENT-012, AGENT-013).
//!
//! An agent reads a tool's name, description and schema as instructions, so
//! all of them are fixed when the component is built: the name and
//! description come from `#[tool]`, the input schema from the action's
//! argument codecs ([`ActionArgumentSchema::json_schema`]). A tool result
//! shows only the fields the component marks agent-visible, copied from the
//! instanced state the successor snapshot signs.

use std::collections::BTreeMap;

use crate::action::ActionArgumentSchema;
use crate::canonical::CanonicalValue;

use super::{ComponentMetadata, MetadataError, MetadataErrorKind};

/// The longest base name a Live tool may have. A browser registers the tool
/// as `{base}.{instance}`, with an instance key of at most 63 bytes, and a
/// tool name has at most 128 characters.
pub const MAX_TOOL_BASE_BYTES: usize = 64;

/// The longest description a tool may carry, in bytes.
const MAX_DESCRIPTION_BYTES: usize = 4_096;

/// The longest display title a tool may carry, in bytes.
const MAX_TITLE_BYTES: usize = 256;

/// The longest instance key a tool component's island may have, in bytes:
/// a base of [`MAX_TOOL_BASE_BYTES`], a `.` and the key fit in 128.
pub const MAX_TOOL_INSTANCE_BYTES: usize = 63;

/// What `#[tool]` declares on one Live action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionToolMetadata {
    name: String,
    description: String,
    title: Option<String>,
    read_only: bool,
    untrusted_content: bool,
    consequential: bool,
    browser: bool,
    headless: bool,
}

impl ActionToolMetadata {
    /// A tool named `name`, the base its browser and MCP names start from,
    /// described for an agent by `description`.
    ///
    /// # Errors
    ///
    /// [`MetadataErrorKind::InvalidActionMetadata`] when the name is empty,
    /// longer than [`MAX_TOOL_BASE_BYTES`] or outside ASCII letters, digits,
    /// `_`, `-` and `.`, or the description is blank or longer than 4 KiB.
    pub fn new(name: &str, description: &str) -> Result<Self, MetadataError> {
        let name_valid = !name.is_empty()
            && name.len() <= MAX_TOOL_BASE_BYTES
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
        if !name_valid || description.trim().is_empty() || description.len() > MAX_DESCRIPTION_BYTES
        {
            return Err(invalid());
        }
        Ok(Self {
            name: name.to_owned(),
            description: description.to_owned(),
            title: None,
            read_only: false,
            untrusted_content: false,
            consequential: false,
            browser: false,
            headless: false,
        })
    }

    /// The same tool with a display title.
    ///
    /// # Errors
    ///
    /// [`MetadataErrorKind::InvalidActionMetadata`] for a blank title or one
    /// longer than 256 bytes.
    pub fn with_title(mut self, title: &str) -> Result<Self, MetadataError> {
        if title.trim().is_empty() || title.len() > MAX_TITLE_BYTES {
            return Err(invalid());
        }
        self.title = Some(title.to_owned());
        Ok(self)
    }

    /// The same tool with the hints WebMCP reads: it changes nothing, its
    /// result may carry untrusted content, and it may destroy or overwrite
    /// data, which a browser agent treats as consequential.
    #[must_use]
    pub const fn with_hints(
        mut self,
        read_only: bool,
        untrusted_content: bool,
        consequential: bool,
    ) -> Self {
        self.read_only = read_only;
        self.untrusted_content = untrusted_content;
        self.consequential = consequential;
        self
    }

    /// The same tool, registered in the browser when `browser` holds: it
    /// accepts a public or a signed-in caller, the callers a page has.
    #[must_use]
    pub const fn for_browser(mut self, browser: bool) -> Self {
        self.browser = browser;
        self
    }

    /// The same tool, callable over the MCP endpoint with no browser when
    /// `headless` holds.
    #[must_use]
    pub const fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// The base name: the MCP name of a headless tool, and the start of
    /// every browser name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the tool does, for an agent to read.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The display title, when declared.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether the tool changes nothing.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }

    /// Whether the tool's result may carry untrusted content.
    #[must_use]
    pub const fn untrusted_content(&self) -> bool {
        self.untrusted_content
    }

    /// Whether the tool may destroy or overwrite data.
    #[must_use]
    pub const fn consequential(&self) -> bool {
        self.consequential
    }

    /// Whether the browser registers the tool.
    #[must_use]
    pub const fn browser(&self) -> bool {
        self.browser
    }

    /// Whether the MCP endpoint offers the tool with no browser.
    #[must_use]
    pub const fn is_headless(&self) -> bool {
        self.headless
    }

    /// The browser name of the tool on the island `instance`:
    /// `{base}.{instance}`, or `None` when `instance` is outside the
    /// instance grammar ([`is_tool_instance_key`]).
    #[must_use]
    pub fn browser_name(&self, instance: &str) -> Option<String> {
        is_tool_instance_key(instance).then(|| format!("{}.{instance}", self.name))
    }

    /// Whether `arguments` can be described to an agent.
    pub(crate) fn fits(arguments: &ActionArgumentSchema) -> bool {
        arguments
            .fields()
            .all(|field| field.codec().has_json_schema())
    }
}

/// Whether `key` may name the island of a tool component: 1 to 63 bytes of
/// ASCII letters, digits, `_` and `-`.
///
/// A tool name allows no `:`, and a `.` would make the browser name
/// ambiguous between the base and the instance, so the key grammar is
/// narrower than the one a document key or a `live:key` otherwise has.
#[must_use]
pub fn is_tool_instance_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_TOOL_INSTANCE_BYTES
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// The fields a tool result shows an agent: each agent-visible field of
/// `metadata`, copied from the instanced `state` in the encoding the
/// snapshot signs, which is the one the browser runtime reads. Every other
/// field is left out, whatever `state` holds.
#[must_use]
pub fn agent_fields(metadata: &ComponentMetadata, state: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(values) = state else {
        return CanonicalValue::Object(BTreeMap::new());
    };
    CanonicalValue::Object(
        metadata
            .fields()
            .iter()
            .filter(|field| field.agent_visible())
            .filter_map(|field| {
                values
                    .get(field.name().as_str())
                    .map(|value| (field.name().as_str().to_owned(), value.clone()))
            })
            .collect(),
    )
}

fn invalid() -> MetadataError {
    MetadataError::new(MetadataErrorKind::InvalidActionMetadata)
}
