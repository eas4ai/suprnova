//! Resolving a source into a plan the developer confirms (REG-006, REG-007,
//! REG-010, REG-011, REG-012, REG-014): every component, its files and
//! their outcomes, every module and registration, every capability, and
//! the key to pin. Nothing is written until the plan is confirmed.
//!
//! Resolution fetches, verifies and scans the whole plan, dependencies
//! included, before it returns, so a write never starts on a plan with an
//! unverified or unscanned part (REG-029). For each third-party component
//! the order is fixed: the library's `source`, version, framework range,
//! namespace and key are checked, then the statement is built from the
//! bytes that arrived and its signature verified, and only after every
//! component verified is anything scanned (REG-023, REG-024).

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::address::{
    ComponentAddress, Dependency, LibraryAddress, LibraryKind, SHIPPED_LIBRARY, Source,
    parse_dependency,
};
use super::fetch::{Commit, EmbeddedFetcher, Fetcher, component_path, shipped_version};
use super::library::{
    ComponentManifest, FileKind, LibraryJson, MAX_FILE_BYTES, namespace_module, parse_library_json,
    parse_manifest, parse_shipped_library_json, parse_shipped_manifest, validate_file_name,
};
use super::project::{Approval, ComponentRecord, InstallRecord, LibraryRecord, ProjectFile};
use super::scan::{ComponentFiles, ScanContext, ScanReport};
use super::signing::{Fingerprint, PublicKey, Signature, verify, verify_handover};
use super::statement::{Digest, Statement};
use super::{Capability, RegistryError, Result};
use crate::secure_fs;

/// The most components one plan may hold (REG-010).
pub const MAX_PLAN_COMPONENTS: usize = 64;

/// The most bytes one plan may fetch.
pub const MAX_PLAN_BYTES: u64 = 64 * 1024 * 1024;

/// What installing a file would do (REG-012, REG-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// The path does not exist and is written.
    New,
    /// The path holds the same bytes.
    Unchanged,
    /// The path holds unedited bytes and is replaced.
    Replaced,
    /// The path holds edited bytes and is kept; `--force` replaces it.
    Kept,
    /// The path holds bytes no record vouches for and is kept.
    ChangedSinceRecord,
    /// The library dropped the file, the application never edited it, and
    /// the install removes it.
    Removed,
}

impl FileOutcome {
    /// How the plan and the report word it.
    pub fn describe(self) -> &'static str {
        match self {
            FileOutcome::New => "new",
            FileOutcome::Unchanged => "unchanged",
            FileOutcome::Replaced => "replaced",
            FileOutcome::Kept => "kept, edited locally (pass --force to replace)",
            FileOutcome::ChangedSinceRecord => {
                "kept, changed since the record: no install record shows it unedited (pass --force to replace)"
            }
            FileOutcome::Removed => "removed: the library dropped it",
        }
    }

    /// Whether the install writes the file.
    pub fn writes(self) -> bool {
        matches!(self, FileOutcome::New | FileOutcome::Replaced)
    }

    /// Whether the install keeps bytes the application has.
    pub fn keeps(self) -> bool {
        matches!(self, FileOutcome::Kept | FileOutcome::ChangedSinceRecord)
    }
}

/// One file of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// The manifest name.
    pub name: String,
    /// Where it lands, from the project root.
    pub destination: PathBuf,
    /// Its kind.
    pub kind: FileKind,
    /// What happens to it.
    pub outcome: FileOutcome,
    /// The digest of the bytes that arrived.
    pub digest: Digest,
    /// The bytes that arrived.
    pub bytes: Vec<u8>,
}

/// A file an earlier install wrote that this version no longer names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedFile {
    /// The file name the earlier version named.
    pub name: String,
    /// Where it sits, from the project root.
    pub destination: PathBuf,
    /// Its kind.
    pub kind: FileKind,
    /// Removed when unedited, else kept.
    pub outcome: FileOutcome,
}

/// One component of the plan, dependencies first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedComponent {
    /// The component's canonical address.
    pub address: ComponentAddress,
    /// The library as it arrived. For a shipped component, the embedded
    /// `library.json`: the namespace `suprnova`, the CLI's version and a
    /// key that is never used.
    pub library: LibraryJson,
    /// The library's version.
    pub version: semver::Version,
    /// The commit every file came from.
    pub commit: Commit,
    /// The manifest as it arrived.
    pub manifest: ComponentManifest,
    /// The bytes of `library.json` and `manifest.json` as they arrived.
    pub library_json_bytes: Vec<u8>,
    /// The manifest bytes as they arrived.
    pub manifest_bytes: Vec<u8>,
    /// The signature file as it arrived, absent for the shipped library.
    pub signature: Option<String>,
    /// The verification hash.
    pub hash: Digest,
    /// Every file.
    pub files: Vec<PlannedFile>,
    /// What the scan found.
    pub scan: ScanReport,
    /// Module declarations to add, as `(file, line)`.
    pub module_declarations: Vec<(PathBuf, String)>,
    /// Registrations to add, as full paths.
    pub registrations: Vec<String>,
    /// Registrations to remove, as full paths.
    pub unregistrations: Vec<String>,
    /// Files the recorded version installed that this one drops.
    pub dropped: Vec<DroppedFile>,
    /// What `--force` let through.
    pub forced: Vec<String>,
}

impl PlannedComponent {
    /// Whether this component is from the shipped library.
    pub fn is_shipped(&self) -> bool {
        self.address.library.is_shipped()
    }

    /// The directory its views, stylesheets, scripts, manifest and install
    /// record land in: `templates/<namespace>-ui/<directory>`.
    pub fn view_directory(&self) -> PathBuf {
        view_directory(&self.library.namespace, &self.address.component)
    }

    /// Where its manifest lands.
    pub fn manifest_destination(&self) -> PathBuf {
        self.view_directory().join("manifest.json")
    }
}

fn view_directory(namespace: &str, directory: &str) -> PathBuf {
    PathBuf::from("templates")
        .join(format!("{namespace}-ui"))
        .join(directory)
}

fn rust_directory(namespace: &str) -> PathBuf {
    PathBuf::from("src")
        .join("live")
        .join(namespace_module(namespace))
}

/// Where a named file lands, by its kind (REG-003).
pub fn destination(namespace: &str, directory: &str, file: &str, kind: FileKind) -> PathBuf {
    match kind {
        FileKind::Rust => rust_directory(namespace).join(file),
        FileKind::View | FileKind::Stylesheet | FileKind::Script => {
            view_directory(namespace, directory).join(file)
        }
    }
}

/// A key the plan would pin or re-pin (REG-024, REG-033).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPin {
    /// The library.
    pub library: LibraryAddress,
    /// The key to pin.
    pub key: PublicKey,
    /// The fingerprint of the key it replaces, when it does.
    pub replaces: Option<Fingerprint>,
}

/// What the developer asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Replace edited files and accept downgrades and moved tags.
    pub force: bool,
    /// Report and write nothing.
    pub dry_run: bool,
    /// Confirm the plan without a terminal; never approves a capability or
    /// pins a key.
    pub yes: bool,
    /// Capabilities approved on the command line.
    pub allow: BTreeSet<Capability>,
}

/// The whole plan, dependencies first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Every component, in install order.
    pub components: Vec<PlannedComponent>,
    /// The framework version the application depends on, as checked (REG-007).
    /// A plan of shipped components only makes no framework check and
    /// carries the CLI's version.
    pub framework_version: semver::Version,
    /// Keys the plan pins.
    pub pins: Vec<KeyPin>,
    /// The router call to report when a namespace's first component installs
    /// (REG-017).
    pub router_calls: Vec<String>,
}

impl Plan {
    /// Every capability the plan needs approved, over all its components.
    pub fn capabilities(&self) -> BTreeSet<Capability> {
        self.components
            .iter()
            .flat_map(|component| component.scan.capabilities.iter().copied())
            .collect()
    }

    /// Whether any component is from a library other than the shipped one.
    pub fn has_third_party(&self) -> bool {
        self.components
            .iter()
            .any(|component| !component.is_shipped())
    }

    /// The version each of a component's dependencies resolved to in this
    /// plan (REG-010).
    pub fn dependency_versions(
        &self,
        component: &PlannedComponent,
    ) -> Result<BTreeMap<ComponentAddress, semver::Version>> {
        let mut versions = BTreeMap::new();
        for dependency in &component.manifest.dependencies {
            let address = dependency_address(&component.address.library, dependency)?.0;
            let planned = self
                .components
                .iter()
                .find(|candidate| candidate.address == address)
                .ok_or_else(|| {
                    RegistryError::Invalid(format!(
                        "{} depends on {address}, which the plan does not hold",
                        component.address
                    ))
                })?;
            versions.insert(address, planned.version.clone());
        }
        Ok(versions)
    }
}

/// Scans one component as data (REG-022). The plan calls it after every
/// component verified, with what it knows beyond the files: the `register`
/// and `elements` of the manifest they arrived under and the views of the
/// components they depend on, which `live:check`'s view checks follow.
/// Tests stand a scanner of their own in.
pub trait Scanner {
    /// The scan report for one component's files in that context.
    fn scan(&self, component: &ComponentFiles<'_>, context: &ScanContext<'_>)
    -> Result<ScanReport>;
}

/// The scan `live:add` runs: Suprnova's allowlist over every file, the Live
/// components the Rust defines checked against the manifest's `register`
/// (REG-005, REG-030), and every element a script defines checked against
/// its `elements` (REG-004, REG-032).
#[derive(Debug, Default)]
pub struct AllowlistScanner;

impl Scanner for AllowlistScanner {
    fn scan(
        &self,
        component: &ComponentFiles<'_>,
        context: &ScanContext<'_>,
    ) -> Result<ScanReport> {
        super::scan::scan_component_in(component, context, super::scan::allowlist::embedded()?)
    }
}

/// The framework version an application uses and where it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkVersion {
    /// The `suprnova` version.
    pub version: semver::Version,
    /// `Cargo.lock`, or the `Cargo.toml` tag it came from.
    pub from: String,
}

/// The `suprnova` version the application's own package depends on: from
/// the nearest `Cargo.lock` in the application's directory or a parent of
/// it, which is where a workspace keeps its members' lock, or with no lock
/// from the `v<version>` tag of the git dependency in `Cargo.toml`, as the
/// scaffold writes it (REG-007). Reads both, writes neither.
pub fn framework_version(root: &Path) -> Result<FrameworkVersion> {
    let manifest_text = std::fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| RegistryError::Io(format!("cannot read Cargo.toml: {error}")))?;
    let manifest: toml::Table = toml::from_str(&manifest_text).map_err(|error| {
        RegistryError::Invalid(format!("Cargo.toml is not valid TOML: {error}"))
    })?;
    let package = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| RegistryError::Invalid("Cargo.toml names no [package]".to_owned()))?
        .to_owned();
    let lock_path = nearest_lock(root);
    let lock_origin = match &lock_path {
        Some(path) if path.parent() == Some(root) => "Cargo.lock".to_owned(),
        Some(path) => format!("the workspace's Cargo.lock at {}", path.display()),
        None => String::new(),
    };
    let lock_read = match &lock_path {
        Some(path) => std::fs::read_to_string(path).map(Some),
        None => Ok(None),
    };
    match lock_read {
        Ok(Some(lock_text)) => {
            let lock: toml::Table = toml::from_str(&lock_text).map_err(|error| {
                RegistryError::Invalid(format!("Cargo.lock is not valid TOML: {error}"))
            })?;
            let packages = lock
                .get("package")
                .and_then(toml::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let own = packages
                .iter()
                .filter(|entry| entry.get("name").and_then(toml::Value::as_str) == Some(&package))
                .find(|entry| entry.get("source").is_none())
                .ok_or_else(|| {
                    RegistryError::Invalid(format!("Cargo.lock holds no package `{package}`"))
                })?;
            let dependency = own
                .get("dependencies")
                .and_then(toml::Value::as_array)
                .and_then(|dependencies| {
                    dependencies
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .find(|entry| entry.split_whitespace().next() == Some("suprnova"))
                })
                .ok_or_else(|| {
                    RegistryError::Invalid(format!(
                        "`{package}` does not depend on suprnova in Cargo.lock"
                    ))
                })?;
            let version_text = match dependency.split_whitespace().nth(1) {
                Some(version) => version.to_owned(),
                None => {
                    let versions: Vec<&str> = packages
                        .iter()
                        .filter(|entry| {
                            entry.get("name").and_then(toml::Value::as_str) == Some("suprnova")
                        })
                        .filter_map(|entry| entry.get("version").and_then(toml::Value::as_str))
                        .collect();
                    match versions.as_slice() {
                        [version] => (*version).to_owned(),
                        _ => {
                            return Err(RegistryError::Invalid(
                                "Cargo.lock does not name one suprnova version".to_owned(),
                            ));
                        }
                    }
                }
            };
            let version = semver::Version::parse(&version_text).map_err(|error| {
                RegistryError::Invalid(format!(
                    "Cargo.lock's suprnova version `{version_text}` is not semver: {error}"
                ))
            })?;
            Ok(FrameworkVersion {
                version,
                from: lock_origin,
            })
        }
        Ok(None) => {
            let tag = manifest
                .get("dependencies")
                .and_then(|dependencies| dependencies.get("suprnova"))
                .and_then(|suprnova| suprnova.get("tag"))
                .and_then(toml::Value::as_str);
            let version = tag
                .and_then(|tag| tag.strip_prefix('v'))
                .and_then(|version| semver::Version::parse(version).ok())
                .ok_or_else(|| {
                    RegistryError::Invalid(
                        "this application has no Cargo.lock and its Cargo.toml names suprnova at no `v<version>` tag, so live:add cannot tell which framework it uses; run `cargo generate-lockfile`"
                            .to_owned(),
                    )
                })?;
            Ok(FrameworkVersion {
                version,
                from: format!("the Cargo.toml tag {}", tag.unwrap_or_default()),
            })
        }
        Err(error) => Err(RegistryError::Io(format!(
            "cannot read Cargo.lock: {error}"
        ))),
    }
}

/// The nearest `Cargo.lock` at `root` or above it: the application's own,
/// or the workspace's when the application is a member (REG-007). Nothing
/// is run to find it, so the lookup is cargo's own rule read from disk.
fn nearest_lock(root: &Path) -> Option<std::path::PathBuf> {
    root.ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|candidate| candidate.is_file())
}

/// Resolves a source into a plan: fetches, verifies and scans every
/// component, dependencies included, and decides each file's outcome.
pub fn resolve(
    source: &Source,
    options: &Options,
    fetcher: &dyn Fetcher,
    project: &ProjectFile,
) -> Result<Plan> {
    resolve_with(source, options, fetcher, project, &AllowlistScanner)
}

/// [`resolve`] with the scanner chosen.
pub fn resolve_with(
    source: &Source,
    options: &Options,
    fetcher: &dyn Fetcher,
    project: &ProjectFile,
    scanner: &dyn Scanner,
) -> Result<Plan> {
    let mut resolver = Resolver {
        options,
        fetcher,
        project,
        records: project.components()?,
        shipped_records: project.shipped()?,
        library_records: project.libraries()?,
        framework: None,
        versions: BTreeMap::new(),
        libraries: BTreeMap::new(),
        chosen: BTreeMap::new(),
        order: Vec::new(),
        pins: Vec::new(),
        namespaces: BTreeMap::new(),
        fetched: 0,
    };
    let address = source.component_address()?;
    resolver.visit(
        address,
        source.version().cloned(),
        "the command line".to_owned(),
    )?;
    resolver.finish(scanner)
}

/// One library at one version, fetched once for the whole plan.
struct LoadedLibrary {
    commit: Commit,
    json_bytes: Vec<u8>,
    json: LibraryJson,
}

/// The version a component address resolved to and who asked for it.
struct Chosen {
    version: semver::Version,
    required_by: Vec<String>,
}

/// One verified component, before the scan and the outcomes.
struct Loaded {
    address: ComponentAddress,
    version: semver::Version,
    library: LibraryJson,
    library_json_bytes: Vec<u8>,
    commit: Commit,
    manifest: ComponentManifest,
    manifest_bytes: Vec<u8>,
    signature: Option<String>,
    hash: Digest,
    files: Vec<(String, Vec<u8>)>,
    dependencies: Vec<ComponentAddress>,
}

struct Resolver<'a> {
    options: &'a Options,
    fetcher: &'a dyn Fetcher,
    project: &'a ProjectFile,
    records: BTreeMap<ComponentAddress, ComponentRecord>,
    shipped_records: BTreeMap<ComponentAddress, super::project::ShippedRecord>,
    library_records: BTreeMap<LibraryAddress, LibraryRecord>,
    framework: Option<FrameworkVersion>,
    versions: BTreeMap<LibraryAddress, Vec<semver::Version>>,
    libraries: BTreeMap<(LibraryAddress, semver::Version), LoadedLibrary>,
    chosen: BTreeMap<ComponentAddress, Chosen>,
    order: Vec<Loaded>,
    pins: Vec<KeyPin>,
    namespaces: BTreeMap<String, LibraryAddress>,
    fetched: u64,
}

/// A dependency's address and the version it names, if any: a `./`
/// dependency names its library's version, which the caller supplies.
fn dependency_address(
    library: &LibraryAddress,
    dependency: &str,
) -> Result<(ComponentAddress, Option<semver::Version>, bool)> {
    Ok(match parse_dependency(dependency)? {
        Dependency::Shipped(component) => (
            ComponentAddress {
                library: LibraryAddress(SHIPPED_LIBRARY.to_owned()),
                component,
            },
            None,
            false,
        ),
        Dependency::Sibling(component) => (
            ComponentAddress {
                library: library.clone(),
                component,
            },
            None,
            true,
        ),
        Dependency::Address(source) => (
            source.component_address()?,
            source.version().cloned(),
            false,
        ),
    })
}

impl Resolver<'_> {
    fn visit(
        &mut self,
        address: ComponentAddress,
        requested: Option<semver::Version>,
        required_by: String,
    ) -> Result<()> {
        let version = self.choose_version(&address.library, requested.as_ref())?;
        if let Some(chosen) = self.chosen.get_mut(&address) {
            if chosen.version != version {
                return Err(RegistryError::Invalid(format!(
                    "the plan needs {address} at two versions: {} for {}, and {version} for {required_by}; nothing was written",
                    chosen.version,
                    chosen.required_by.join(", ")
                )));
            }
            chosen.required_by.push(required_by);
            return Ok(());
        }
        if self.chosen.len() >= MAX_PLAN_COMPONENTS {
            return Err(RegistryError::Invalid(format!(
                "the plan holds more than {MAX_PLAN_COMPONENTS} components; nothing was written"
            )));
        }
        self.chosen.insert(
            address.clone(),
            Chosen {
                version: version.clone(),
                required_by: vec![required_by],
            },
        );
        let mut loaded = self.load(&address, &version)?;
        for dependency in loaded.manifest.dependencies.clone() {
            let (dependency_address, named, sibling) =
                dependency_address(&address.library, &dependency)?;
            let requested = if sibling {
                Some(version.clone())
            } else {
                named
            };
            self.visit(dependency_address.clone(), requested, address.to_string())?;
            loaded.dependencies.push(dependency_address);
        }
        self.order.push(loaded);
        Ok(())
    }

    fn choose_version(
        &mut self,
        library: &LibraryAddress,
        requested: Option<&semver::Version>,
    ) -> Result<semver::Version> {
        if library.is_shipped() {
            let shipped = shipped_version();
            if let Some(requested) = requested
                && *requested != shipped
            {
                return Err(RegistryError::Invalid(format!(
                    "the shipped library is at {shipped}, not {requested}"
                )));
            }
            return Ok(shipped);
        }
        if let Some(requested) = requested {
            return Ok(requested.clone());
        }
        if !self.versions.contains_key(library) {
            let versions = self.fetcher.versions(library)?;
            self.versions.insert(library.clone(), versions);
        }
        self.versions
            .get(library)
            .and_then(|versions| {
                versions
                    .iter()
                    .filter(|version| version.pre.is_empty())
                    .max()
                    .cloned()
            })
            .ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "{library} lists no release: tag a release `v<version>` (pre-releases are not installed by default)"
                ))
            })
    }

    fn framework(&mut self) -> Result<FrameworkVersion> {
        if let Some(framework) = &self.framework {
            return Ok(framework.clone());
        }
        let framework = framework_version(self.project.root())?;
        self.framework = Some(framework.clone());
        Ok(framework)
    }

    fn library(&mut self, library: &LibraryAddress, version: &semver::Version) -> Result<()> {
        let key = (library.clone(), version.clone());
        if self.libraries.contains_key(&key) {
            return Ok(());
        }
        if library.is_shipped() {
            // The binary serves the shipped library as the tree every library
            // has, `library.json` included (REG-016); it is read here like any
            // other, by the reader that admits its reserved namespace and no
            // key, and it is never verified or checked against the framework.
            let commit = EmbeddedFetcher.resolve(library, version)?;
            let json_bytes = EmbeddedFetcher.file(library, &commit, "library.json")?;
            let json = parse_shipped_library_json(&json_bytes)?;
            self.libraries.insert(
                key,
                LoadedLibrary {
                    commit,
                    json_bytes,
                    json,
                },
            );
            return Ok(());
        }
        let kind = library.kind()?;
        let commit = self.fetcher.resolve(library, version)?;
        let fetcher = self.fetcher;
        let json_bytes = self
            .fetch(fetcher, library, &commit, "library.json")
            .map_err(|error| {
                RegistryError::Invalid(format!(
                    "{library} at v{version} has no readable library.json at its root: {error}"
                ))
            })?;
        let json = parse_library_json(&json_bytes)
            .map_err(|error| RegistryError::Invalid(format!("{library} library.json: {error}")))?;
        if json.version != *version {
            return Err(RegistryError::Invalid(format!(
                "{library}'s tag v{version} holds a library.json that says version {}; refusing it",
                json.version
            )));
        }
        if matches!(
            kind,
            LibraryKind::Repository { .. } | LibraryKind::Url { .. }
        ) && json.source != library.0
        {
            return Err(RegistryError::Invalid(format!(
                "{library}'s library.json names its source as {}, not the address it was fetched from; refusing it",
                json.source
            )));
        }
        self.check_namespace(library, &json.namespace)?;
        let framework = self.framework()?;
        if !json.framework.matches(&framework.version) {
            return Err(RegistryError::Invalid(format!(
                "{library} {version} requires suprnova {}; this application uses suprnova {} (from {}); refusing it",
                json.framework, framework.version, framework.from
            )));
        }
        self.check_key(library, &json)?;
        self.libraries.insert(
            key,
            LoadedLibrary {
                commit,
                json_bytes,
                json,
            },
        );
        Ok(())
    }

    /// A namespace stays with the library it was first installed from
    /// (REG-011), and a library keeps the namespace it was installed under.
    fn check_namespace(&mut self, library: &LibraryAddress, namespace: &str) -> Result<()> {
        if let Some(owner) = self.project.namespace_owner(namespace)?
            && owner != *library
        {
            return Err(RegistryError::Invalid(format!(
                "the namespace `{namespace}` belongs to {owner} in this application; {library} claims it too, so it is refused"
            )));
        }
        if let Some(recorded) = self
            .library_records
            .get(library)
            .and_then(|record| record.namespace.as_deref())
            && recorded != namespace
        {
            return Err(RegistryError::Invalid(format!(
                "{library} was installed under the namespace `{recorded}` and now names `{namespace}`; refusing it"
            )));
        }
        if let Some(claimed) = self.namespaces.get(namespace)
            && claimed != library
        {
            return Err(RegistryError::Invalid(format!(
                "{claimed} and {library} both claim the namespace `{namespace}` in one plan; refusing it"
            )));
        }
        self.namespaces
            .insert(namespace.to_owned(), library.clone());
        Ok(())
    }

    /// The pinned key decides (REG-024, REG-033): no pin pins the library's
    /// key after a terminal confirmation; a pin that differs is accepted
    /// only with a handover statement the pinned key signed.
    fn check_key(&mut self, library: &LibraryAddress, json: &LibraryJson) -> Result<()> {
        let pinned = self
            .library_records
            .get(library)
            .and_then(|record| record.public_key.clone());
        let pin = match pinned {
            Some(pinned) if pinned == json.public_key => None,
            Some(pinned) => {
                let handover = json
                    .previous_keys
                    .iter()
                    .find(|handover| handover.from == pinned)
                    .ok_or_else(|| {
                        RegistryError::Invalid(format!(
                            "{library} is signed by the key {}, but this application pinned {} for it, and the pinned key vouches for no change; refusing it",
                            json.public_key.fingerprint(),
                            pinned.fingerprint()
                        ))
                    })?;
                verify_handover(handover, &json.public_key, &json.source).map_err(|error| {
                    RegistryError::Invalid(format!(
                        "{library} changed its key from {} to {}, and the handover does not verify: {error}; refusing it",
                        pinned.fingerprint(),
                        json.public_key.fingerprint()
                    ))
                })?;
                Some(KeyPin {
                    library: library.clone(),
                    key: json.public_key.clone(),
                    replaces: Some(pinned.fingerprint()),
                })
            }
            None => Some(KeyPin {
                library: library.clone(),
                key: json.public_key.clone(),
                replaces: None,
            }),
        };
        if let Some(pin) = pin {
            if let Some(existing) = self
                .pins
                .iter()
                .find(|existing| existing.library == pin.library)
            {
                if existing.key != pin.key {
                    return Err(RegistryError::Invalid(format!(
                        "{library} names two signing keys in one plan; refusing it"
                    )));
                }
                return Ok(());
            }
            self.pins.push(pin);
        }
        Ok(())
    }

    /// Fetches one file and counts it against the plan's limit (REG-009):
    /// a plan that has fetched more than [`MAX_PLAN_BYTES`] is refused
    /// before it fetches more.
    fn fetch(
        &mut self,
        fetcher: &dyn Fetcher,
        library: &LibraryAddress,
        commit: &Commit,
        path: &str,
    ) -> Result<Vec<u8>> {
        if self.fetched > MAX_PLAN_BYTES {
            return Err(too_large_plan());
        }
        let bytes = fetcher.file(library, commit, path)?;
        self.fetched += bytes.len() as u64;
        if self.fetched > MAX_PLAN_BYTES {
            return Err(too_large_plan());
        }
        Ok(bytes)
    }

    fn load(&mut self, address: &ComponentAddress, version: &semver::Version) -> Result<Loaded> {
        self.library(&address.library, version)?;
        let library = self
            .libraries
            .get(&(address.library.clone(), version.clone()))
            .ok_or_else(|| RegistryError::Invalid(format!("{} was not loaded", address.library)))?;
        let directory = address.component.as_str();
        let shipped = address.library.is_shipped();
        let fetcher: &dyn Fetcher = if shipped {
            &EmbeddedFetcher
        } else {
            self.fetcher
        };
        let commit = library.commit.clone();
        let json = library.json.clone();
        let json_bytes = library.json_bytes.clone();
        let manifest_bytes = self
            .fetch(
                fetcher,
                &address.library,
                &commit,
                &component_path(directory, "manifest.json"),
            )
            .map_err(|error| {
                if shipped && !plan_too_large(&error) {
                    RegistryError::Invalid(format!(
                        "`{directory}` is not a shipped library component; shipped components: {}",
                        EmbeddedFetcher.components().join(", ")
                    ))
                } else {
                    RegistryError::Invalid(format!(
                        "{address} has no readable components/{directory}/manifest.json: {error}"
                    ))
                }
            })?;
        let manifest = if shipped {
            parse_shipped_manifest(&manifest_bytes, directory)?
        } else {
            parse_manifest(&manifest_bytes, directory, &json.namespace)?
        };
        let mut files = Vec::with_capacity(manifest.files.len());
        for name in &manifest.files {
            let bytes = self
                .fetch(
                    fetcher,
                    &address.library,
                    &commit,
                    &component_path(directory, name),
                )
                .map_err(|error| {
                    RegistryError::Invalid(format!(
                        "{address} names {name}, which could not be read: {error}"
                    ))
                })?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(RegistryError::Invalid(format!(
                    "{address}'s {name} is {} bytes, over the {MAX_FILE_BYTES} byte limit",
                    bytes.len()
                )));
            }
            files.push((name.clone(), bytes));
        }
        let statement = Statement {
            library: json.source.clone(),
            version: version.clone(),
            component: directory.to_owned(),
            library_json: Digest::of(&json_bytes),
            manifest: Digest::of(&manifest_bytes),
            files: files
                .iter()
                .map(|(name, bytes)| (name.clone(), Digest::of(bytes)))
                .collect(),
        };
        let hash = statement.verification_hash();
        let signature = if shipped {
            None
        } else {
            let bytes = self
                .fetch(
                    fetcher,
                    &address.library,
                    &commit,
                    &component_path(directory, "manifest.sig"),
                )
                .map_err(|error| {
                    RegistryError::Invalid(format!(
                        "{address} is unsigned: components/{directory}/manifest.sig could not be read ({error}); refusing it"
                    ))
                })?;
            let text = String::from_utf8(bytes).map_err(|_| {
                RegistryError::Invalid(format!("{address}'s manifest.sig is not text; refusing it"))
            })?;
            let signature = Signature::parse_strict(&text).map_err(|error| {
                RegistryError::Invalid(format!("{address}'s manifest.sig: {error}; refusing it"))
            })?;
            verify(&json.public_key, &hash, &signature).map_err(|error| {
                RegistryError::Invalid(format!("{address} does not verify: {error}; refusing it"))
            })?;
            Some(signature.encode())
        };
        Ok(Loaded {
            address: address.clone(),
            version: version.clone(),
            library: json,
            library_json_bytes: json_bytes,
            commit,
            manifest,
            manifest_bytes,
            signature,
            hash,
            files,
            dependencies: Vec::new(),
        })
    }

    fn finish(mut self, scanner: &dyn Scanner) -> Result<Plan> {
        let root = self.project.root().to_path_buf();
        let order = std::mem::take(&mut self.order);
        let in_plan: BTreeSet<ComponentAddress> =
            order.iter().map(|loaded| loaded.address.clone()).collect();
        let shipped_views = shipped_views();
        let mut modules_of: BTreeMap<ComponentAddress, Reachable> = BTreeMap::new();
        // Each planned component's view sources by view path, for the view
        // checks of the components that depend on it.
        let mut view_sources_of: BTreeMap<ComponentAddress, Vec<(String, String)>> =
            BTreeMap::new();
        let mut rust_owners: BTreeMap<(LibraryAddress, String), ComponentAddress> = BTreeMap::new();
        let mut components = Vec::with_capacity(order.len());
        let mut router_calls = Vec::new();
        for mut loaded in order {
            let address = loaded.address.clone();
            let namespace = loaded.library.namespace.clone();
            let ns_module = namespace_module(&namespace);
            let shipped = address.library.is_shipped();
            let mut forced = self.check_recorded_version(&loaded)?;
            forced.extend(self.check_dependents(&loaded, &in_plan)?);
            for name in &loaded.manifest.files {
                if FileKind::of(name) != Some(FileKind::Rust) {
                    continue;
                }
                let key = (address.library.clone(), name.clone());
                if let Some(other) = rust_owners.get(&key) {
                    return Err(RegistryError::Invalid(format!(
                        "{other} and {address} both install src/live/{ns_module}/{name}; refusing it"
                    )));
                }
                if let Some((other, _)) = self.records.iter().find(|(other, record)| {
                    other.library == address.library
                        && **other != address
                        && record.files.contains_key(name)
                }) {
                    return Err(RegistryError::Invalid(format!(
                        "{address} would overwrite src/live/{ns_module}/{name}, which {other} installed; refusing it"
                    )));
                }
                rust_owners.insert(key, address.clone());
            }

            let directory = view_directory(&namespace, &address.component);
            let record = InstallRecord::load(&root, &directory)?;
            let mut outcomes = Vec::with_capacity(loaded.files.len());
            for (name, bytes) in &loaded.files {
                let kind = FileKind::of(name).ok_or_else(|| {
                    RegistryError::Invalid(format!("{address} names {name}, which has no kind"))
                })?;
                let path = destination(&namespace, &address.component, name, kind);
                let outcome = outcome_for(&root, &path, bytes, &record, self.options.force)?;
                outcomes.push((kind, path, outcome));
            }
            let dropped = self.dropped_files(&loaded, &namespace, &record)?;
            let kept_modules: Vec<&str> = dropped
                .iter()
                .filter(|(file, _)| file.kind == FileKind::Rust && file.outcome.keeps())
                .filter_map(|(file, _)| file.name.strip_suffix(".rs"))
                .collect();

            let modules = loaded.manifest.rust_modules();
            let mut module_declarations = Vec::new();
            if !modules.is_empty() {
                module_declarations.push((
                    PathBuf::from("src/live/mod.rs"),
                    format!("pub mod {ns_module};"),
                ));
                for module in &modules {
                    module_declarations.push((
                        rust_directory(&namespace).join("mod.rs"),
                        format!("pub mod {module};"),
                    ));
                }
            }
            let recorded: Vec<String> = self
                .records
                .get(&address)
                .map(|record| record.registered.clone())
                .unwrap_or_default();
            // A Rust file the library dropped and the application edited
            // stays in the build, and so do the registrations of its types.
            let mut registrations: Vec<String> = loaded
                .manifest
                .register
                .iter()
                .map(|entry| format!("crate::live::{ns_module}::{entry}"))
                .collect();
            for path in &recorded {
                let retained = kept_modules.iter().any(|module| {
                    path.starts_with(&format!("crate::live::{ns_module}::{module}::"))
                });
                if retained && !registrations.contains(path) {
                    registrations.push(path.clone());
                }
            }
            let unregistrations: Vec<String> = recorded
                .iter()
                .filter(|path| !registrations.contains(path))
                .cloned()
                .collect();
            let kept_rust: Vec<(String, PathBuf)> = loaded
                .files
                .iter()
                .zip(&outcomes)
                .filter(|(_, (kind, _, outcome))| *kind == FileKind::Rust && outcome.keeps())
                .map(|((name, _), (_, path, _))| (name.clone(), path.clone()))
                .collect();
            if !kept_rust.is_empty() {
                let mut incoming = registrations.clone();
                incoming.sort();
                let mut before = recorded.clone();
                before.sort();
                if incoming != before {
                    let names: Vec<String> = kept_rust
                        .iter()
                        .map(|(_, path)| path.display().to_string())
                        .collect();
                    return Err(RegistryError::Invalid(format!(
                        "{address} changes what it registers, but {} is kept as edited; reconcile it with the new version (or pass --force to replace it), then run live:add again",
                        names.join(", ")
                    )));
                }
            }

            let scan = if shipped {
                ScanReport::default()
            } else {
                let mut dependency_modules = Vec::new();
                let mut importable_views = shipped_views.clone();
                let mut importable_scripts = Vec::new();
                let mut dependency_views = Vec::new();
                for dependency in &loaded.dependencies {
                    if let Some(reachable) = modules_of.get(dependency) {
                        dependency_modules.extend(
                            reachable
                                .rust
                                .iter()
                                .map(|name| format!("{}::{name}", reachable.module)),
                        );
                        importable_views.extend(reachable.views.iter().cloned());
                        importable_scripts.extend(reachable.scripts.iter().cloned());
                    }
                    if let Some(sources) = view_sources_of.get(dependency) {
                        dependency_views.extend(sources.iter().cloned());
                    }
                }
                let context = ScanContext {
                    register: Some(&loaded.manifest.register),
                    elements: Some(&loaded.manifest.elements),
                    dependency_views: &dependency_views,
                };
                let incoming = scanner.scan(
                    &ComponentFiles {
                        namespace: &namespace,
                        directory: &address.component,
                        files: &loaded.files,
                        dependency_modules: &dependency_modules,
                        importable_views: &importable_views,
                        importable_scripts: &importable_scripts,
                    },
                    &context,
                )?;
                if !incoming.accepted() {
                    return Err(RegistryError::Refused(incoming.findings));
                }
                check_register(&address, &loaded.manifest, &incoming)?;
                let kept_dropped: Vec<(String, Vec<u8>)> = dropped
                    .iter()
                    .filter(|(file, _)| file.outcome.keeps())
                    .map(|(file, bytes)| (file.name.clone(), bytes.clone()))
                    .collect();
                if kept_rust.is_empty() && kept_dropped.is_empty() {
                    incoming
                } else {
                    // The capabilities recorded are those of what ends up
                    // installed (REG-013): kept Rust files are scanned in
                    // place of the incoming ones, and every kept file the
                    // library dropped is scanned beside them.
                    let incoming_count = loaded.files.len();
                    loaded.files.extend(kept_dropped);
                    let mut swapped = Vec::new();
                    for (name, path) in &kept_rust {
                        let on_disk = std::fs::read(root.join(path)).map_err(|error| {
                            RegistryError::Io(format!("cannot read {}: {error}", path.display()))
                        })?;
                        if let Some((_, bytes)) =
                            loaded.files.iter_mut().find(|(file, _)| file == name)
                        {
                            swapped.push((name.clone(), std::mem::replace(bytes, on_disk)));
                        }
                    }
                    // What stays installed registers what the incoming
                    // manifest does and the types of the kept dropped files.
                    let mut installed_manifest = loaded.manifest.clone();
                    let prefix = format!("crate::live::{ns_module}::");
                    for path in &registrations {
                        if let Some(entry) = path.strip_prefix(&prefix)
                            && !installed_manifest
                                .register
                                .iter()
                                .any(|known| known == entry)
                        {
                            installed_manifest.register.push(entry.to_owned());
                        }
                    }
                    let installed = scanner.scan(
                        &ComponentFiles {
                            namespace: &namespace,
                            directory: &address.component,
                            files: &loaded.files,
                            dependency_modules: &dependency_modules,
                            importable_views: &importable_views,
                            importable_scripts: &importable_scripts,
                        },
                        &ScanContext {
                            register: Some(&installed_manifest.register),
                            elements: Some(&installed_manifest.elements),
                            dependency_views: &dependency_views,
                        },
                    );
                    for (name, original) in swapped {
                        if let Some((_, bytes)) =
                            loaded.files.iter_mut().find(|(file, _)| *file == name)
                        {
                            *bytes = original;
                        }
                    }
                    loaded.files.truncate(incoming_count);
                    let installed = installed?;
                    if !installed.accepted() {
                        return Err(RegistryError::Refused(installed.findings));
                    }
                    installed
                }
            };

            let views: Vec<String> = loaded
                .manifest
                .files
                .iter()
                .filter(|name| FileKind::of(name) == Some(FileKind::View))
                .map(|name| format!("{namespace}-ui/{}/{name}", address.component))
                .collect();
            let scripts: Vec<String> = loaded
                .manifest
                .files
                .iter()
                .filter(|name| FileKind::of(name) == Some(FileKind::Script))
                .map(|name| format!("{namespace}-ui/{}/{name}", address.component))
                .collect();
            modules_of.insert(
                address.clone(),
                Reachable {
                    module: ns_module.clone(),
                    rust: modules.clone(),
                    views,
                    scripts,
                },
            );
            let sources: Vec<(String, String)> = loaded
                .files
                .iter()
                .filter(|(name, _)| FileKind::of(name) == Some(FileKind::View))
                .filter_map(|(name, bytes)| {
                    let source = std::str::from_utf8(bytes).ok()?.to_string();
                    Some((
                        format!("{namespace}-ui/{}/{name}", address.component),
                        source,
                    ))
                })
                .collect();
            view_sources_of.insert(address.clone(), sources);

            let call = if shipped {
                (self.shipped_records.is_empty()).then(|| "try_live_ui_assets()".to_owned())
            } else {
                self.project
                    .namespace_owner(&namespace)?
                    .is_none()
                    .then(|| format!("try_live_ui_assets_for(\"{namespace}\")"))
            };
            if let Some(call) = call
                && !router_calls.contains(&call)
            {
                router_calls.push(call);
            }

            let files = loaded
                .files
                .into_iter()
                .zip(outcomes)
                .map(
                    |((name, bytes), (kind, destination, outcome))| PlannedFile {
                        digest: Digest::of(&bytes),
                        name,
                        destination,
                        kind,
                        outcome,
                        bytes,
                    },
                )
                .collect();
            components.push(PlannedComponent {
                address,
                library: loaded.library,
                version: loaded.version,
                commit: loaded.commit,
                manifest: loaded.manifest,
                library_json_bytes: loaded.library_json_bytes,
                manifest_bytes: loaded.manifest_bytes,
                signature: loaded.signature,
                hash: loaded.hash,
                files,
                scan,
                module_declarations,
                registrations,
                unregistrations,
                dropped: dropped.into_iter().map(|(file, _)| file).collect(),
                forced,
            });
        }
        let framework_version = match self.framework {
            Some(framework) => framework.version,
            None => shipped_version(),
        };
        Ok(Plan {
            components,
            framework_version,
            pins: self.pins,
            router_calls,
        })
    }

    /// A downgrade, or the recorded version with other content, is refused
    /// without `--force` (REG-026); with it, each is returned in those words
    /// for the plan to show.
    fn check_recorded_version(&self, loaded: &Loaded) -> Result<Vec<String>> {
        let Some(record) = self.records.get(&loaded.address) else {
            return Ok(Vec::new());
        };
        let mut forced = Vec::new();
        if loaded.version < record.version {
            if !self.options.force {
                return Err(RegistryError::Invalid(format!(
                    "{} {} is older than the {} this application records; pass --force to downgrade",
                    loaded.address, loaded.version, record.version
                )));
            }
            forced.push(format!(
                "downgrade from {} to {}, let through by --force",
                record.version, loaded.version
            ));
        }
        if loaded.version == record.version && loaded.hash != record.hash {
            if !self.options.force {
                return Err(RegistryError::Invalid(format!(
                    "{} {} is not what this application recorded at that version: the library changed a released version (recorded {}, fetched {}); pass --force to accept it",
                    loaded.address, loaded.version, record.hash, loaded.hash
                )));
            }
            forced.push(format!(
                "moved tag: {} holds other content than this application recorded (recorded {}, fetched {}), let through by --force",
                loaded.version, record.hash, loaded.hash
            ));
        }
        Ok(forced)
    }

    /// Files the recorded version installed that this version no longer
    /// names (REG-028), each with its bytes on disk: removed when the
    /// application never edited it, or with `--force`; kept otherwise.
    fn dropped_files(
        &self,
        loaded: &Loaded,
        namespace: &str,
        record: &InstallRecord,
    ) -> Result<Vec<(DroppedFile, Vec<u8>)>> {
        let root = self.project.root();
        let mut recorded: BTreeSet<String> = BTreeSet::new();
        if let Some(previous) = self.records.get(&loaded.address) {
            recorded.extend(previous.files.keys().cloned());
            recorded.extend(previous.kept.iter().cloned());
        }
        if let Some(previous) = self.shipped_records.get(&loaded.address) {
            recorded.extend(previous.files.keys().cloned());
        }
        let mut dropped = Vec::new();
        for name in recorded {
            if loaded.manifest.files.contains(&name) {
                continue;
            }
            let Ok(kind) = validate_file_name(&name) else {
                continue;
            };
            let destination = destination(namespace, &loaded.address.component, &name, kind);
            secure_fs::ensure_contained(root, &destination).map_err(RegistryError::Io)?;
            let bytes = match std::fs::read(root.join(&destination)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(RegistryError::Io(format!(
                        "cannot read {}: {error}",
                        destination.display()
                    )));
                }
            };
            let outcome = if self.options.force {
                FileOutcome::Removed
            } else {
                match record.digest(&destination) {
                    Some(digest) if *digest == Digest::of(&bytes) => FileOutcome::Removed,
                    Some(_) => FileOutcome::Kept,
                    None => FileOutcome::ChangedSinceRecord,
                }
            };
            dropped.push((
                DroppedFile {
                    name,
                    destination,
                    kind,
                    outcome,
                },
                bytes,
            ));
        }
        Ok(dropped)
    }

    /// Replacing a component with a version an installed dependent did not
    /// record is refused without `--force` (REG-010).
    fn check_dependents(
        &self,
        loaded: &Loaded,
        in_plan: &BTreeSet<ComponentAddress>,
    ) -> Result<Vec<String>> {
        let mut forced = Vec::new();
        for (dependent, record) in &self.records {
            if in_plan.contains(dependent) {
                continue;
            }
            if let Some(version) = record.dependencies.get(&loaded.address)
                && *version != loaded.version
            {
                if !self.options.force {
                    return Err(RegistryError::Invalid(format!(
                        "{dependent} depends on {} {version}; this plan installs {}; pass --force to replace it anyway",
                        loaded.address, loaded.version
                    )));
                }
                forced.push(format!(
                    "replaces the {version} {dependent} depends on with {}, let through by --force",
                    loaded.version
                ));
            }
        }
        Ok(forced)
    }
}

fn too_large_plan() -> RegistryError {
    RegistryError::Invalid(format!(
        "the plan has fetched more than {} MiB, the most one install may fetch; refusing it before fetching more",
        MAX_PLAN_BYTES / (1024 * 1024)
    ))
}

fn plan_too_large(error: &RegistryError) -> bool {
    *error == too_large_plan()
}

/// Each type a `#[live]` attribute defines must be exactly one entry of
/// the manifest's `register`, and each entry a type its Rust defines
/// (REG-030, REG-005): a registration names only what the scan saw.
fn check_register(
    address: &ComponentAddress,
    manifest: &ComponentManifest,
    scan: &ScanReport,
) -> Result<()> {
    let defined: BTreeSet<&String> = scan.defined_components.iter().collect();
    let declared: BTreeSet<&String> = manifest.register.iter().collect();
    if defined == declared && defined.len() == scan.defined_components.len() {
        return Ok(());
    }
    let list = |items: &BTreeSet<&String>| {
        if items.is_empty() {
            "nothing".to_owned()
        } else {
            items
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    Err(RegistryError::Invalid(format!(
        "{address}'s Rust defines {} as Live components and its manifest registers {}; each #[live] type must be exactly one `register` entry; refusing it",
        list(&defined),
        list(&declared)
    )))
}

/// Every shipped view, as a view of another component may name it.
/// What a planned component offers the components that depend on it: its
/// namespace module and Rust file stems, which their Rust may name, and its
/// views and scripts, which their views may include and their scripts may
/// import (REG-030, REG-031, REG-032).
struct Reachable {
    module: String,
    rust: Vec<String>,
    views: Vec<String>,
    scripts: Vec<String>,
}

fn shipped_views() -> Vec<String> {
    super::fetch::COMPONENTS
        .iter()
        .flat_map(|component| {
            component
                .files
                .iter()
                .filter(|(name, _)| FileKind::of(name) == Some(FileKind::View))
                .map(|(name, _)| format!("{SHIPPED_LIBRARY}-ui/{}/{name}", component.directory))
        })
        .collect()
}

/// What installing `incoming` at `path` would do, judged against the
/// install record (REG-028, UI-022): an unedited file is replaced, an edited
/// one or one no record vouches for is kept, and `--force` replaces it.
pub fn outcome_for(
    root: &Path,
    path: &Path,
    incoming: &[u8],
    record: &InstallRecord,
    force: bool,
) -> Result<FileOutcome> {
    secure_fs::ensure_contained(root, path).map_err(RegistryError::Io)?;
    match std::fs::read(root.join(path)) {
        Ok(existing) if existing == incoming => Ok(FileOutcome::Unchanged),
        Ok(_) if force => Ok(FileOutcome::Replaced),
        Ok(existing) => Ok(match record.digest(path) {
            Some(digest) if *digest == Digest::of(&existing) => FileOutcome::Replaced,
            Some(_) => FileOutcome::Kept,
            None => FileOutcome::ChangedSinceRecord,
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(FileOutcome::New),
        Err(error) => Err(RegistryError::Io(format!(
            "cannot read {}: {error}",
            path.display()
        ))),
    }
}

/// What the developer decided: the approval of each capability of each
/// component (REG-006).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decisions {
    /// Each third-party component's capabilities with their approvals.
    pub approvals: BTreeMap<ComponentAddress, BTreeMap<Capability, Approval>>,
}

/// Asks the developer, on a terminal or not at all.
pub trait Prompter {
    /// Whether a person can answer.
    fn is_terminal(&self) -> bool;

    /// Asks a yes-or-no question; no is the default.
    fn confirm(&mut self, question: &str) -> Result<bool>;
}

/// The terminal `live:add` runs on: a person can answer when standard input
/// and standard error are both a terminal.
#[derive(Debug, Default)]
pub struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn is_terminal(&self) -> bool {
        use std::io::IsTerminal as _;
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    fn confirm(&mut self, question: &str) -> Result<bool> {
        dialoguer::Confirm::new()
            .with_prompt(question)
            .default(false)
            .interact()
            .map_err(|error| RegistryError::Io(format!("cannot ask on the terminal: {error}")))
    }
}

/// The approvals the records and `--allow` already give, asking nobody.
/// Refuses a capability neither covers, since no approval can be recorded
/// for it.
pub fn decisions_from_flags(
    plan: &Plan,
    options: &Options,
    project: &ProjectFile,
) -> Result<Decisions> {
    struct Nobody;
    impl Prompter for Nobody {
        fn is_terminal(&self) -> bool {
            false
        }
        fn confirm(&mut self, _question: &str) -> Result<bool> {
            Ok(false)
        }
    }
    approvals(plan, options, project, &mut Nobody)
}

fn approvals(
    plan: &Plan,
    options: &Options,
    project: &ProjectFile,
    prompter: &mut dyn Prompter,
) -> Result<Decisions> {
    let records = project.components()?;
    let terminal = prompter.is_terminal();
    let mut decisions = Decisions::default();
    for component in plan
        .components
        .iter()
        .filter(|component| !component.is_shipped())
    {
        let recorded = records
            .get(&component.address)
            .map(|record| &record.capabilities);
        let mut approved = BTreeMap::new();
        for capability in &component.scan.capabilities {
            if let Some(approval) = recorded.and_then(|recorded| recorded.get(capability)) {
                approved.insert(*capability, *approval);
                continue;
            }
            if options.allow.contains(capability) {
                approved.insert(*capability, Approval::Flag);
                continue;
            }
            if !terminal {
                return Err(RegistryError::Declined(format!(
                    "{} uses the `{capability}` capability, which needs your approval: pass --allow {capability}, or run live:add on a terminal (--yes does not approve a capability); nothing was written",
                    component.address
                )));
            }
            if prompter.confirm(&format!(
                "Allow {} the `{capability}` capability?",
                component.address
            ))? {
                approved.insert(*capability, Approval::Terminal);
            } else {
                return Err(RegistryError::Declined(format!(
                    "the `{capability}` capability of {} was denied; nothing was written",
                    component.address
                )));
            }
        }
        decisions
            .approvals
            .insert(component.address.clone(), approved);
    }
    Ok(decisions)
}

/// Takes the developer's decisions on a rendered plan (REG-006, REG-012,
/// REG-024, REG-033): every capability by name, every key to pin, and the
/// plan itself for any library but the shipped one. `--yes` confirms the
/// plan without a terminal and does nothing else; `--allow` approves the
/// capabilities it names. Any refusal leaves nothing written.
pub fn confirm(
    plan: &Plan,
    options: &Options,
    project: &ProjectFile,
    prompter: &mut dyn Prompter,
) -> Result<Decisions> {
    let decisions = approvals(plan, options, project, prompter)?;
    let terminal = prompter.is_terminal();
    for pin in &plan.pins {
        if !terminal {
            return Err(RegistryError::Declined(format!(
                "{} {}; pinning a key needs a terminal, and --yes does not pin one. To pin it by hand, add to suprnova.toml:\n[live.libraries.\"{}\"]\nkey = \"{}\"\nNothing was written.",
                pin.library,
                match &pin.replaces {
                    None => format!("has no pinned key; its key is {}", pin.key.fingerprint()),
                    Some(old) => format!(
                        "changed its key from {old} to {}, handed over by the pinned key",
                        pin.key.fingerprint()
                    ),
                },
                pin.library,
                pin.key.encode()
            )));
        }
        let question = match &pin.replaces {
            None => format!(
                "Pin the key {} for {}? Later versions must be signed by it.",
                pin.key.fingerprint(),
                pin.library
            ),
            Some(old) => format!(
                "{} changed its signing key from {old} to {}, and the pinned key vouches for the change. Re-pin it?",
                pin.library,
                pin.key.fingerprint()
            ),
        };
        if !prompter.confirm(&question)? {
            return Err(RegistryError::Declined(format!(
                "the key for {} was not pinned; nothing was written",
                pin.library
            )));
        }
    }
    if plan.has_third_party() && !options.yes {
        if !terminal {
            return Err(RegistryError::Declined(
                "live:add installs a component from a third-party library only after you confirm its plan on a terminal; without one, pass --yes. Nothing was written."
                    .to_owned(),
            ));
        }
        if !prompter.confirm("Install this plan?")? {
            return Err(RegistryError::Declined(
                "the plan was declined; nothing was written".to_owned(),
            ));
        }
    }
    Ok(decisions)
}

/// Renders the plan as `live:add` reports it before asking (REG-012).
pub fn render(plan: &Plan) -> String {
    render_lines(plan, None)
}

/// Renders the plan with each capability's approval and the manifest's
/// outcome, as `live:add` shows it before asking (REG-012).
pub fn render_with(plan: &Plan, options: &Options, project: &ProjectFile) -> String {
    render_lines(plan, Some((options, project)))
}

fn render_lines(plan: &Plan, context: Option<(&Options, &ProjectFile)>) -> String {
    let mut out = String::new();
    let records = context
        .and_then(|(_, project)| project.components().ok())
        .unwrap_or_default();
    // What the application's registry already registers, so a repeat
    // install says so rather than listing it as a line it would add.
    let registered = context
        .and_then(|(_, project)| {
            super::registration::registered_components(project.root())
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    if plan.has_third_party() {
        out.push_str(&format!("framework: suprnova {}\n", plan.framework_version));
    }
    for component in &plan.components {
        if component.is_shipped() {
            out.push_str(&format!(
                "{} (shipped library, suprnova-cli {})\n",
                component.address, component.version
            ));
        } else {
            out.push_str(&format!("{} {}\n", component.address, component.version));
            if !component.commit.0.is_empty() {
                out.push_str(&format!("  commit      {}\n", component.commit.0));
            }
            out.push_str(&format!("  hash        {}\n", component.hash));
        }
        if let Some((options, project)) = context {
            let manifest = component.manifest_destination();
            let outcome = InstallRecord::load(project.root(), &component.view_directory())
                .and_then(|record| {
                    outcome_for(
                        project.root(),
                        &manifest,
                        &component.manifest_bytes,
                        &record,
                        options.force,
                    )
                });
            match outcome {
                Ok(outcome) => out.push_str(&format!(
                    "  {}  {}\n",
                    manifest.display(),
                    outcome.describe()
                )),
                Err(error) => out.push_str(&format!("  {}  {error}\n", manifest.display())),
            }
        }
        for file in &component.files {
            out.push_str(&format!(
                "  {}  {}\n",
                file.destination.display(),
                file.outcome.describe()
            ));
        }
        for file in &component.dropped {
            let what = match file.outcome {
                FileOutcome::Kept => {
                    "kept, edited locally: the library dropped it (pass --force to remove it)"
                }
                FileOutcome::ChangedSinceRecord => {
                    "kept, no install record shows it unedited: the library dropped it (pass --force to remove it)"
                }
                other => other.describe(),
            };
            out.push_str(&format!("  {}  {what}\n", file.destination.display()));
        }
        for line in &component.forced {
            out.push_str(&format!("  forced      {line}\n"));
        }
        for (file, line) in &component.module_declarations {
            let state = match context {
                Some((_, project)) if declared(project.root(), file, line) => " already declared",
                _ => "",
            };
            out.push_str(&format!(
                "  module      {}: {line}{state}\n",
                file.display()
            ));
        }
        for path in &component.registrations {
            let state = if registered.contains(path) {
                " already registered"
            } else {
                ""
            };
            out.push_str(&format!("  register    {path}{state}\n"));
        }
        for path in &component.unregistrations {
            out.push_str(&format!("  unregister  {path}\n"));
        }
        if let Ok(versions) = plan.dependency_versions(component) {
            for (address, version) in versions {
                out.push_str(&format!("  depends on  {address} {version}\n"));
            }
        }
        if !component.is_shipped() {
            out.push_str(&format!(
                "{}: capabilities: {}\n",
                component.address,
                capability_list(&component.scan.capabilities)
            ));
        }
        for capability in &component.scan.capabilities {
            let status = match context {
                Some((options, _)) => {
                    match records
                        .get(&component.address)
                        .and_then(|record| record.capabilities.get(capability))
                    {
                        Some(approval) => {
                            format!("approved at an earlier install ({})", approval.name())
                        }
                        None if options.allow.contains(capability) => {
                            "approved by --allow".to_owned()
                        }
                        None => format!(
                            "needs your approval (--allow {capability}, or answer on the terminal)"
                        ),
                    }
                }
                None => "needs approval".to_owned(),
            };
            out.push_str(&format!("  approval    {capability}: {status}\n"));
        }
    }
    for pin in &plan.pins {
        match &pin.replaces {
            None => out.push_str(&format!(
                "key: pin {} for {} (no key is pinned for it yet)\n",
                pin.key.fingerprint(),
                pin.library
            )),
            Some(old) => out.push_str(&format!(
                "key: re-pin {} from {old} to {} (handed over by the pinned key)\n",
                pin.library,
                pin.key.fingerprint()
            )),
        }
    }
    for call in &plan.router_calls {
        out.push_str(&format!(
            "serve its assets: call `router.{call}` when you build the router\n"
        ));
    }
    out
}

/// A component's capabilities as `live:add` and `live:registry check` both
/// print them: lowercase names, comma-separated, or `none`.
pub fn capability_list(capabilities: &BTreeSet<Capability>) -> String {
    if capabilities.is_empty() {
        "none".to_owned()
    } else {
        capabilities
            .iter()
            .map(|capability| capability.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Whether `file` under `root` already declares the module a plan line
/// `pub mod <name>;` names, as a bodiless `mod <name>;` item.
fn declared(root: &Path, file: &Path, line: &str) -> bool {
    let Some(name) = line
        .strip_prefix("pub mod ")
        .and_then(|rest| rest.strip_suffix(';'))
    else {
        return false;
    };
    if secure_fs::ensure_contained(root, file).is_err() {
        return false;
    }
    let Ok(source) = std::fs::read_to_string(root.join(file)) else {
        return false;
    };
    let Ok(parsed) = syn::parse_file(&source) else {
        return false;
    };
    parsed.items.iter().any(|item| {
        matches!(item, syn::Item::Mod(module) if module.content.is_none() && module.ident == name)
    })
}

/// The record a third-party component's install writes (REG-013): what
/// arrived, what it registered, the approvals, and the files it kept.
pub fn component_record(
    plan: &Plan,
    component: &PlannedComponent,
    decisions: &Decisions,
    kept: Vec<String>,
) -> Result<ComponentRecord> {
    let signature = component.signature.as_deref().ok_or_else(|| {
        RegistryError::Invalid(format!("{} carries no signature", component.address))
    })?;
    Ok(ComponentRecord {
        source: component.library.source.clone(),
        version: component.version.clone(),
        commit: component.commit.0.clone(),
        library_json: Digest::of(&component.library_json_bytes),
        manifest: Digest::of(&component.manifest_bytes),
        files: component
            .files
            .iter()
            .map(|file| (file.name.clone(), file.digest.clone()))
            .collect(),
        hash: component.hash.clone(),
        signature: Signature::parse(signature)?,
        dependencies: plan.dependency_versions(component)?,
        registered: component.registrations.clone(),
        capabilities: decisions
            .approvals
            .get(&component.address)
            .cloned()
            .unwrap_or_default(),
        kept,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::{Options, Scanner, resolve_with};
    use crate::registry::address::{self, LibraryAddress};
    use crate::registry::fetch::{Commit, FakeFetcher, Fetcher};
    use crate::registry::project::ProjectFile;
    use crate::registry::scan::{ComponentFiles, ScanContext, ScanReport};
    use crate::registry::signing::{SecretKey, sign};
    use crate::registry::statement::{Digest, Statement};
    use crate::registry::{RegistryError, Result};

    struct Accept;

    impl Scanner for Accept {
        fn scan(
            &self,
            _component: &ComponentFiles<'_>,
            _context: &ScanContext<'_>,
        ) -> Result<ScanReport> {
            Ok(ScanReport::default())
        }
    }

    /// Records where each component file it serves lives on the heap.
    struct Recording {
        inner: FakeFetcher,
        served: RefCell<BTreeMap<String, usize>>,
    }

    impl Fetcher for Recording {
        fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>> {
            self.inner.versions(library)
        }

        fn resolve(&self, library: &LibraryAddress, version: &semver::Version) -> Result<Commit> {
            self.inner.resolve(library, version)
        }

        fn file(&self, library: &LibraryAddress, commit: &Commit, path: &str) -> Result<Vec<u8>> {
            let bytes = self.inner.file(library, commit, path)?;
            self.served
                .borrow_mut()
                .insert(path.to_owned(), bytes.as_ptr() as usize);
            Ok(bytes)
        }
    }

    /// MEM-005: the plan holds the bytes the fetch returned, moved and
    /// never cloned.
    #[test]
    fn mem_audit_the_plan_keeps_the_fetched_files_without_copying_them() {
        let secret = SecretKey::from_bytes([2; 32]);
        let library_json = serde_json::to_vec(&serde_json::json!({
            "namespace": "acme",
            "source": "github.com/acme/acme-ui",
            "version": "1.0.0",
            "framework": "*",
            "publicKey": secret.public_key().encode(),
        }))
        .expect("library.json");
        let manifest = br#"{"name":"acme.widget","files":["widget.html","widget.css"]}"#.to_vec();
        let view = vec![b'v'; 4096];
        let css = vec![b'c'; 4096];
        let statement = Statement {
            library: "github.com/acme/acme-ui".to_owned(),
            version: semver::Version::new(1, 0, 0),
            component: "widget".to_owned(),
            library_json: Digest::of(&library_json),
            manifest: Digest::of(&manifest),
            files: BTreeMap::from([
                ("widget.css".to_owned(), Digest::of(&css)),
                ("widget.html".to_owned(), Digest::of(&view)),
            ]),
        };
        let signature = sign(&secret, &statement.verification_hash()).expect("sign");
        let mut inner = FakeFetcher::default();
        inner.add_version(
            LibraryAddress("github.com/acme/acme-ui".to_owned()),
            semver::Version::new(1, 0, 0),
            BTreeMap::from([
                ("library.json".to_owned(), library_json),
                ("components/widget/manifest.json".to_owned(), manifest),
                ("components/widget/widget.html".to_owned(), view),
                ("components/widget/widget.css".to_owned(), css),
                (
                    "components/widget/manifest.sig".to_owned(),
                    signature.encode().into_bytes(),
                ),
            ]),
        );
        let fetcher = Recording {
            inner,
            served: RefCell::default(),
        };
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\n\n[dependencies]\nsuprnova = { git = \"x\", tag = \"v3.2.1\" }\n",
        )
        .expect("manifest");
        let project = ProjectFile::load(root.path()).expect("project");
        let plan = resolve_with(
            &address::parse("acme/acme-ui/widget").expect("source"),
            &Options::default(),
            &fetcher,
            &project,
            &Accept,
        )
        .map_err(|error: RegistryError| error.to_string())
        .expect("plan");
        let served = fetcher.served.borrow();
        for file in &plan.components[0].files {
            assert_eq!(
                Some(&(file.bytes.as_ptr() as usize)),
                served.get(&format!("components/widget/{}", file.name)),
                "{} was copied for the plan",
                file.name
            );
        }
        assert_eq!(
            Some(&(plan.components[0].manifest_bytes.as_ptr() as usize)),
            served.get("components/widget/manifest.json"),
            "the manifest was copied for the plan"
        );
    }
}
