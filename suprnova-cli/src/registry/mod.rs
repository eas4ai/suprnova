//! Third-party Live component libraries: addressing, fetching, verifying,
//! scanning, planning and installing a component, and authoring a library
//! (`docs/spec/component-registries.md`, REG-001 to REG-033).
//!
//! A library is one repository tree: `library.json` at the root and one
//! directory per component under `components/`, each holding a manifest,
//! the files it names and an Ed25519 signature over the component's
//! verification hash. `live:add` fetches a component at a release tag,
//! verifies the signature against the key the project pinned, scans every
//! file as data without running any of it, shows the plan, and only then
//! writes into the application and records what arrived in `suprnova.toml`.
//!
//! The module tree is the interface contract the implementation lanes build
//! against. A function that returns [`RegistryError::NotBuilt`] is a lane 0
//! placeholder; none reaches `main`.

pub mod address;
pub mod author_key;
pub mod fetch;
pub mod install;
pub mod library;
pub mod plan;
pub mod project;
pub mod registration;
pub mod registry_commands;
pub mod scaffold;
pub mod scan;
pub mod signing;
pub mod statement;

use std::fmt;

/// An effect a component reaches through Suprnova's API (REG-006). The
/// developer approves each one a component uses before it installs; the
/// plan names them in lowercase, which `--allow <capability>` takes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// Reads or writes a database.
    Database,
    /// Opens a connection to another host.
    Network,
    /// Reads or writes files, a storage disk included.
    Files,
    /// Sends mail.
    Mail,
    /// Pushes to or works a queue.
    Queue,
    /// Reads or writes a cache.
    Cache,
    /// Reads or writes the session.
    Session,
    /// Reads the environment or the configuration.
    Environment,
    /// Starts a process.
    Process,
}

impl Capability {
    /// Every capability, in the order the plan lists them.
    pub const ALL: [Capability; 9] = [
        Capability::Database,
        Capability::Network,
        Capability::Files,
        Capability::Mail,
        Capability::Queue,
        Capability::Cache,
        Capability::Session,
        Capability::Environment,
        Capability::Process,
    ];

    /// The lowercase name the plan shows and `--allow` takes.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Database => "database",
            Capability::Network => "network",
            Capability::Files => "files",
            Capability::Mail => "mail",
            Capability::Queue => "queue",
            Capability::Cache => "cache",
            Capability::Session => "session",
            Capability::Environment => "environment",
            Capability::Process => "process",
        }
    }

    /// The capability a lowercase name denotes.
    pub fn parse(name: &str) -> Option<Self> {
        Capability::ALL
            .into_iter()
            .find(|capability| capability.name() == name)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why the registry refused or failed. Every refusal names what it refused
/// so the developer can act on it; nothing is written when one is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A source, address, manifest or record is malformed.
    Invalid(String),
    /// Validation refused the component: every finding, each naming its
    /// check, file and line (REG-022).
    Refused(Vec<scan::Finding>),
    /// The developer declined the plan, a capability or a key.
    Declined(String),
    /// A file could not be read or written.
    Io(String),
    /// A fetch failed or was refused (REG-009).
    Network(String),
    /// A lane 0 placeholder: the named part is not built yet.
    NotBuilt(&'static str),
}

/// Every message is escaped as it is written, so no error a registry
/// operation returns can carry a terminal control sequence to whoever
/// prints it; the line breaks the CLI laid out stay.
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Invalid(message)
            | RegistryError::Declined(message)
            | RegistryError::Io(message)
            | RegistryError::Network(message) => f.write_str(&printable_lines(message)),
            RegistryError::Refused(findings) => {
                for (index, finding) in findings.iter().enumerate() {
                    if index > 0 {
                        f.write_str("\n")?;
                    }
                    f.write_str(&printable(&finding.to_string()))?;
                }
                Ok(())
            }
            RegistryError::NotBuilt(part) => write!(f, "{part} is not built yet"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// The registry's result type.
pub type Result<T> = std::result::Result<T, RegistryError>;

/// Text safe to print on a terminal: every control character, the line
/// break included, and every Unicode control that reorders text, written as
/// its escape (`\u{1b}`). Names, addresses, keys and messages that come from
/// a fetched library, a project file or a server go through this before
/// they reach the terminal, so none of them can move the cursor, set the
/// clipboard (OSC 52), retitle the window, or start a line of its own.
pub fn printable(text: &str) -> String {
    escape(text, false)
}

/// [`printable`] for a message the CLI laid out in lines: the line breaks
/// stay, every other control is escaped.
pub fn printable_lines(text: &str) -> String {
    escape(text, true)
}

fn escape(text: &str, keep_lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let reorders = matches!(
            character,
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        );
        if (character == '\n' && keep_lines) || !(character.is_control() || reorders) {
            out.push(character);
        } else {
            out.extend(character.escape_unicode());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Capability, printable, printable_lines};

    #[test]
    fn printable_text_escapes_every_control_and_reordering_character() {
        assert_eq!(
            printable("a\u{1b}]52;c;eA==\u{7}b"),
            "a\\u{1b}]52;c;eA==\\u{7}b"
        );
        assert_eq!(printable("one\ntwo"), "one\\u{a}two");
        assert_eq!(printable_lines("one\ntwo\r"), "one\ntwo\\u{d}");
        assert_eq!(printable("x\u{202e}y"), "x\\u{202e}y");
        assert_eq!(printable("caf\u{e9} \u{2713}"), "caf\u{e9} \u{2713}");
    }

    #[test]
    fn every_capability_round_trips_through_its_name() {
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.name()), Some(capability));
            assert_eq!(capability.to_string(), capability.name());
        }
        assert_eq!(Capability::parse("Files"), None);
    }
}
