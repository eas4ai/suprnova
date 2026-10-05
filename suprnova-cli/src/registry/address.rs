//! The four sources `live:add` accepts and the one address each resolves to
//! (REG-008).

use std::fmt;
use std::path::PathBuf;

use super::{RegistryError, Result};

/// A forge that hosts a library repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// `github.com`, the default when a source names no host.
    GitHub,
    /// `gitlab.com`.
    GitLab,
    /// `codeberg.org`.
    Codeberg,
}

impl Host {
    /// The host name as it appears in a library address.
    pub fn domain(self) -> &'static str {
        match self {
            Host::GitHub => "github.com",
            Host::GitLab => "gitlab.com",
            Host::Codeberg => "codeberg.org",
        }
    }

    /// The host a domain names, lowercase.
    pub fn parse(domain: &str) -> Option<Self> {
        match domain.to_ascii_lowercase().as_str() {
            "github.com" => Some(Host::GitHub),
            "gitlab.com" => Some(Host::GitLab),
            "codeberg.org" => Some(Host::Codeberg),
            _ => None,
        }
    }
}

/// What the developer typed, resolved to its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A component of the shipped library, by bare name.
    Shipped {
        /// The component directory name.
        component: String,
    },
    /// `[<host>/]<owner>/<library>/<component>[@<version>]`.
    Repository {
        /// The forge.
        host: Host,
        /// The owner segment as typed.
        owner: String,
        /// The repository segment as typed.
        library: String,
        /// The component directory name.
        component: String,
        /// The release version, when the source names one.
        version: Option<semver::Version>,
    },
    /// An `https://` URL of a component directory; the library is two
    /// segments up.
    Url {
        /// The library base: scheme, lowercase host, port when not the
        /// default, and the path two segments above the component.
        library_base: String,
        /// The component directory name.
        component: String,
    },
    /// A component directory, or its `manifest.json`, inside a library tree
    /// on disk.
    Path {
        /// The library root, absolute, symbolic links resolved.
        library_root: PathBuf,
        /// The component directory name.
        component: String,
    },
}

/// A library's canonical address: `<host>/<owner>/<library>` lowercase for
/// a repository, the URL base for a URL, the absolute root for a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LibraryAddress(pub String);

impl fmt::Display for LibraryAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A component's canonical address: its library address, `/`, and its
/// directory name. It never carries a version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentAddress {
    /// The library.
    pub library: LibraryAddress,
    /// The component directory name.
    pub component: String,
}

impl fmt::Display for ComponentAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.library, self.component)
    }
}

/// The library address the shipped library records under.
pub const SHIPPED_LIBRARY: &str = "suprnova";

/// Parses what the developer typed into a [`Source`]. `--manifest <file>`
/// callers pass the file path with `./` prepended when it has no prefix.
pub fn parse(spec: &str) -> Result<Source> {
    let _ = spec;
    Err(RegistryError::NotBuilt("address parsing"))
}

impl Source {
    /// The library address this source resolves to.
    pub fn library_address(&self) -> Result<LibraryAddress> {
        Err(RegistryError::NotBuilt("library addresses"))
    }

    /// The component address this source resolves to.
    pub fn component_address(&self) -> Result<ComponentAddress> {
        Err(RegistryError::NotBuilt("component addresses"))
    }
}

/// Whether an owner or library segment is `<owner>`-shaped: ASCII letters,
/// digits, `-`, `_` and `.`, not starting with `.`.
pub fn valid_repository_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::valid_repository_segment;

    #[test]
    fn repository_segments_are_closed() {
        for ok in ["acme", "acme-ui", "Acme_UI", "a.b"] {
            assert!(valid_repository_segment(ok), "{ok}");
        }
        for hostile in ["", ".git", "a/b", "a b", "a%2fb", "../x"] {
            assert!(!valid_repository_segment(hostile), "{hostile}");
        }
    }
}
