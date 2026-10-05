//! The project's side of an install: `suprnova.toml` as the provenance
//! record and the key pins (REG-001, REG-013, REG-024), the per-directory
//! install record that detects edits (REG-028), and the lock and journal
//! that make an install atomic (REG-029).
//!
//! `suprnova.toml` is edited with `toml_edit`, never re-serialized: the CLI
//! owns `[live.components."<address>"]` and `[live.libraries."<address>"]`
//! and keeps every other byte, comment and key order as it found them.
//! Each table the CLI writes is one header with dotted keys under it, so
//! replacing one never moves another.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::address::{ComponentAddress, LibraryAddress, SHIPPED_LIBRARY};
use super::library::strict_json_object;
use super::signing::{PublicKey, Signature};
use super::statement::Digest;
use super::{Capability, RegistryError, Result};
use crate::secure_fs;

/// The project file, all lowercase (REG-001).
pub const PROJECT_FILE: &str = "suprnova.toml";

/// The name the CLI used to read and no longer does.
pub const LEGACY_PROJECT_FILE: &str = "Suprnova.toml";

/// The install record beside a component's files (REG-028).
pub const INSTALL_RECORD: &str = ".suprnova-installed.json";

/// The lock `live:add` holds for the whole install (REG-029). It stays on
/// disk between installs: deleting a lock file while another process opens
/// it would let two processes each hold a lock on a different file.
pub const LOCK_FILE: &str = ".suprnova-live.lock";

/// The journal written before an install's first write (REG-029).
pub const JOURNAL_FILE: &str = ".suprnova-live-journal.json";

/// Refuses a project that holds `Suprnova.toml` and no `suprnova.toml`, by
/// its exact directory entries, naming the rename (REG-001).
pub fn refuse_legacy_project_file(root: &Path) -> Result<()> {
    let entries = std::fs::read_dir(root)
        .map_err(|error| RegistryError::Io(format!("cannot read {}: {error}", root.display())))?;
    let mut legacy = false;
    let mut current = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == LEGACY_PROJECT_FILE {
            legacy = true;
        } else if name == PROJECT_FILE {
            current = true;
        }
    }
    if legacy && !current {
        return Err(RegistryError::Invalid(format!(
            "the project file is `{PROJECT_FILE}`, all lowercase; rename `{LEGACY_PROJECT_FILE}` to `{PROJECT_FILE}`"
        )));
    }
    Ok(())
}

/// One `[live.components."<address>"]` table (REG-013): exactly what
/// arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentRecord {
    /// The library's `source`.
    pub source: String,
    /// The library version installed.
    pub version: semver::Version,
    /// The commit the tag resolved to.
    pub commit: String,
    /// The digest of `library.json` as it arrived.
    pub library_json: Digest,
    /// The digest of `manifest.json` as it arrived.
    pub manifest: Digest,
    /// The digest of each named file as it arrived, by file name.
    pub files: BTreeMap<String, Digest>,
    /// The verification hash (REG-023).
    pub hash: Digest,
    /// The signature (REG-024).
    pub signature: Signature,
    /// Each dependency with the version it resolved to (REG-010).
    pub dependencies: BTreeMap<ComponentAddress, semver::Version>,
    /// Each registered type's full path, as the registration line wrote it
    /// (REG-005).
    pub registered: Vec<String>,
    /// Each capability the scan found, with its approval (REG-006).
    pub capabilities: BTreeMap<Capability, Approval>,
    /// Files the install kept as edited (REG-028).
    pub kept: Vec<String>,
}

/// A shipped component's table (REG-013): the CLI's version and the
/// digests of what it wrote, and no hash or signature, since the shipped
/// library is exempt from both (REG-016).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedRecord {
    /// The version of the CLI that installed it.
    pub version: semver::Version,
    /// The digest of its manifest.
    pub manifest: Digest,
    /// The digest of each named file, by file name.
    pub files: BTreeMap<String, Digest>,
}

/// One `[live.libraries."<address>"]` table: the namespace the library
/// owns in this application (REG-011) and the key pinned for it (REG-024).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryRecord {
    /// The namespace its components install under.
    pub namespace: Option<String>,
    /// The pinned key.
    pub public_key: Option<PublicKey>,
}

/// How a capability came to be approved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The developer confirmed it on a terminal.
    Terminal,
    /// `--allow <capability>` named it.
    Flag,
}

impl Approval {
    /// The word `suprnova.toml` records it as.
    pub fn name(self) -> &'static str {
        match self {
            Approval::Terminal => "terminal",
            Approval::Flag => "flag",
        }
    }

    /// The approval a recorded word names.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "terminal" => Some(Approval::Terminal),
            "flag" => Some(Approval::Flag),
            _ => None,
        }
    }
}

/// The project's `suprnova.toml`, read whole and edited in place with
/// every byte it does not own kept (REG-001).
#[derive(Debug)]
pub struct ProjectFile {
    root: PathBuf,
    document: toml_edit::DocumentMut,
}

impl ProjectFile {
    /// Loads the project file, or an empty one when none exists; refuses a
    /// project that holds only the legacy name.
    pub fn load(root: &Path) -> Result<Self> {
        refuse_legacy_project_file(root)?;
        secure_fs::ensure_contained(root, Path::new(PROJECT_FILE)).map_err(RegistryError::Io)?;
        let path = root.join(PROJECT_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(RegistryError::Io(format!(
                    "cannot read {}: {error}",
                    path.display()
                )));
            }
        };
        let document = text.parse::<toml_edit::DocumentMut>().map_err(|error| {
            RegistryError::Invalid(format!("{PROJECT_FILE} is not valid TOML: {error}"))
        })?;
        Ok(ProjectFile {
            root: root.to_path_buf(),
            document,
        })
    }

    /// The project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file as it would be written now.
    pub fn render(&self) -> String {
        self.document.to_string()
    }

    /// Writes the file atomically.
    pub fn save(&self) -> Result<()> {
        secure_fs::write_atomic_under(
            &self.root,
            Path::new(PROJECT_FILE),
            self.render().as_bytes(),
        )
        .map_err(RegistryError::Io)
    }

    fn live_table(&self, name: &str) -> Result<Option<&toml_edit::Table>> {
        let Some(live) = self.document.get("live") else {
            return Ok(None);
        };
        let live = live.as_table().ok_or_else(|| {
            RegistryError::Invalid(format!("{PROJECT_FILE}: `live` is not a table"))
        })?;
        match live.get(name) {
            None => Ok(None),
            Some(item) => item.as_table().map(Some).ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `live.{name}` is not a table"))
            }),
        }
    }

    /// Every installed third-party component's record. A shipped
    /// component's table is read by [`ProjectFile::shipped`].
    pub fn components(&self) -> Result<BTreeMap<ComponentAddress, ComponentRecord>> {
        let mut records = BTreeMap::new();
        let Some(table) = self.live_table("components")? else {
            return Ok(records);
        };
        for (key, item) in table.iter() {
            let address = ComponentAddress::parse(key)?;
            if address.library.is_shipped() {
                continue;
            }
            let entry = item.as_table_like().ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}` is not a table"))
            })?;
            records.insert(address, read_component(key, entry)?);
        }
        Ok(records)
    }

    /// Every installed shipped component's record.
    pub fn shipped(&self) -> Result<BTreeMap<ComponentAddress, ShippedRecord>> {
        let mut records = BTreeMap::new();
        let Some(table) = self.live_table("components")? else {
            return Ok(records);
        };
        for (key, item) in table.iter() {
            let address = ComponentAddress::parse(key)?;
            if !address.library.is_shipped() {
                continue;
            }
            let entry = item.as_table_like().ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}` is not a table"))
            })?;
            records.insert(
                address,
                ShippedRecord {
                    version: version_field(key, entry, "version")?,
                    manifest: digest_field(key, entry, "manifest")?,
                    files: digest_map(key, entry, "files")?,
                },
            );
        }
        Ok(records)
    }

    /// Every library this project has a table for.
    pub fn libraries(&self) -> Result<BTreeMap<LibraryAddress, LibraryRecord>> {
        let mut records = BTreeMap::new();
        let Some(table) = self.live_table("libraries")? else {
            return Ok(records);
        };
        for (key, item) in table.iter() {
            let address = LibraryAddress(key.to_owned());
            address.kind()?;
            let entry = item.as_table_like().ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}` is not a table"))
            })?;
            let namespace = optional_string(key, entry, "namespace")?;
            let public_key = optional_string(key, entry, "key")?
                .map(|text| PublicKey::parse(&text))
                .transpose()?;
            records.insert(
                address,
                LibraryRecord {
                    namespace,
                    public_key,
                },
            );
        }
        Ok(records)
    }

    /// The key pinned for a library (REG-024), if any.
    pub fn pinned_key(&self, library: &LibraryAddress) -> Result<Option<PublicKey>> {
        Ok(self
            .libraries()?
            .remove(library)
            .and_then(|record| record.public_key))
    }

    /// The library an installed namespace belongs to (REG-011), if any.
    pub fn namespace_owner(&self, namespace: &str) -> Result<Option<LibraryAddress>> {
        Ok(self
            .libraries()?
            .into_iter()
            .find(|(_, record)| record.namespace.as_deref() == Some(namespace))
            .map(|(address, _)| address))
    }

    /// Writes a component's record and, when given, a key pin, keeping
    /// every other byte of the file (REG-001, REG-013, REG-024).
    pub fn record(
        &mut self,
        address: &ComponentAddress,
        record: &ComponentRecord,
        pin: Option<(&LibraryAddress, &PublicKey)>,
    ) -> Result<()> {
        self.set_component(address, record)?;
        if let Some((library, key)) = pin {
            self.set_library(library, None, Some(key))?;
        }
        self.save()
    }

    /// Sets a third-party component's table in memory.
    pub fn set_component(
        &mut self,
        address: &ComponentAddress,
        record: &ComponentRecord,
    ) -> Result<()> {
        let mut table = toml_edit::Table::new();
        table.insert("source", toml_edit::value(record.source.as_str()));
        table.insert("version", toml_edit::value(record.version.to_string()));
        if !record.commit.is_empty() {
            table.insert("commit", toml_edit::value(record.commit.as_str()));
        }
        table.insert(
            "library_json",
            toml_edit::value(record.library_json.as_str()),
        );
        table.insert("manifest", toml_edit::value(record.manifest.as_str()));
        table.insert("hash", toml_edit::value(record.hash.as_str()));
        table.insert("signature", toml_edit::value(record.signature.encode()));
        table.insert(
            "files",
            dotted(
                record
                    .files
                    .iter()
                    .map(|(name, digest)| (name.clone(), digest.as_str().to_owned())),
            ),
        );
        if !record.dependencies.is_empty() {
            table.insert(
                "dependencies",
                dotted(
                    record
                        .dependencies
                        .iter()
                        .map(|(address, version)| (address.to_string(), version.to_string())),
                ),
            );
        }
        if !record.registered.is_empty() {
            table.insert("registered", string_array(&record.registered));
        }
        if !record.capabilities.is_empty() {
            table.insert(
                "capabilities",
                dotted(record.capabilities.iter().map(|(capability, approval)| {
                    (capability.name().to_owned(), approval.name().to_owned())
                })),
            );
        }
        if !record.kept.is_empty() {
            table.insert("kept", string_array(&record.kept));
        }
        self.put("components", &address.to_string(), table)
    }

    /// Sets a shipped component's table in memory.
    pub fn set_shipped(
        &mut self,
        address: &ComponentAddress,
        record: &ShippedRecord,
    ) -> Result<()> {
        let mut table = toml_edit::Table::new();
        table.insert("source", toml_edit::value(SHIPPED_LIBRARY));
        table.insert("version", toml_edit::value(record.version.to_string()));
        table.insert("manifest", toml_edit::value(record.manifest.as_str()));
        table.insert(
            "files",
            dotted(
                record
                    .files
                    .iter()
                    .map(|(name, digest)| (name.clone(), digest.as_str().to_owned())),
            ),
        );
        self.put("components", &address.to_string(), table)
    }

    /// Sets a library's namespace and pinned key in memory, keeping what
    /// the call does not name.
    pub fn set_library(
        &mut self,
        library: &LibraryAddress,
        namespace: Option<&str>,
        key: Option<&PublicKey>,
    ) -> Result<()> {
        let mut record = self.libraries()?.remove(library).unwrap_or_default();
        if let Some(namespace) = namespace {
            record.namespace = Some(namespace.to_owned());
        }
        if let Some(key) = key {
            record.public_key = Some(key.clone());
        }
        let mut table = toml_edit::Table::new();
        if let Some(namespace) = &record.namespace {
            table.insert("namespace", toml_edit::value(namespace.as_str()));
        }
        if let Some(key) = &record.public_key {
            table.insert("key", toml_edit::value(key.encode()));
        }
        self.put("libraries", &library.0, table)
    }

    /// Puts one table under `live.<group>`. A table it replaces keeps its
    /// position and the comments above its header; a new one goes after
    /// everything else in the file, taking any comment that ended the file
    /// above its header, so every byte the file held stays where it was.
    fn put(&mut self, group: &str, key: &str, mut table: toml_edit::Table) -> Result<()> {
        let next = max_position(self.document.as_table()) + 1;
        let trailing = self
            .document
            .trailing()
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let live = self
            .document
            .entry("live")
            .or_insert_with(|| implicit_table(next));
        let live = live.as_table_mut().ok_or_else(|| {
            RegistryError::Invalid(format!("{PROJECT_FILE}: `live` is not a table"))
        })?;
        let group_table = live.entry(group).or_insert_with(|| implicit_table(next));
        let group_table = group_table.as_table_mut().ok_or_else(|| {
            RegistryError::Invalid(format!("{PROJECT_FILE}: `live.{group}` is not a table"))
        })?;
        let mut moved_trailing = false;
        match group_table.get(key).and_then(toml_edit::Item::as_table) {
            Some(old) => {
                table.set_position(old.position().or(Some(next)));
                *table.decor_mut() = old.decor().clone();
            }
            None => {
                table.set_position(Some(next));
                if !trailing.is_empty() {
                    table.decor_mut().set_prefix(format!("{trailing}\n"));
                    moved_trailing = true;
                }
            }
        }
        group_table.insert(key, toml_edit::Item::Table(table));
        if moved_trailing {
            self.document.set_trailing("");
        }
        Ok(())
    }
}

fn implicit_table(position: isize) -> toml_edit::Item {
    let mut table = toml_edit::Table::new();
    table.set_implicit(true);
    table.set_position(Some(position));
    toml_edit::Item::Table(table)
}

fn max_position(table: &toml_edit::Table) -> isize {
    let mut max = table.position().unwrap_or(0);
    for (_, item) in table.iter() {
        match item {
            toml_edit::Item::Table(inner) => max = max.max(max_position(inner)),
            toml_edit::Item::ArrayOfTables(array) => {
                for inner in array.iter() {
                    max = max.max(max_position(inner));
                }
            }
            _ => {}
        }
    }
    max
}

fn dotted(entries: impl Iterator<Item = (String, String)>) -> toml_edit::Item {
    let mut table = toml_edit::Table::new();
    table.set_dotted(true);
    for (key, text) in entries {
        table.insert(&key, toml_edit::value(text));
    }
    toml_edit::Item::Table(table)
}

fn string_array(items: &[String]) -> toml_edit::Item {
    toml_edit::value(
        items
            .iter()
            .map(String::as_str)
            .collect::<toml_edit::Array>(),
    )
}

fn field<'a>(
    key: &str,
    entry: &'a dyn toml_edit::TableLike,
    name: &str,
) -> Result<&'a toml_edit::Item> {
    entry
        .get(name)
        .ok_or_else(|| RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}` has no `{name}`")))
}

fn string_field(key: &str, entry: &dyn toml_edit::TableLike, name: &str) -> Result<String> {
    field(key, entry, name)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}`.{name} is not a string"))
        })
}

fn optional_string(
    key: &str,
    entry: &dyn toml_edit::TableLike,
    name: &str,
) -> Result<Option<String>> {
    match entry.get(name) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}`.{name} is not a string"))
            }),
    }
}

fn version_field(
    key: &str,
    entry: &dyn toml_edit::TableLike,
    name: &str,
) -> Result<semver::Version> {
    let text = string_field(key, entry, name)?;
    semver::Version::parse(&text).map_err(|error| {
        RegistryError::Invalid(format!(
            "{PROJECT_FILE}: `{key}`.{name} `{text}` is not semver: {error}"
        ))
    })
}

fn digest_field(key: &str, entry: &dyn toml_edit::TableLike, name: &str) -> Result<Digest> {
    let text = string_field(key, entry, name)?;
    Digest::parse(&text).ok_or_else(|| {
        RegistryError::Invalid(format!(
            "{PROJECT_FILE}: `{key}`.{name} `{text}` is not a sha256 digest"
        ))
    })
}

fn sub_table<'a>(
    key: &str,
    entry: &'a dyn toml_edit::TableLike,
    name: &str,
) -> Result<Option<&'a dyn toml_edit::TableLike>> {
    match entry.get(name) {
        None => Ok(None),
        Some(item) => item.as_table_like().map(Some).ok_or_else(|| {
            RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}`.{name} is not a table"))
        }),
    }
}

fn digest_map(
    key: &str,
    entry: &dyn toml_edit::TableLike,
    name: &str,
) -> Result<BTreeMap<String, Digest>> {
    let mut map = BTreeMap::new();
    if let Some(table) = sub_table(key, entry, name)? {
        for (file, item) in table.iter() {
            let text = item.as_str().unwrap_or_default();
            let digest = Digest::parse(text).ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "{PROJECT_FILE}: `{key}`.{name}.{file} is not a sha256 digest"
                ))
            })?;
            map.insert(file.to_owned(), digest);
        }
    }
    Ok(map)
}

fn string_list(key: &str, entry: &dyn toml_edit::TableLike, name: &str) -> Result<Vec<String>> {
    match entry.get(name) {
        None => Ok(Vec::new()),
        Some(item) => {
            let array = item.as_array().ok_or_else(|| {
                RegistryError::Invalid(format!("{PROJECT_FILE}: `{key}`.{name} is not a list"))
            })?;
            array
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        RegistryError::Invalid(format!(
                            "{PROJECT_FILE}: `{key}`.{name} holds a value that is not a string"
                        ))
                    })
                })
                .collect()
        }
    }
}

fn read_component(key: &str, entry: &dyn toml_edit::TableLike) -> Result<ComponentRecord> {
    let mut dependencies = BTreeMap::new();
    if let Some(table) = sub_table(key, entry, "dependencies")? {
        for (address, item) in table.iter() {
            let text = item.as_str().unwrap_or_default();
            let version = semver::Version::parse(text).map_err(|error| {
                RegistryError::Invalid(format!(
                    "{PROJECT_FILE}: `{key}`.dependencies `{address}` is not semver: {error}"
                ))
            })?;
            dependencies.insert(ComponentAddress::parse(address)?, version);
        }
    }
    let mut capabilities = BTreeMap::new();
    if let Some(table) = sub_table(key, entry, "capabilities")? {
        for (name, item) in table.iter() {
            let capability = Capability::parse(name).ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "{PROJECT_FILE}: `{key}`.capabilities names `{name}`, which is not a capability"
                ))
            })?;
            let approval = item.as_str().and_then(Approval::parse).ok_or_else(|| {
                RegistryError::Invalid(format!(
                    "{PROJECT_FILE}: `{key}`.capabilities.{name} is not `terminal` or `flag`"
                ))
            })?;
            capabilities.insert(capability, approval);
        }
    }
    Ok(ComponentRecord {
        source: string_field(key, entry, "source")?,
        version: version_field(key, entry, "version")?,
        commit: optional_string(key, entry, "commit")?.unwrap_or_default(),
        library_json: digest_field(key, entry, "library_json")?,
        manifest: digest_field(key, entry, "manifest")?,
        files: digest_map(key, entry, "files")?,
        hash: digest_field(key, entry, "hash")?,
        signature: Signature::parse_strict(&string_field(key, entry, "signature")?)?,
        dependencies,
        registered: string_list(key, entry, "registered")?,
        capabilities,
        kept: string_list(key, entry, "kept")?,
    })
}

/// What `live:check` found verifying the recorded components offline
/// (REG-027).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verification {
    /// Components whose recorded signature verified against the pinned key.
    pub verified: Vec<ComponentAddress>,
    /// Each component that failed, with why: a bad signature, a missing
    /// pin, or a record altered by hand.
    pub failures: Vec<(ComponentAddress, String)>,
    /// Installed files whose bytes differ from what arrived. Reported, never
    /// a failure: the application owns its vendored files.
    pub changed: Vec<(ComponentAddress, PathBuf)>,
}

/// Verifies every third-party component `suprnova.toml` records, without
/// the network (REG-027): the recorded signature against the key pinned
/// for its library, over the hash recomputed from the recorded fields, so a
/// field altered by hand fails. Each installed file is compared with the
/// digest of what arrived and reported when it differs.
pub fn verify_installed(root: &Path) -> Result<Verification> {
    let mut verification = Verification::default();
    if std::fs::symlink_metadata(root.join(PROJECT_FILE)).is_err() {
        return Ok(verification);
    }
    let project = ProjectFile::load(root)?;
    let libraries = project.libraries()?;
    for (address, record) in project.components()? {
        let library = libraries.get(&address.library);
        let Some(key) = library.and_then(|library| library.public_key.as_ref()) else {
            verification.failures.push((
                address.clone(),
                format!("no key is pinned for {} in {PROJECT_FILE}", address.library),
            ));
            continue;
        };
        let statement = super::statement::Statement {
            library: record.source.clone(),
            version: record.version.clone(),
            component: address.component.clone(),
            library_json: record.library_json.clone(),
            manifest: record.manifest.clone(),
            files: record.files.clone(),
        };
        let hash = statement.verification_hash();
        if hash != record.hash {
            verification.failures.push((
                address.clone(),
                format!(
                    "the recorded fields hash to {hash}, not the recorded {}; the record was changed after install",
                    record.hash
                ),
            ));
            continue;
        }
        if let Err(error) = super::signing::verify(key, &hash, &record.signature) {
            verification
                .failures
                .push((address.clone(), error.to_string()));
            continue;
        }
        verification.verified.push(address.clone());
        let Some(namespace) = library.and_then(|library| library.namespace.as_deref()) else {
            continue;
        };
        for (name, digest) in &record.files {
            let Some(kind) = super::library::FileKind::of(name) else {
                continue;
            };
            let path = super::plan::destination(namespace, &address.component, name, kind);
            let same = std::fs::read(root.join(&path))
                .map(|bytes| Digest::of(&bytes) == *digest)
                .unwrap_or(false);
            if !same {
                verification.changed.push((address.clone(), path));
            }
        }
    }
    Ok(verification)
}

/// The per-directory record of the upstream bytes last written at each
/// path, keyed by the path from the project root (REG-028).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallRecord {
    /// Each installed path's digest.
    pub digests: BTreeMap<PathBuf, Digest>,
}

/// A path relative to the project root, written with `/` separators: the
/// install record's and the journal's key form on every platform.
pub fn project_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Reads a `/`-separated path from the project root, refusing anything that
/// is not a plain relative path.
fn relative_path(text: &str) -> Option<PathBuf> {
    let valid = !text.is_empty()
        && text
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains('\\'));
    (valid && !text.starts_with('/')).then(|| PathBuf::from(text))
}

impl InstallRecord {
    /// Reads a component directory's record, reading today's bare-name form
    /// as files of that directory (REG-028). A record written in both forms
    /// takes the path-keyed entry, the newer one, for a path both name.
    pub fn load(project_root: &Path, component_dir: &Path) -> Result<Self> {
        let directory = component_dir
            .strip_prefix(project_root)
            .unwrap_or(component_dir)
            .to_path_buf();
        let relative = directory.join(INSTALL_RECORD);
        secure_fs::ensure_contained(project_root, &relative).map_err(RegistryError::Io)?;
        let path = project_root.join(&relative);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(RegistryError::Io(format!(
                    "cannot read {}: {error}",
                    path.display()
                )));
            }
        };
        let damaged = |reason: String| {
            RegistryError::Invalid(format!(
                "cannot decode the install record {}: {reason}; delete it to keep every file that differs from the one live:add would write",
                path.display()
            ))
        };
        let object = strict_json_object(&bytes).map_err(|error| damaged(error.to_string()))?;
        let mut legacy = BTreeMap::new();
        let mut current = BTreeMap::new();
        for (key, value) in object {
            let text = value
                .as_str()
                .ok_or_else(|| damaged(format!("`{key}` is not a digest")))?;
            let digest = Digest::parse(text)
                .or_else(|| Digest::parse(&format!("sha256:{text}")))
                .ok_or_else(|| damaged(format!("`{key}` is not a sha256 digest")))?;
            if key.contains('/') {
                let path = relative_path(&key)
                    .ok_or_else(|| damaged(format!("`{key}` is not a path in the project")))?;
                current.insert(path, digest);
            } else {
                let file = relative_path(&key)
                    .ok_or_else(|| damaged(format!("`{key}` is not a file name")))?;
                legacy.insert(directory.join(file), digest);
            }
        }
        legacy.extend(current);
        Ok(InstallRecord { digests: legacy })
    }

    /// Writes the record in the new form.
    pub fn save(&self, project_root: &Path, component_dir: &Path) -> Result<()> {
        let directory = component_dir
            .strip_prefix(project_root)
            .unwrap_or(component_dir);
        let map: BTreeMap<String, &str> = self
            .digests
            .iter()
            .map(|(path, digest)| (project_path(path), digest.as_str()))
            .collect();
        let mut bytes = serde_json::to_vec_pretty(&map).map_err(|error| {
            RegistryError::Io(format!("cannot encode the install record: {error}"))
        })?;
        bytes.push(b'\n');
        secure_fs::write_atomic_under(project_root, &directory.join(INSTALL_RECORD), &bytes)
            .map_err(RegistryError::Io)
    }

    /// The digest recorded for a path from the project root.
    pub fn digest(&self, path: &Path) -> Option<&Digest> {
        self.digests.get(Path::new(&project_path(path)))
    }
}

/// The exclusive project lock `live:add` holds from before it reads the
/// records until it finishes (REG-029); `serve` waits on it.
#[derive(Debug)]
pub struct ProjectLock {
    path: PathBuf,
    file: std::fs::File,
}

impl ProjectLock {
    /// Takes the lock, refusing when another `live:add` holds it.
    pub fn acquire(project_root: &Path) -> Result<Self> {
        secure_fs::ensure_contained(project_root, Path::new(LOCK_FILE))
            .map_err(RegistryError::Io)?;
        let path = project_root.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                RegistryError::Io(format!("cannot open {}: {error}", path.display()))
            })?;
        match file.try_lock() {
            Ok(()) => Ok(ProjectLock { path, file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(RegistryError::Declined(format!(
                "another live:add is installing into this project (it holds {LOCK_FILE}); run this again when it finishes"
            ))),
            Err(std::fs::TryLockError::Error(error)) => Err(RegistryError::Io(format!(
                "cannot lock {}: {error}",
                path.display()
            ))),
        }
    }

    /// Whether another process holds the lock. Never creates the lock file,
    /// so a dry run can ask (REG-014).
    pub fn is_held(project_root: &Path) -> bool {
        if secure_fs::ensure_contained(project_root, Path::new(LOCK_FILE)).is_err() {
            return false;
        }
        let Ok(file) = std::fs::File::open(project_root.join(LOCK_FILE)) else {
            return false;
        };
        matches!(
            file.try_lock_shared(),
            Err(std::fs::TryLockError::WouldBlock)
        )
    }

    /// The lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Releases the lock now rather than when it is dropped.
    pub fn release(self) -> Result<()> {
        self.file.unlock().map_err(|error| {
            RegistryError::Io(format!("cannot unlock {}: {error}", self.path.display()))
        })
    }
}

/// The journal written before the first write: every path the install will
/// change or create, with the prior bytes of each it changes (REG-029).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Journal {
    /// Paths that will be created, directories before the files in them.
    pub created: Vec<PathBuf>,
    /// Paths that will change, with their prior bytes.
    pub changed: BTreeMap<PathBuf, Vec<u8>>,
}

impl Journal {
    /// Writes the journal into the project.
    pub fn write(&self, project_root: &Path) -> Result<()> {
        let created: Vec<String> = self.created.iter().map(|path| project_path(path)).collect();
        let changed: BTreeMap<String, String> = self
            .changed
            .iter()
            .map(|(path, bytes)| (project_path(path), STANDARD.encode(bytes)))
            .collect();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "created": created,
            "changed": changed,
        }))
        .map_err(|error| RegistryError::Io(format!("cannot encode the journal: {error}")))?;
        secure_fs::write_atomic_under(project_root, Path::new(JOURNAL_FILE), &bytes)
            .map_err(RegistryError::Io)
    }

    /// Whether a journal is on disk.
    pub fn exists(project_root: &Path) -> bool {
        std::fs::symlink_metadata(project_root.join(JOURNAL_FILE)).is_ok()
    }

    /// Reads the journal on disk, if there is one.
    pub fn read(project_root: &Path) -> Result<Option<Self>> {
        secure_fs::ensure_contained(project_root, Path::new(JOURNAL_FILE))
            .map_err(RegistryError::Io)?;
        let path = project_root.join(JOURNAL_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(RegistryError::Io(format!(
                    "cannot read {}: {error}",
                    path.display()
                )));
            }
        };
        let damaged = |reason: &str| {
            RegistryError::Invalid(format!(
                "the install journal {} is damaged: {reason}; restore the files it names by hand, then delete it",
                path.display()
            ))
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| damaged(&error.to_string()))?;
        let mut journal = Journal::default();
        for item in value
            .get("created")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| damaged("no created list"))?
        {
            let path = item
                .as_str()
                .and_then(relative_path)
                .ok_or_else(|| damaged("a created entry is not a path in the project"))?;
            journal.created.push(path);
        }
        for (key, item) in value
            .get("changed")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| damaged("no changed map"))?
        {
            let path = relative_path(key)
                .ok_or_else(|| damaged("a changed entry is not a path in the project"))?;
            let bytes = item
                .as_str()
                .and_then(|text| STANDARD.decode(text).ok())
                .ok_or_else(|| damaged("a changed entry holds no base64 bytes"))?;
            journal.changed.insert(path, bytes);
        }
        Ok(Some(journal))
    }

    /// Puts back every path the journal names: each changed file gets its
    /// prior bytes, each created file is removed, then each created
    /// directory that is empty again.
    pub fn restore_into(&self, project_root: &Path) -> Result<()> {
        let mut failures = Vec::new();
        for (path, bytes) in &self.changed {
            if let Err(error) = secure_fs::write_atomic_under(project_root, path, bytes) {
                failures.push(error);
            }
        }
        for path in self.created.iter().rev() {
            if secure_fs::ensure_contained(project_root, path).is_err() {
                failures.push(format!("{} is not inside the project", path.display()));
                continue;
            }
            let full = project_root.join(path);
            let Ok(entry) = std::fs::symlink_metadata(&full) else {
                continue;
            };
            let removed = if entry.is_dir() {
                match std::fs::remove_dir(&full) {
                    // A directory something else also wrote into stays.
                    Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => Ok(()),
                    other => other,
                }
            } else {
                std::fs::remove_file(&full)
            };
            if let Err(error) = removed {
                failures.push(format!("cannot remove {}: {error}", full.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RegistryError::Io(failures.join("; ")))
        }
    }

    /// Removes the journal file.
    pub fn remove(project_root: &Path) -> Result<()> {
        match std::fs::remove_file(project_root.join(JOURNAL_FILE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(RegistryError::Io(format!(
                "cannot remove {JOURNAL_FILE}: {error}"
            ))),
        }
    }

    /// Restores every file the journal names and removes the journal;
    /// `live:add` and `serve` call this when they find one with no lock held.
    pub fn restore(project_root: &Path) -> Result<bool> {
        let Some(journal) = Journal::read(project_root)? else {
            return Ok(false);
        };
        journal.restore_into(project_root)?;
        Journal::remove(project_root)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::{ProjectFile, ShippedRecord, refuse_legacy_project_file};
    use crate::registry::address::{ComponentAddress, LibraryAddress};
    use crate::registry::signing::SecretKey;
    use crate::registry::statement::Digest;

    fn shipped(version: &str) -> ShippedRecord {
        ShippedRecord {
            version: semver::Version::parse(version).expect("semver"),
            manifest: Digest::of(b"manifest"),
            files: [("field.html".to_owned(), Digest::of(b"view"))].into(),
        }
    }

    fn edited(original: &str) -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("suprnova.toml"), original).expect("write");
        let mut project = ProjectFile::load(dir.path()).expect("load");
        let field = ComponentAddress::parse("suprnova/field").expect("address");
        project.set_shipped(&field, &shipped("3.2.1")).expect("set");
        project
            .set_library(
                &LibraryAddress("github.com/acme/acme-ui".to_owned()),
                Some("acme"),
                Some(&SecretKey::from_bytes([1; 32]).public_key()),
            )
            .expect("library");
        project
            .set_shipped(&field, &shipped("3.2.2"))
            .expect("replace");
        project.render()
    }

    #[test]
    fn every_byte_the_cli_does_not_own_stays_where_it_was() {
        for original in [
            "",
            "# only a comment",
            "# only a comment\n",
            "[serve]\nname = \"x\"",
            "[serve] # trailing\n\n[[serve.process]]\nname = \"q\"  # aligned\ncommand = \"c\"\n\n# the end\n",
            "[a]\nx = 1\n[b]\ny = 2\n[a.inner]\nz = 3\n# after\n",
        ] {
            let text = edited(original);
            assert!(text.starts_with(original), "{original:?} became {text:?}");
            assert!(text.contains("version = \"3.2.2\""), "{text}");
            assert!(!text.contains("3.2.1"), "{text}");
            assert!(text.parse::<toml_edit::DocumentMut>().is_ok(), "{text}");
        }
    }

    #[test]
    fn a_replaced_table_keeps_the_comment_above_its_header() {
        let original = "# my pins\n[live.libraries.\"github.com/acme/acme-ui\"] # pinned by hand\nkey = \"ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\"\n\n[serve]\n";
        let text = edited(original);
        assert!(
            text.starts_with("# my pins\n[live.libraries.\"github.com/acme/acme-ui\"]"),
            "{text}"
        );
        assert!(text.contains("namespace = \"acme\""), "{text}");
        let serve = text.find("[serve]").expect("serve");
        let library = text.find("[live.libraries").expect("library");
        assert!(library < serve, "{text}");
    }

    #[test]
    fn a_project_holding_only_the_legacy_name_is_refused_naming_the_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Suprnova.toml"), "[serve]\n").expect("write");
        let error = refuse_legacy_project_file(dir.path()).expect_err("refused");
        assert!(
            error
                .to_string()
                .contains("rename `Suprnova.toml` to `suprnova.toml`"),
            "{error}"
        );
        std::fs::write(dir.path().join("suprnova.toml"), "").expect("write");
        refuse_legacy_project_file(dir.path()).expect("both present is allowed");
    }
}
