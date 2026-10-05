//! Fetching a library's files at a release tag (REG-009, REG-026), from a
//! forge over HTTPS, from a tree on disk, or from the embedded shipped
//! library, behind one trait so every other step is the same for each.
//!
//! Fetching only moves bytes: nothing fetched is parsed as anything but
//! data, and nothing runs. The HTTPS client sends no credentials, uses no
//! proxy, follows a redirect only within the origin and the repository or
//! library path it was asked for, and refuses a body over 2 MiB or a
//! response that takes more than 30 seconds.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::address::{Host, LibraryAddress, LibraryKind, is_loopback_host};
use super::library::{parse_library_json, strict_json};
use super::{RegistryError, Result};

/// The largest response a fetch accepts (REG-009).
pub const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

/// How long a fetch may take (REG-009).
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How many redirects one fetch follows, each within its origin and path.
const MAX_REDIRECTS: usize = 5;

/// How many pages of tags a forge listing is read for. A hostile forge that
/// answers every page full would otherwise keep a fetch going forever.
const MAX_TAG_PAGES: usize = 100;

/// The commit a tag resolved to; every file of one plan comes from it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Commit(pub String);

/// Where a library's bytes come from.
pub trait Fetcher {
    /// The release versions the library lists, from its `v<semver>` tags,
    /// pre-releases excluded.
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>>;

    /// The commit the tag `v<version>` names.
    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit>;

    /// One file of the library at that commit, by path from the library
    /// root (`library.json`, `components/<name>/manifest.json`, ...).
    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>>;
}

/// Fetches raw files from a forge over HTTPS (REG-009), with the default
/// limits. [`HttpsClient`] is the same fetcher with its limits chosen.
#[derive(Debug, Default)]
pub struct HttpsFetcher;

impl Fetcher for HttpsFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        HttpsClient::default().versions(library)
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        HttpsClient::default().resolve(library, version)
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        HttpsClient::default().file(library, commit, path)
    }
}

/// The HTTPS fetcher with its timeout and, for tests, a loopback server
/// standing in for the forges.
#[derive(Debug, Clone)]
pub struct HttpsClient {
    timeout: Duration,
    forge_base: Option<String>,
}

impl Default for HttpsClient {
    fn default() -> Self {
        HttpsClient {
            timeout: TIMEOUT,
            forge_base: None,
        }
    }
}

/// One GET the client may make, with the path prefix a redirect must stay
/// under on the same origin.
struct Request {
    url: String,
    scope: String,
}

impl HttpsClient {
    /// A client that gives each fetch at most `timeout`.
    pub fn new(timeout: Duration) -> Self {
        HttpsClient {
            timeout,
            forge_base: None,
        }
    }

    /// Routes every forge request to a loopback server instead: a request
    /// for `https://<forge-host>/<path>` goes to `<base>/<forge-host>/<path>`.
    /// Only a loopback base is accepted, so this can stand a test server in
    /// for a forge and never send a library's requests anywhere else.
    pub fn with_forge_base(mut self, base: &str) -> Result<Self> {
        let parsed = reqwest::Url::parse(base)
            .map_err(|error| RegistryError::Invalid(format!("`{base}` is not a URL: {error}")))?;
        let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
        if !is_loopback_host(&host) {
            return Err(RegistryError::Invalid(format!(
                "a forge stand-in must be on a loopback host, not `{host}`"
            )));
        }
        self.forge_base = Some(base.trim_end_matches('/').to_owned());
        Ok(self)
    }

    fn forge_url(&self, origin: &str, path: &str) -> Request {
        match &self.forge_base {
            Some(base) => {
                let host = origin.trim_start_matches("https://");
                Request {
                    url: format!("{base}/{host}{path}"),
                    scope: String::new(),
                }
            }
            None => Request {
                url: format!("{origin}{path}"),
                scope: String::new(),
            },
        }
    }

    fn scoped(&self, origin: &str, path: &str, scope: &str) -> Request {
        let mut request = self.forge_url(origin, path);
        request.scope = match &self.forge_base {
            Some(_) => format!("/{}{scope}", origin.trim_start_matches("https://")),
            None => scope.to_owned(),
        };
        request
    }

    fn tags(&self, host: Host, owner: &str, library: &str) -> Result<Vec<(String, String)>> {
        let mut tags = Vec::new();
        for page in 1..=MAX_TAG_PAGES {
            let (request, per_page) = match host {
                Host::GitHub => (
                    self.scoped(
                        "https://api.github.com",
                        &format!("/repos/{owner}/{library}/tags?per_page=100&page={page}"),
                        &format!("/repos/{owner}/{library}/"),
                    ),
                    100,
                ),
                Host::GitLab => (
                    self.scoped(
                        "https://gitlab.com",
                        &format!(
                            "/api/v4/projects/{owner}%2F{library}/repository/tags?per_page=100&page={page}"
                        ),
                        &format!("/api/v4/projects/{owner}%2F{library}/"),
                    ),
                    100,
                ),
                Host::Codeberg => (
                    self.scoped(
                        "https://codeberg.org",
                        &format!("/api/v1/repos/{owner}/{library}/tags?limit=50&page={page}"),
                        &format!("/api/v1/repos/{owner}/{library}/"),
                    ),
                    50,
                ),
            };
            let body = self.get(&request)?;
            let value = strict_json(&body)?;
            let entries = value.as_array().ok_or_else(|| {
                RegistryError::Network(format!(
                    "{} answered its tag listing with something other than a list",
                    host.domain()
                ))
            })?;
            for entry in entries {
                let name = entry.get("name").and_then(serde_json::Value::as_str);
                let commit = entry.get("commit").and_then(|commit| {
                    commit
                        .get("sha")
                        .or_else(|| commit.get("id"))
                        .and_then(serde_json::Value::as_str)
                });
                if let (Some(name), Some(commit)) = (name, commit) {
                    tags.push((name.to_owned(), commit.to_owned()));
                }
            }
            if entries.len() < per_page {
                return Ok(tags);
            }
        }
        Ok(tags)
    }

    fn raw_request(
        &self,
        host: Host,
        owner: &str,
        library: &str,
        commit: &Commit,
        path: &str,
    ) -> Request {
        let sha = &commit.0;
        match host {
            Host::GitHub => self.scoped(
                "https://raw.githubusercontent.com",
                &format!("/{owner}/{library}/{sha}/{path}"),
                &format!("/{owner}/{library}/{sha}/"),
            ),
            Host::GitLab => self.scoped(
                "https://gitlab.com",
                &format!("/{owner}/{library}/-/raw/{sha}/{path}"),
                &format!("/{owner}/{library}/-/raw/{sha}/"),
            ),
            Host::Codeberg => self.scoped(
                "https://codeberg.org",
                &format!("/{owner}/{library}/raw/commit/{sha}/{path}"),
                &format!("/{owner}/{library}/raw/commit/{sha}/"),
            ),
        }
    }

    fn url_request(base: &str, path: &str) -> Result<Request> {
        let parsed = reqwest::Url::parse(base)
            .map_err(|error| RegistryError::Invalid(format!("`{base}` is not a URL: {error}")))?;
        let base_path = parsed.path().trim_end_matches('/');
        Ok(Request {
            url: format!("{base}/{path}"),
            scope: format!("{base_path}/"),
        })
    }

    /// One GET: HTTPS, or HTTP on a loopback host; no credentials and no
    /// proxy; a redirect only to the same origin under `scope`; at most
    /// [`MAX_RESPONSE_BYTES`]; at most the client's timeout.
    fn get(&self, request: &Request) -> Result<Vec<u8>> {
        let timeout = self.timeout;
        let url = request.url.clone();
        let scope = request.scope.clone();
        std::thread::scope(|threads| {
            threads
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            RegistryError::Io(format!("cannot start the HTTP client: {error}"))
                        })?;
                    runtime.block_on(async move {
                        match tokio::time::timeout(timeout, fetch(&url, &scope, timeout)).await {
                            Ok(result) => result,
                            Err(_) => Err(timed_out(&url, timeout)),
                        }
                    })
                })
                .join()
                .unwrap_or_else(|_| {
                    Err(RegistryError::Network(
                        "the HTTP client stopped unexpectedly".to_owned(),
                    ))
                })
        })
    }
}

fn timed_out(url: &str, timeout: Duration) -> RegistryError {
    RegistryError::Network(format!(
        "{url} took more than {} seconds; refusing it",
        timeout.as_secs_f64()
    ))
}

async fn fetch(url: &str, scope: &str, timeout: Duration) -> Result<Vec<u8>> {
    // The CLI builds `ring` already; installing it as the process's rustls
    // provider keeps a second TLS crypto library out of the build. It is
    // already installed after the first fetch, which is not an error.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(timeout)
        .connect_timeout(timeout)
        .user_agent(concat!("suprnova-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            RegistryError::Network(format!("cannot build the HTTP client: {error}"))
        })?;
    let mut current = checked_url(url)?;
    let origin = origin_of(&current);
    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|error| request_error(current.as_str(), &error, timeout))?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    RegistryError::Network(format!("{current} redirected with no usable Location"))
                })?;
            let next = current.join(location).map_err(|error| {
                RegistryError::Network(format!(
                    "{current} redirected to `{location}`, which is not a URL: {error}"
                ))
            })?;
            let same_origin = origin_of(&next) == origin;
            if !same_origin
                || !next.path().starts_with(scope)
                || !next.username().is_empty()
                || next.password().is_some()
            {
                return Err(RegistryError::Network(format!(
                    "{current} redirected to {next}; a redirect to another origin or another repository path is refused. If the library moved, add it from its new address: {next}"
                )));
            }
            current = next;
            continue;
        }
        if !status.is_success() {
            return Err(RegistryError::Network(format!(
                "{current} answered {status}"
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
        {
            return Err(too_large(current.as_str()));
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| request_error(current.as_str(), &error, timeout))?
        {
            if body.len() as u64 + chunk.len() as u64 > MAX_RESPONSE_BYTES {
                return Err(too_large(current.as_str()));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(body);
    }
    Err(RegistryError::Network(format!(
        "{url} redirected more than {MAX_REDIRECTS} times"
    )))
}

fn too_large(url: &str) -> RegistryError {
    RegistryError::Network(format!(
        "{url} is over the {MAX_RESPONSE_BYTES} byte limit; refusing it"
    ))
}

fn request_error(url: &str, error: &reqwest::Error, timeout: Duration) -> RegistryError {
    if error.is_timeout() {
        return timed_out(url, timeout);
    }
    RegistryError::Network(format!("cannot fetch {url}: {error}"))
}

/// Accepts an https URL, or an http one on a loopback host, with no user.
fn checked_url(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| RegistryError::Invalid(format!("`{url}` is not a URL: {error}")))?;
    // The URL type writes an IPv6 host in brackets, as a source address
    // does, so `[::1]` compares the same in both.
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed = match parsed.scheme() {
        "https" => true,
        "http" => is_loopback_host(&host),
        _ => false,
    };
    if !allowed {
        return Err(RegistryError::Invalid(format!(
            "`{url}` is not HTTPS; live:add fetches over HTTPS only, except from a loopback host"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(RegistryError::Invalid(format!(
            "`{url}` carries credentials; live:add sends none"
        )));
    }
    Ok(parsed)
}

fn origin_of(url: &reqwest::Url) -> (String, String, Option<u16>) {
    (
        url.scheme().to_owned(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default(),
    )
}

fn valid_commit(sha: &str) -> bool {
    (sha.len() == 40 || sha.len() == 64)
        && sha
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Refuses a library path that is not `/`-separated closed segments: the
/// paths fetched are built by the CLI from validated names, and this keeps
/// them from ever carrying a traversal.
fn checked_path(path: &str) -> Result<Vec<&str>> {
    let segments: Vec<&str> = path.split('/').collect();
    let valid = segments.iter().all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('.')
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    });
    if valid {
        Ok(segments)
    } else {
        Err(RegistryError::Invalid(format!(
            "`{path}` is not a path inside a library tree"
        )))
    }
}

impl Fetcher for HttpsClient {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        match library.kind()? {
            LibraryKind::Repository {
                host,
                owner,
                library: name,
            } => {
                let mut versions: Vec<semver::Version> = self
                    .tags(host, &owner, &name)?
                    .into_iter()
                    .filter_map(|(tag, _)| {
                        tag.strip_prefix('v')
                            .and_then(|version| semver::Version::parse(version).ok())
                    })
                    .filter(|version| version.pre.is_empty())
                    .collect();
                versions.sort();
                versions.dedup();
                Ok(versions)
            }
            LibraryKind::Url { base } => {
                let bytes = self.get(&Self::url_request(&base, "library.json")?)?;
                Ok(vec![parse_library_json(&bytes)?.version])
            }
            LibraryKind::Shipped | LibraryKind::Path { .. } => Err(RegistryError::Invalid(
                format!("{library} is not fetched over HTTPS"),
            )),
        }
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        match library.kind()? {
            LibraryKind::Repository {
                host,
                owner,
                library: name,
            } => {
                let tag = format!("v{version}");
                let commits: Vec<String> = self
                    .tags(host, &owner, &name)?
                    .into_iter()
                    .filter(|(name, _)| *name == tag)
                    .map(|(_, commit)| commit)
                    .collect();
                match commits.as_slice() {
                    [commit] if valid_commit(commit) => Ok(Commit(commit.clone())),
                    [commit] => Err(RegistryError::Network(format!(
                        "{library} tag {tag} names `{commit}`, which is not a commit id"
                    ))),
                    [] => Err(RegistryError::Network(format!(
                        "{library} has no tag {tag}"
                    ))),
                    _ => Err(RegistryError::Network(format!(
                        "{library} lists the tag {tag} more than once"
                    ))),
                }
            }
            // A URL base has no commits: its one version is the one its
            // library.json names, which the plan checks it fetched.
            LibraryKind::Url { .. } => Ok(Commit(String::new())),
            LibraryKind::Shipped | LibraryKind::Path { .. } => Err(RegistryError::Invalid(
                format!("{library} is not fetched over HTTPS"),
            )),
        }
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        checked_path(path)?;
        match library.kind()? {
            LibraryKind::Repository {
                host,
                owner,
                library: name,
            } => {
                if !valid_commit(&commit.0) {
                    return Err(RegistryError::Invalid(format!(
                        "`{}` is not a commit id",
                        commit.0
                    )));
                }
                self.get(&self.raw_request(host, &owner, &name, commit, path))
            }
            LibraryKind::Url { base } => self.get(&Self::url_request(&base, path)?),
            LibraryKind::Shipped | LibraryKind::Path { .. } => Err(RegistryError::Invalid(
                format!("{library} is not fetched over HTTPS"),
            )),
        }
    }
}

/// Reads a library tree on disk, the author's own (REG-008 path sources);
/// its one version is the one `library.json` names.
#[derive(Debug)]
pub struct PathFetcher {
    /// The library root.
    pub root: std::path::PathBuf,
}

impl Fetcher for PathFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        let bytes = self.file(library, &Commit(String::new()), "library.json")?;
        Ok(vec![parse_library_json(&bytes)?.version])
    }

    fn resolve(&self, _library: &LibraryAddress, _version: &semver::Version) -> Result<Commit> {
        Ok(Commit(String::new()))
    }

    fn file(&self, _library: &LibraryAddress, _commit: &Commit, path: &str) -> Result<Vec<u8>> {
        read_regular_file(&self.root, path, MAX_RESPONSE_BYTES)
    }
}

/// Reads one file of a tree on disk, refusing anything but a regular file
/// reached through real directories (UI-023). A link would install whatever
/// it points at, a key or a configuration file included. Each entry is
/// judged without following it, then the file is read through one handle;
/// on Unix that handle must be the very file judged, so a link swapped in
/// between is refused rather than followed.
pub fn read_regular_file(root: &Path, path: &str, limit: u64) -> Result<Vec<u8>> {
    let segments = checked_path(path)?;
    let mut current = root.to_path_buf();
    let cannot_read = |at: &Path, error: std::io::Error| {
        RegistryError::Io(format!("cannot read {}: {error}", at.display()))
    };
    let (file, directories) = segments
        .split_last()
        .ok_or_else(|| RegistryError::Invalid("an empty path".to_owned()))?;
    for directory in directories {
        current.push(directory);
        let entry =
            std::fs::symlink_metadata(&current).map_err(|error| cannot_read(&current, error))?;
        if entry.file_type().is_symlink() || !entry.is_dir() {
            return Err(RegistryError::Invalid(format!(
                "{} is not a directory of the library tree; a component's files must be regular files in its directory",
                current.display()
            )));
        }
    }
    current.push(file);
    let entry =
        std::fs::symlink_metadata(&current).map_err(|error| cannot_read(&current, error))?;
    if entry.file_type().is_symlink() {
        return Err(RegistryError::Invalid(format!(
            "{} is a symbolic link; a component's files must be regular files in its directory",
            current.display()
        )));
    }
    if !entry.is_file() {
        return Err(RegistryError::Invalid(format!(
            "{} is not a regular file",
            current.display()
        )));
    }
    let handle = std::fs::File::open(&current).map_err(|error| cannot_read(&current, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let opened = handle
            .metadata()
            .map_err(|error| cannot_read(&current, error))?;
        if (opened.dev(), opened.ino()) != (entry.dev(), entry.ino()) {
            return Err(RegistryError::Invalid(format!(
                "{} changed while it was read; run the command again",
                current.display()
            )));
        }
    }
    let mut bytes = Vec::new();
    handle
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| cannot_read(&current, error))?;
    if bytes.len() as u64 > limit {
        return Err(RegistryError::Invalid(format!(
            "{} is over the {limit} byte limit",
            current.display()
        )));
    }
    Ok(bytes)
}

/// One component of the shipped library, embedded at build time.
pub struct EmbeddedComponent {
    /// The component directory name, which is also the `live:add` argument.
    pub directory: &'static str,
    /// The manifest's bytes.
    pub manifest: &'static str,
    /// Every file beside the manifest, by name.
    pub files: &'static [(&'static str, &'static str)],
}

include!(concat!(env!("OUT_DIR"), "/live_components.rs"));

/// The shipped library, embedded in this binary: one version, the CLI's,
/// and no `library.json`, since the shipped library is exempt from the
/// hash and the signature (REG-016).
#[derive(Debug, Default)]
pub struct EmbeddedFetcher;

/// The version the shipped library is recorded at: the CLI's.
pub fn shipped_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

impl EmbeddedFetcher {
    /// The directory names of every shipped component.
    pub fn components(&self) -> Vec<&'static str> {
        COMPONENTS
            .iter()
            .map(|component| component.directory)
            .collect()
    }
}

impl Fetcher for EmbeddedFetcher {
    fn versions(&self, _library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        Ok(vec![shipped_version()])
    }

    fn resolve(&self, _library: &LibraryAddress, _version: &semver::Version) -> Result<Commit> {
        Ok(Commit(String::new()))
    }

    fn file(&self, _library: &LibraryAddress, _commit: &Commit, path: &str) -> Result<Vec<u8>> {
        let segments = checked_path(path)?;
        let not_shipped =
            || RegistryError::Invalid(format!("`{path}` is not a file of the shipped library"));
        let ["components", directory, file] = segments.as_slice() else {
            return Err(not_shipped());
        };
        let component = COMPONENTS
            .iter()
            .find(|component| component.directory == *directory)
            .ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "`{directory}` is not a shipped library component; shipped components: {}",
                    self.components().join(", ")
                ))
            })?;
        if *file == "manifest.json" {
            return Ok(component.manifest.as_bytes().to_vec());
        }
        component
            .files
            .iter()
            .find(|(name, _)| name == file)
            .map(|(_, content)| content.as_bytes().to_vec())
            .ok_or_else(not_shipped)
    }
}

/// The fetcher `live:add` uses: a path address reads its tree on disk,
/// the shipped library reads the binary, and every other library is
/// fetched over HTTPS.
#[derive(Debug, Default)]
pub struct SourceFetcher {
    https: HttpsClient,
}

impl SourceFetcher {
    /// A fetcher whose HTTPS requests use `client`.
    pub fn new(client: HttpsClient) -> Self {
        SourceFetcher { https: client }
    }
}

impl Fetcher for SourceFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        match library.kind()? {
            LibraryKind::Shipped => EmbeddedFetcher.versions(library),
            LibraryKind::Path { root } => PathFetcher { root }.versions(library),
            _ => self.https.versions(library),
        }
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        match library.kind()? {
            LibraryKind::Shipped => EmbeddedFetcher.resolve(library, version),
            LibraryKind::Path { root } => PathFetcher { root }.resolve(library, version),
            _ => self.https.resolve(library, version),
        }
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        match library.kind()? {
            LibraryKind::Shipped => EmbeddedFetcher.file(library, commit, path),
            LibraryKind::Path { root } => PathFetcher { root }.file(library, commit, path),
            _ => self.https.file(library, commit, path),
        }
    }
}

/// A library held in memory, for tests: versions, each with its files by
/// path.
#[derive(Debug, Default)]
pub struct FakeFetcher {
    libraries: BTreeMap<LibraryAddress, BTreeMap<semver::Version, BTreeMap<String, Vec<u8>>>>,
}

impl FakeFetcher {
    /// Adds one version of a library.
    pub fn add_version(
        &mut self,
        library: LibraryAddress,
        version: semver::Version,
        files: BTreeMap<String, Vec<u8>>,
    ) {
        self.libraries
            .entry(library)
            .or_default()
            .insert(version, files);
    }
}

impl Fetcher for FakeFetcher {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
        Ok(self
            .libraries
            .get(library)
            .map(|versions| {
                versions
                    .keys()
                    .filter(|v| v.pre.is_empty())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
        self.libraries
            .get(library)
            .and_then(|versions| versions.get(version))
            .map(|_| Commit(format!("fake-{library}-{version}")))
            .ok_or_else(|| RegistryError::Network(format!("{library} has no tag v{version}")))
    }

    fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
        let versions = self
            .libraries
            .get(library)
            .ok_or_else(|| RegistryError::Network(format!("{library} is unknown")))?;
        let (_, files) = versions
            .iter()
            .find(|(version, _)| Commit(format!("fake-{library}-{version}")) == *commit)
            .ok_or_else(|| {
                RegistryError::Network(format!("{library} has no commit {}", commit.0))
            })?;
        files
            .get(path)
            .cloned()
            .ok_or_else(|| RegistryError::Network(format!("{library}@{} has no {path}", commit.0)))
    }
}

/// The path of a component file in a library tree.
pub fn component_path(directory: &str, file: &str) -> String {
    format!("components/{directory}/{file}")
}

/// Where an absolute library root sits, for a path address.
pub fn path_root(library: &LibraryAddress) -> Option<PathBuf> {
    match library.kind() {
        Ok(LibraryKind::Path { root }) => Some(root),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{EmbeddedFetcher, FakeFetcher, Fetcher, checked_path, valid_commit};
    use crate::registry::address::LibraryAddress;

    #[test]
    fn the_fake_fetcher_lists_release_versions_and_serves_files_at_their_commit() {
        let mut fetcher = FakeFetcher::default();
        let library = LibraryAddress("github.com/acme/acme-ui".to_owned());
        fetcher.add_version(
            library.clone(),
            semver::Version::new(1, 0, 0),
            BTreeMap::from([("library.json".to_owned(), b"{}".to_vec())]),
        );
        fetcher.add_version(
            library.clone(),
            semver::Version::parse("2.0.0-rc.1").expect("semver"),
            BTreeMap::new(),
        );
        assert_eq!(
            fetcher.versions(&library).expect("versions"),
            vec![semver::Version::new(1, 0, 0)]
        );
        let commit = fetcher
            .resolve(&library, &semver::Version::new(1, 0, 0))
            .expect("commit");
        assert_eq!(
            fetcher
                .file(&library, &commit, "library.json")
                .expect("file"),
            b"{}".to_vec()
        );
        assert!(fetcher.file(&library, &commit, "missing").is_err());
    }

    #[test]
    fn library_paths_and_commits_are_closed() {
        assert!(checked_path("components/widget/widget.html").is_ok());
        for hostile in ["", "/x", "a//b", "../x", "components/.x", "a\\b", "a b"] {
            assert!(checked_path(hostile).is_err(), "{hostile}");
        }
        assert!(valid_commit(&"a".repeat(40)));
        assert!(!valid_commit(&"A".repeat(40)));
        assert!(!valid_commit("main"));
    }

    #[test]
    fn the_embedded_fetcher_serves_shipped_components_only() {
        let library = LibraryAddress("suprnova".to_owned());
        let commit = EmbeddedFetcher
            .resolve(&library, &super::shipped_version())
            .expect("commit");
        assert!(
            EmbeddedFetcher
                .file(&library, &commit, "components/field/manifest.json")
                .is_ok()
        );
        assert!(
            EmbeddedFetcher
                .file(&library, &commit, "components/carousel/manifest.json")
                .is_err()
        );
        assert!(
            EmbeddedFetcher
                .file(&library, &commit, "library.json")
                .is_err()
        );
    }
}
