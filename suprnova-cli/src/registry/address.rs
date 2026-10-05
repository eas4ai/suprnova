//! The four sources `live:add` accepts and the one address each resolves to
//! (REG-008).
//!
//! Every source is resolved to one library address and one component
//! address before anything is fetched, so two spellings of one component
//! (a different case, a trailing slash, the directory or its
//! `manifest.json`) always record one address. Each part is checked against
//! a closed character set: what the developer typed is hostile input.

use std::fmt;
use std::path::{Path, PathBuf};

use super::library::valid_directory_name;
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

/// What kind of library an address names, read from the address alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryKind {
    /// The library embedded in this binary.
    Shipped,
    /// A repository on a forge, fetched at a tag.
    Repository {
        /// The forge.
        host: Host,
        /// The owner segment, lowercase.
        owner: String,
        /// The repository segment, lowercase.
        library: String,
    },
    /// A tree served under an HTTPS URL base (or HTTP on loopback).
    Url {
        /// The base, without a trailing slash.
        base: String,
    },
    /// A tree on disk, at an absolute path.
    Path {
        /// The library root.
        root: PathBuf,
    },
}

impl LibraryAddress {
    /// The library this address names: the shipped library, a repository, a
    /// URL base or a path, refusing any address that is none of them in
    /// its canonical spelling.
    pub fn kind(&self) -> Result<LibraryKind> {
        let text = self.0.as_str();
        if text == SHIPPED_LIBRARY {
            return Ok(LibraryKind::Shipped);
        }
        if text.starts_with("https://") || text.starts_with("http://") {
            let parsed = parse_url_base(text)?;
            if parsed != text {
                return Err(RegistryError::Invalid(format!(
                    "`{text}` is not a canonical library address; it is written `{parsed}`"
                )));
            }
            return Ok(LibraryKind::Url {
                base: text.to_owned(),
            });
        }
        if Path::new(text).is_absolute() {
            return Ok(LibraryKind::Path {
                root: PathBuf::from(text),
            });
        }
        let segments: Vec<&str> = text.split('/').collect();
        if let [domain, owner, library] = segments.as_slice()
            && let Some(host) = Host::parse(domain)
            && host.domain() == *domain
            && valid_repository_segment(owner)
            && valid_repository_segment(library)
            && owner.to_ascii_lowercase() == *owner
            && library.to_ascii_lowercase() == *library
        {
            return Ok(LibraryKind::Repository {
                host,
                owner: (*owner).to_owned(),
                library: (*library).to_owned(),
            });
        }
        Err(RegistryError::Invalid(format!(
            "`{text}` is not a library address: expected `<host>/<owner>/<library>` in lowercase, an `https://` URL base, or an absolute path"
        )))
    }

    /// Whether this is the shipped library's address.
    pub fn is_shipped(&self) -> bool {
        self.0 == SHIPPED_LIBRARY
    }
}

/// Parses a library address as `library.json`'s `source` writes it: the
/// canonical address of a repository or a URL base (REG-025). A path is
/// never a published address, and the shipped library has no
/// `library.json`.
pub fn parse_published_address(text: &str) -> Result<LibraryAddress> {
    let address = LibraryAddress(text.to_owned());
    match address.kind()? {
        LibraryKind::Repository { .. } | LibraryKind::Url { .. } => Ok(address),
        LibraryKind::Shipped | LibraryKind::Path { .. } => Err(RegistryError::Invalid(format!(
            "`{text}` is not a published library address: `source` names a repository or an https:// URL base"
        ))),
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

impl ComponentAddress {
    /// Reads a component address as `suprnova.toml` keys it: a library
    /// address, `/`, and a directory name.
    pub fn parse(text: &str) -> Result<Self> {
        let (library, component) = text.rsplit_once('/').ok_or_else(|| {
            RegistryError::Invalid(format!("`{text}` is not a component address"))
        })?;
        if !valid_directory_name(component) {
            return Err(RegistryError::Invalid(format!(
                "`{text}` does not end in a component directory name"
            )));
        }
        let library = LibraryAddress(library.to_owned());
        library.kind()?;
        Ok(ComponentAddress {
            library,
            component: component.to_owned(),
        })
    }
}

/// The library address the shipped library records under.
pub const SHIPPED_LIBRARY: &str = "suprnova";

/// Parses what the developer typed into a [`Source`]. `--manifest <file>`
/// callers pass the file path with `./` prepended when it has no prefix.
pub fn parse(spec: &str) -> Result<Source> {
    if spec.is_empty() {
        return Err(RegistryError::Invalid(
            "name a component: a shipped name, <owner>/<library>/<component>, an https:// URL or a ./path"
                .to_owned(),
        ));
    }
    if spec.starts_with("./")
        || spec.starts_with("../")
        || spec.starts_with('/')
        || Path::new(spec).is_absolute()
    {
        return parse_path(Path::new(spec));
    }
    let lowered = spec.get(..8).map(str::to_ascii_lowercase);
    if lowered.as_deref() == Some("https://")
        || spec.get(..7).map(str::to_ascii_lowercase).as_deref() == Some("http://")
    {
        return parse_url(spec);
    }
    if spec.contains('/') || spec.contains('@') {
        return parse_repository(spec);
    }
    let component = spec.strip_prefix("suprnova.").unwrap_or(spec);
    if !valid_directory_name(component) {
        return Err(RegistryError::Invalid(format!(
            "`{spec}` is not a shipped library component: a component name is lowercase letters, digits and hyphens"
        )));
    }
    Ok(Source::Shipped {
        component: component.to_owned(),
    })
}

fn parse_repository(spec: &str) -> Result<Source> {
    let (path, version) = match spec.rsplit_once('@') {
        Some((path, version)) => {
            let parsed = semver::Version::parse(version).map_err(|error| {
                RegistryError::Invalid(format!(
                    "`@{version}` is not a semver version (it names the tag `v<version>`): {error}"
                ))
            })?;
            (path, Some(parsed))
        }
        None => (spec, None),
    };
    let segments: Vec<&str> = path.split('/').collect();
    let (host, owner, library, component) = match segments.as_slice() {
        [owner, library, component] => (Host::GitHub, *owner, *library, *component),
        [domain, owner, library, component] => {
            let host = Host::parse(domain).ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "`{domain}` is not a supported host: use github.com, gitlab.com or codeberg.org, or an https:// URL"
                ))
            })?;
            (host, *owner, *library, *component)
        }
        _ => {
            return Err(RegistryError::Invalid(format!(
                "`{spec}` is not `[<host>/]<owner>/<library>/<component>[@<version>]`"
            )));
        }
    };
    for (what, segment) in [("owner", owner), ("library", library)] {
        if !valid_repository_segment(segment) {
            return Err(RegistryError::Invalid(format!(
                "the {what} `{segment}` must be ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
            )));
        }
    }
    if !valid_directory_name(component) {
        return Err(RegistryError::Invalid(format!(
            "the component `{component}` must be 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen"
        )));
    }
    Ok(Source::Repository {
        host,
        owner: owner.to_owned(),
        library: library.to_owned(),
        component: component.to_owned(),
        version,
    })
}

fn parse_url(spec: &str) -> Result<Source> {
    let (base, mut segments) = split_url(spec)?;
    let component = segments.pop().ok_or_else(|| {
        RegistryError::Invalid(format!(
            "`{spec}` names no component directory: expected <base>/components/<component>"
        ))
    })?;
    if segments.pop().as_deref() != Some("components") {
        return Err(RegistryError::Invalid(format!(
            "`{spec}` is not a component directory of a library tree: expected <base>/components/<component>"
        )));
    }
    if !valid_directory_name(&component) {
        return Err(RegistryError::Invalid(format!(
            "the component `{component}` must be 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen"
        )));
    }
    let mut library_base = base;
    for segment in segments {
        library_base.push('/');
        library_base.push_str(&segment);
    }
    Ok(Source::Url {
        library_base,
        component,
    })
}

/// Reads an http(s) URL base into its canonical spelling: lowercase scheme
/// and host, the port only when it is not the default, the path segments,
/// no trailing slash.
fn parse_url_base(text: &str) -> Result<String> {
    let (mut base, segments) = split_url(text)?;
    for segment in segments {
        base.push('/');
        base.push_str(&segment);
    }
    Ok(base)
}

/// Splits a URL into `scheme://host[:port]` and its path segments, refusing
/// a user, a query, a fragment, a percent escape, an empty, `.` or `..`
/// segment, plain HTTP to any host but a loopback one, and any character
/// outside the closed sets.
fn split_url(text: &str) -> Result<(String, Vec<String>)> {
    if text
        .bytes()
        .any(|byte| matches!(byte, b'?' | b'#' | b'%' | b'@' | b'\\') || !byte.is_ascii_graphic())
    {
        return Err(RegistryError::Invalid(format!(
            "`{text}` may carry no user, query, fragment, percent escape or space"
        )));
    }
    let (scheme, rest) = text
        .split_once("://")
        .ok_or_else(|| RegistryError::Invalid(format!("`{text}` is not a URL")))?;
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, path),
        None => (rest, ""),
    };
    let (host, port) = split_authority(authority)?;
    let default_port = match scheme.as_str() {
        "https" => 443,
        "http" if is_loopback_host(&host) => 80,
        "http" => {
            return Err(RegistryError::Invalid(format!(
                "`{text}` is plain HTTP; live:add fetches over HTTPS only, except from a loopback host"
            )));
        }
        _ => {
            return Err(RegistryError::Invalid(format!(
                "`{text}` is not an https:// URL"
            )));
        }
    };
    let mut base = format!("{scheme}://{host}");
    if let Some(port) = port
        && port != default_port
    {
        base.push_str(&format!(":{port}"));
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let mut segments = Vec::new();
    if !path.is_empty() {
        for segment in path.split('/') {
            if !valid_repository_segment(segment) {
                return Err(RegistryError::Invalid(format!(
                    "`{text}` holds the path segment `{segment}`; a segment is ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
                )));
            }
            segments.push(segment.to_owned());
        }
    }
    Ok((base, segments))
}

fn split_authority(authority: &str) -> Result<(String, Option<u16>)> {
    let invalid = || RegistryError::Invalid(format!("`{authority}` is not a host and port"));
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (inside, after) = rest.split_once(']').ok_or_else(invalid)?;
        if !inside
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b':')
        {
            return Err(invalid());
        }
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or_else(invalid)?),
        };
        (format!("[{}]", inside.to_ascii_lowercase()), port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let valid_host = !host.is_empty()
            && host.split('.').all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            });
        if !valid_host {
            return Err(invalid());
        }
        (host.to_ascii_lowercase(), port)
    };
    let port = match port {
        None => None,
        Some(digits) => {
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            let port: u16 = digits.parse().map_err(|_| invalid())?;
            if port == 0 {
                return Err(invalid());
            }
            Some(port)
        }
    };
    Ok((host, port))
}

/// Whether a lowercase host is a loopback one: `localhost`, an IPv4
/// address in `127.0.0.0/8`, or `[::1]`. Only these may be fetched over
/// plain HTTP (REG-009).
pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host == "[::1]" {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|address| address.is_loopback())
}

fn parse_path(path: &Path) -> Result<Source> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        RegistryError::Io(format!("cannot resolve {}: {error}", path.display()))
    })?;
    let directory =
        if canonical.file_name().and_then(|name| name.to_str()) == Some("manifest.json") {
            canonical.parent().map(Path::to_path_buf).ok_or_else(|| {
                RegistryError::Invalid(format!("{} has no directory", path.display()))
            })?
        } else {
            canonical
        };
    if !directory.is_dir() {
        return Err(RegistryError::Invalid(format!(
            "{} is not a component directory or its manifest.json",
            path.display()
        )));
    }
    let component = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if !valid_directory_name(&component) {
        return Err(RegistryError::Invalid(format!(
            "the component directory `{component}` must be 1 to 64 lowercase letters, digits and hyphens, neither starting nor ending with a hyphen"
        )));
    }
    let components = directory
        .parent()
        .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("components"));
    let library_root = components
        .and_then(Path::parent)
        .ok_or_else(|| {
            RegistryError::Invalid(format!(
                "{} is not inside a library tree: a component directory sits under `components/` beside `library.json`",
                directory.display()
            ))
        })?
        .to_path_buf();
    if library_root.to_str().is_none() {
        return Err(RegistryError::Invalid(format!(
            "{} is not a UTF-8 path",
            library_root.display()
        )));
    }
    Ok(Source::Path {
        library_root,
        component,
    })
}

impl Source {
    /// The library address this source resolves to.
    pub fn library_address(&self) -> Result<LibraryAddress> {
        Ok(match self {
            Source::Shipped { .. } => LibraryAddress(SHIPPED_LIBRARY.to_owned()),
            Source::Repository {
                host,
                owner,
                library,
                ..
            } => LibraryAddress(format!(
                "{}/{}/{}",
                host.domain(),
                owner.to_ascii_lowercase(),
                library.to_ascii_lowercase()
            )),
            Source::Url { library_base, .. } => LibraryAddress(library_base.clone()),
            Source::Path { library_root, .. } => {
                LibraryAddress(library_root.to_str().map(str::to_owned).ok_or_else(|| {
                    RegistryError::Invalid(format!(
                        "{} is not a UTF-8 path",
                        library_root.display()
                    ))
                })?)
            }
        })
    }

    /// The component address this source resolves to.
    pub fn component_address(&self) -> Result<ComponentAddress> {
        let component = match self {
            Source::Shipped { component }
            | Source::Repository { component, .. }
            | Source::Url { component, .. }
            | Source::Path { component, .. } => component.clone(),
        };
        Ok(ComponentAddress {
            library: self.library_address()?,
            component,
        })
    }

    /// The version the source names, when it names one.
    pub fn version(&self) -> Option<&semver::Version> {
        match self {
            Source::Repository { version, .. } => version.as_ref(),
            _ => None,
        }
    }
}

/// One entry of a manifest's `dependencies` (REG-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependency {
    /// A bare name: a component of the shipped library.
    Shipped(String),
    /// `./<component>`: a component of the same library at the same version.
    Sibling(String),
    /// A full address, a repository or an https:// URL, with the version it
    /// names, if any.
    Address(Source),
}

/// Parses one `dependencies` entry: a bare shipped name, `./<component>`,
/// or a full repository or URL address. A path on disk is never a
/// dependency: a library cannot reach outside itself on its user's disk.
pub fn parse_dependency(text: &str) -> Result<Dependency> {
    if let Some(sibling) = text.strip_prefix("./") {
        if !valid_directory_name(sibling) {
            return Err(RegistryError::Invalid(format!(
                "dependency `{text}` does not name a component directory"
            )));
        }
        return Ok(Dependency::Sibling(sibling.to_owned()));
    }
    if !text.contains('/') && !text.contains('@') {
        if !valid_directory_name(text) {
            return Err(RegistryError::Invalid(format!(
                "dependency `{text}` is not a shipped component name"
            )));
        }
        return Ok(Dependency::Shipped(text.to_owned()));
    }
    if text.starts_with("../") || text.starts_with('/') || Path::new(text).is_absolute() {
        return Err(RegistryError::Invalid(format!(
            "dependency `{text}` is a path; a dependency is a shipped name, ./<component> or a full address"
        )));
    }
    match parse(text)? {
        source @ (Source::Repository { .. } | Source::Url { .. }) => {
            Ok(Dependency::Address(source))
        }
        _ => Err(RegistryError::Invalid(format!(
            "dependency `{text}` is not a full address"
        ))),
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
    use super::{
        Dependency, Host, LibraryAddress, LibraryKind, Source, parse, parse_dependency,
        valid_repository_segment,
    };

    #[test]
    fn repository_segments_are_closed() {
        for ok in ["acme", "acme-ui", "Acme_UI", "a.b"] {
            assert!(valid_repository_segment(ok), "{ok}");
        }
        for hostile in ["", ".git", "a/b", "a b", "a%2fb", "../x"] {
            assert!(!valid_repository_segment(hostile), "{hostile}");
        }
    }

    #[test]
    fn a_repository_source_defaults_to_github_and_lowercases_its_address() {
        let source = parse("Acme/Acme-UI/date-picker@1.2.0").expect("parses");
        assert_eq!(
            source.component_address().expect("address").to_string(),
            "github.com/acme/acme-ui/date-picker"
        );
        assert_eq!(source.version(), Some(&semver::Version::new(1, 2, 0)));
        let gitlab = parse("GitLab.com/acme/ui/widget").expect("parses");
        assert!(matches!(
            gitlab,
            Source::Repository {
                host: Host::GitLab,
                ..
            }
        ));
        for hostile in [
            "example.com/acme/ui/widget",
            "acme/ui",
            "acme/ui/Widget",
            "acme/.git/widget",
            "acme/ui/widget@v1.0.0",
            "a/b/c/d/e",
        ] {
            assert!(parse(hostile).is_err(), "{hostile}");
        }
    }

    #[test]
    fn a_url_source_resolves_two_segments_up_with_one_spelling() {
        let a = parse("HTTPS://Example.TEST:443/libs/acme-ui/components/widget/").expect("parses");
        let b = parse("https://example.test/libs/acme-ui/components/widget").expect("parses");
        assert_eq!(
            a.component_address().expect("a"),
            b.component_address().expect("b")
        );
        assert_eq!(
            b.library_address().expect("library").0,
            "https://example.test/libs/acme-ui"
        );
        let port = parse("https://example.test:8443/components/widget").expect("parses");
        assert_eq!(
            port.library_address().expect("library").0,
            "https://example.test:8443"
        );
        for hostile in [
            "http://example.test/components/widget",
            "https://user@example.test/components/widget",
            "https://example.test/components/widget?x=1",
            "https://example.test/components/widget#x",
            "https://example.test/a%2fb/components/widget",
            "https://example.test/./components/widget",
            "https://example.test/../components/widget",
            "https://example.test//components/widget",
            "https://example.test/lib/widget",
            "ftp://example.test/components/widget",
        ] {
            assert!(parse(hostile).is_err(), "{hostile}");
        }
        assert!(parse("http://127.0.0.1:8080/components/widget").is_ok());
        assert!(parse("http://[::1]:8080/components/widget").is_ok());
    }

    #[test]
    fn library_addresses_know_their_kind() {
        assert_eq!(
            LibraryAddress("suprnova".to_owned()).kind().expect("kind"),
            LibraryKind::Shipped
        );
        assert!(matches!(
            LibraryAddress("github.com/acme/acme-ui".to_owned()).kind(),
            Ok(LibraryKind::Repository { .. })
        ));
        assert!(
            LibraryAddress("github.com/Acme/acme-ui".to_owned())
                .kind()
                .is_err()
        );
        assert!(
            LibraryAddress("https://Example.test".to_owned())
                .kind()
                .is_err()
        );
        assert!(LibraryAddress("acme".to_owned()).kind().is_err());
    }

    #[test]
    fn dependencies_are_shipped_siblings_or_full_addresses() {
        assert_eq!(
            parse_dependency("button").expect("shipped"),
            Dependency::Shipped("button".to_owned())
        );
        assert_eq!(
            parse_dependency("./calendar").expect("sibling"),
            Dependency::Sibling("calendar".to_owned())
        );
        assert!(matches!(
            parse_dependency("acme/acme-ui/calendar@1.0.0"),
            Ok(Dependency::Address(_))
        ));
        for hostile in ["../x", "/etc", "./", "./A", "Button"] {
            assert!(parse_dependency(hostile).is_err(), "{hostile}");
        }
    }
}
