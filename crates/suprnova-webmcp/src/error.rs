//! Why registering a tool, reading the page's server tools or calling one
//! did not happen.
//!
//! A call that reached the server and failed there is not an error: it is a
//! [`ToolResult`](crate::ToolResult) with `isError: true`, so an agent can
//! read what went wrong. These errors are for what never became a call.

use std::fmt;

/// What stopped a registration, a read of the page's server tools or a
/// call before it ran.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebMcpError {
    /// WebMCP cannot be reached from where this code runs.
    #[error("WebMCP is not available: {0}")]
    Unsupported(Unsupported),
    /// The tool name breaks the AGENT-001 grammar, which a browser and an
    /// agent both rely on to tell tools apart.
    #[error(
        "the tool name `{name}` is not valid: a tool name has 1 to 128 characters, each an ASCII \
         letter, a digit, `_`, `-` or `.`"
    )]
    InvalidName {
        /// The name as given.
        name: String,
    },
    /// The tool has no description, and an agent chooses tools by theirs.
    #[error("the tool `{name}` has no description")]
    MissingDescription {
        /// The tool's name.
        name: String,
    },
    /// The tool's title is blank, which would show an agent an empty label.
    #[error("the tool `{name}` has a blank title")]
    BlankTitle {
        /// The tool's name.
        name: String,
    },
    /// The tool's input type is not a struct, so its schema is not an
    /// object and WebMCP cannot pass it named arguments.
    #[error("the tool `{name}`: its input must be a struct, whose schema is an object")]
    InputNotObject {
        /// The tool's name.
        name: String,
    },
    /// The browser refused the registration, most often because a tool of
    /// that name is already registered on the page.
    #[error("the browser refused to register the tool `{name}`: {reason}")]
    Rejected {
        /// The tool's name.
        name: String,
        /// What the browser said.
        reason: String,
    },
    /// The page's server tools element is not what the server writes, so
    /// none of its tools are trusted.
    #[error("the page's server tools are not valid: {0}")]
    InvalidPage(String),
    /// The page offers no server tool of that name.
    #[error("the page offers no server tool named `{0}`")]
    UnknownTool(String),
    /// A tool result could not be built from the value given.
    #[error("the tool result could not be built: {0}")]
    Output(String),
}

/// Why WebMCP cannot be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsupported {
    /// The crate was built for a target that is not a browser, so it has
    /// no page to register a tool with or call from.
    Target,
    /// The code runs where there is no window or no document, such as a
    /// worker.
    NoDocument,
    /// The browser does not expose `document.modelContext.registerTool`.
    NoModelContext,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Target => "this build does not target a browser",
            Self::NoDocument => "there is no window or document here",
            Self::NoModelContext => "the browser does not expose document.modelContext",
        })
    }
}
