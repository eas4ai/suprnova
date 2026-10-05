//! Authoring a library: `live:registry new`, `check` and `sign` (REG-018,
//! REG-019, REG-020).
//!
//! `new` scaffolds the tree every library has (REG-025): `library.json`, one
//! example component under `components/`, and `preview/`, an application
//! `suprnova new` generates that compiles and renders each component from
//! where it sits. It makes the library's key pair, signs the example, and
//! writes the private key to the author's configuration directory, never
//! into the library.
//!
//! `check` makes every check `live:add` makes for a reason the library alone
//! decides: the tree, each manifest and named file, the signature over the
//! verification hash, the scan, the names, and the dependencies, resolved as
//! `live:add` resolves them. It matches each `register` entry against what
//! the preview registers, and lists each component's capabilities. `sign`
//! makes the same checks but the signature's, then signs every component,
//! verifies each new signature, and writes every `manifest.sig` or none.
//! Nothing a component carries runs: the scan reads it as data.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use super::address::{self, LibraryAddress, Source};
use super::fetch::{Fetcher, HttpsFetcher};
use super::install::RegistrationEdits;
use super::library::{
    self, ComponentManifest, FileKind, LibraryJson, MAX_FILE_BYTES, MAX_JSON_BYTES,
    RESERVED_NAMESPACES,
};
use super::scan::{self, ComponentFiles, ScanReport};
use super::signing::{self, Fingerprint, KeyHandover, PublicKey, SecretKey, Signature};
use super::statement::{Digest, Statement};
use super::{Capability, RegistryError, Result, author_key, registration, scaffold};
use crate::commands::live_add::COMPONENTS;
use crate::ui;

/// The environment variable that names the author's private key file.
pub const KEY_ENV: &str = "SUPRNOVA_LIBRARY_KEY";

const LIBRARY_FILE: &str = "library.json";
const COMPONENTS_DIR: &str = "components";
const MANIFEST_FILE: &str = "manifest.json";
const SIGNATURE_FILE: &str = "manifest.sig";
const PREVIEW_DIR: &str = "preview";

/// A real `manifest.sig` is 88 bytes of base64.
const MAX_SIGNATURE_BYTES: u64 = 1024;

/// The most components one plan installs (REG-010).
const MAX_PLAN_COMPONENTS: usize = 64;

/// The example component's one Live component, as its manifest names it.
const EXAMPLE_REGISTER: &str = "counter::Counter";

/// The version a new library starts at.
const FIRST_VERSION: &str = "0.1.0";

/// Scaffolds a library tree with one example component and a `preview/`
/// application, makes the key pair, and says where the private key is.
pub fn new(namespace: &str, directory: &Path) -> Result<()> {
    let config = author_key::config_dir()?;
    let created = create_library(namespace, directory, &config, &Registry, &HttpsFetcher)?;
    ui::success(&format!(
        "Created the {namespace} library in {}",
        directory.display()
    ));
    ui::hint(
        "library.json, components/counter/ (a view, a stylesheet, a script and a Live component), and preview/, an application that renders each component from where it sits",
    );
    ui::success(&format!(
        "Signed {} with the key {}",
        plural(created.signed, "component"),
        created.fingerprint
    ));
    ui::br();
    ui::warning(&format!(
        "Your private signing key is at {}",
        created.key_path.display()
    ));
    ui::hint(
        "Back it up and never commit it. Losing it strands every pin: an application that pinned this library's key accepts no version another key signs, and nothing else can sign one.",
    );
    ui::hint(&format!(
        "library.json names the library's source as {}; set it to the repository you publish at, then run `suprnova live:registry sign`.",
        created.source
    ));
    ui::br();
    let library = format!("cd {}", directory.display());
    let preview = format!("cd {}", directory.join(PREVIEW_DIR).display());
    ui::panel(
        "Next Steps",
        &[
            &library,
            "suprnova live:registry check",
            &preview,
            "suprnova serve --backend-only",
            "open http://localhost:8765/preview/counter",
        ],
    );
    Ok(())
}

/// Checks every component as `live:add` would for reasons the library
/// alone decides, and lists each component's capabilities.
pub fn check(library_root: &Path) -> Result<()> {
    let inspection = inspect(library_root, true, &Registry, &HttpsFetcher)?;
    print_report(&inspection.report);
    if inspection.report.passed() {
        ui::success(&passed_summary(inspection.report.components.len()));
        Ok(())
    } else {
        Err(RegistryError::Invalid(format!(
            "live:registry check found {}",
            plural(problem_count(&inspection.report), "problem")
        )))
    }
}

/// Runs every check but the signature check, then signs every component,
/// all or nothing.
pub fn sign(library_root: &Path) -> Result<()> {
    let named = std::env::var_os(KEY_ENV).filter(|value| !value.is_empty());
    let config = match author_key::config_dir() {
        Ok(config) => config,
        // A named key needs no configuration directory.
        Err(_) if named.is_some() => PathBuf::new(),
        Err(error) => return Err(error),
    };
    let (inspection, signed) =
        sign_with_key_file(library_root, named, &config, &Registry, &HttpsFetcher)?;
    print_report(&inspection.report);
    let fingerprint = inspection
        .library
        .as_ref()
        .map(|library| library.public_key.fingerprint().to_string())
        .unwrap_or_default();
    ui::success(&format!(
        "Signed {} with the key {fingerprint}",
        plural(signed, "component")
    ));
    Ok(())
}

/// Hands the library to a new signing key (REG-033) and re-signs every
/// component with it, all or nothing. Each fingerprint in `drop` is a
/// former key whose statement is dropped because its private key file is
/// lost.
pub fn rotate_key(library_root: &Path, drop: &[String]) -> Result<()> {
    let mut dropping = BTreeSet::new();
    for text in drop {
        let fingerprint = Fingerprint::parse(text).ok_or_else(|| {
            RegistryError::Invalid(format!(
                "--drop-key {text:?} is not a key fingerprint: `sha256:` and 64 lowercase hex characters, as live:registry prints them"
            ))
        })?;
        dropping.insert(fingerprint);
    }
    let named = std::env::var_os(KEY_ENV).filter(|value| !value.is_empty());
    let config = author_key::config_dir()?;
    let rotated = rotate_with_key_file(
        library_root,
        named,
        &config,
        &dropping,
        &Registry,
        &HttpsFetcher,
    )?;
    ui::success(&format!(
        "Handed the library from the key {} to the key {}",
        rotated.former, rotated.new
    ));
    ui::success(&format!(
        "Signed {} with the new key; library.json is now version {}",
        plural(rotated.signed, "component"),
        rotated.version
    ));
    for fingerprint in &rotated.vouching {
        ui::info(&format!("{fingerprint} vouches for the new key"));
    }
    for key in &rotated.dropped {
        ui::warning(&format!(
            "Dropped the statement of {}. An application still pinned to it refuses this library until its developer pins the new key by hand: in its suprnova.toml, under [live.libraries.\"{}\"] (the library's address there), set `key = \"{}\"` and move the dropped key, \"{}\", into `previous_keys`, so the components recorded under it still verify.",
            key.fingerprint(),
            rotated.source,
            rotated.new_key.encode(),
            key.encode()
        ));
    }
    ui::br();
    ui::warning(&format!(
        "Your new private signing key is at {}",
        rotated.key_path.display()
    ));
    ui::hint(
        "Back it up and keep every former key's file: the next rotation signs a statement with each one, so an application pinned to any of them follows. Losing the new key strands every pin.",
    );
    ui::hint(&format!(
        "Commit library.json and every manifest.sig, then tag v{} to release it.",
        rotated.version
    ));
    Ok(())
}

/// A full dependency address resolved to its library, component and the
/// version it names, if any.
pub(crate) struct RemoteSpec {
    library: LibraryAddress,
    component: String,
    version: Option<semver::Version>,
}

/// The library steps `check` and `sign` share with `live:add`: reading
/// `library.json` and manifests, keys and signatures, the scan, and full
/// addresses. One trait, so the authoring steps here are tested on their
/// own; [`Registry`] is the one the commands use.
pub(crate) trait Tools {
    fn parse_library_json(&self, bytes: &[u8]) -> Result<LibraryJson>;
    fn parse_manifest(
        &self,
        bytes: &[u8],
        directory: &str,
        namespace: &str,
    ) -> Result<ComponentManifest>;
    fn verify(&self, key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()>;
    fn sign(&self, key: &SecretKey, hash: &Digest) -> Result<Signature>;
    fn generate(&self) -> Result<(SecretKey, PublicKey)>;
    fn sign_handover(
        &self,
        former: &SecretKey,
        new_key: &PublicKey,
        library: &str,
    ) -> Result<KeyHandover>;
    fn verify_handover(
        &self,
        handover: &KeyHandover,
        new_key: &PublicKey,
        library: &str,
    ) -> Result<()>;
    /// Scans a component against its manifest: the Live components its
    /// Rust defines must be its `register` entries, and the elements its
    /// scripts define its `elements` (REG-030, REG-032).
    fn scan(
        &self,
        component: &ComponentFiles<'_>,
        manifest: &ComponentManifest,
    ) -> Result<ScanReport>;
    fn resolve_address(&self, spec: &str) -> Result<RemoteSpec>;
}

/// The steps exactly as `live:add` takes them.
struct Registry;

impl Tools for Registry {
    fn parse_library_json(&self, bytes: &[u8]) -> Result<LibraryJson> {
        library::parse_library_json(bytes)
    }

    fn parse_manifest(
        &self,
        bytes: &[u8],
        directory: &str,
        namespace: &str,
    ) -> Result<ComponentManifest> {
        library::parse_manifest(bytes, directory, namespace)
    }

    fn verify(&self, key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()> {
        signing::verify(key, hash, signature)
    }

    fn sign(&self, key: &SecretKey, hash: &Digest) -> Result<Signature> {
        signing::sign(key, hash)
    }

    fn generate(&self) -> Result<(SecretKey, PublicKey)> {
        signing::generate()
    }

    fn sign_handover(
        &self,
        former: &SecretKey,
        new_key: &PublicKey,
        library: &str,
    ) -> Result<KeyHandover> {
        signing::sign_handover(former, new_key, library)
    }

    fn verify_handover(
        &self,
        handover: &KeyHandover,
        new_key: &PublicKey,
        library: &str,
    ) -> Result<()> {
        signing::verify_handover(handover, new_key, library)
    }

    fn scan(
        &self,
        component: &ComponentFiles<'_>,
        manifest: &ComponentManifest,
    ) -> Result<ScanReport> {
        scan::scan_component_with_manifest(
            component,
            &manifest.register,
            &manifest.elements,
            scan::allowlist::embedded()?,
        )
    }

    fn resolve_address(&self, spec: &str) -> Result<RemoteSpec> {
        let source = address::parse(spec)?;
        let version = match &source {
            Source::Repository { version, .. } => version.clone(),
            Source::Url { .. } => None,
            Source::Shipped { .. } | Source::Path { .. } => {
                return Err(RegistryError::Invalid(format!(
                    "`{spec}` is not a full address; a library depends on a shipped name, `./<component>`, or `[<host>/]<owner>/<library>/<component>[@<version>]`"
                )));
            }
        };
        let component = source.component_address()?;
        Ok(RemoteSpec {
            library: component.library,
            component: component.component,
            version,
        })
    }
}

/// What `check` found for one component.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentReport {
    /// The component directory name.
    pub directory: String,
    /// Each capability its scan found, as `live:add` shows them.
    pub capabilities: BTreeSet<Capability>,
    /// Every reason `live:add` would refuse it.
    pub problems: Vec<String>,
}

/// What `check` found for a whole library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryReport {
    /// Problems of the library as a whole.
    pub problems: Vec<String>,
    /// Each component, by directory name.
    pub components: Vec<ComponentReport>,
}

impl LibraryReport {
    /// Whether every check passed.
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
            && self
                .components
                .iter()
                .all(|component| component.problems.is_empty())
    }
}

/// A library read and checked, with the statement of each component that
/// passed: what its signature covers.
struct Inspection {
    report: LibraryReport,
    library: Option<LibraryJson>,
    statements: Vec<(String, Statement)>,
}

/// A component as its directory holds it.
struct LocalComponent {
    manifest: ComponentManifest,
    files: Vec<(String, Vec<u8>)>,
    statement: Statement,
}

/// What `new` made.
struct Created {
    key_path: PathBuf,
    fingerprint: Fingerprint,
    source: String,
    signed: usize,
}

/// Writes a new library, signs it, and writes its private key; on any
/// failure, removes the directory it created and keeps no key.
fn create_library(
    namespace: &str,
    directory: &Path,
    config: &Path,
    tools: &dyn Tools,
    fetcher: &dyn Fetcher,
) -> Result<Created> {
    if !library::valid_namespace(namespace) || RESERVED_NAMESPACES.contains(&namespace) {
        return Err(RegistryError::Invalid(format!(
            "{namespace:?} is not a namespace a library may take: 1 to 32 bytes of lowercase letters, digits and hyphens, starting with a letter, not a Rust keyword, and not suprnova, sn or live"
        )));
    }
    if std::fs::symlink_metadata(directory).is_ok() {
        return Err(RegistryError::Invalid(format!(
            "{} already exists; live:registry new writes a new directory",
            directory.display()
        )));
    }
    let (secret, public) = tools.generate()?;
    let key_path = author_key::key_path_in(config, &public);
    std::fs::create_dir(directory).map_err(|error| {
        RegistryError::Io(format!("cannot create {}: {error}", directory.display()))
    })?;
    let source = placeholder_source(namespace);
    let outcome = write_library(namespace, &source, directory, &public)
        .and_then(|()| {
            let inspection = inspect(directory, false, tools, fetcher)?;
            sign_inspection(directory, &inspection, &secret, &public, tools)
        })
        .and_then(|signed| {
            author_key::refuse_inside_library(directory, &key_path)?;
            author_key::write_key_file(&key_path, &secret, &public)?;
            Ok(signed)
        });
    match outcome {
        Ok(signed) => Ok(Created {
            key_path,
            fingerprint: public.fingerprint(),
            source,
            signed,
        }),
        Err(error) => match std::fs::remove_dir_all(directory) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(RegistryError::Io(format!(
                "{error}; {} could not be removed ({cleanup}) and holds a partial library",
                directory.display()
            ))),
        },
    }
}

/// Writes `library.json`, the example component and the preview.
fn write_library(namespace: &str, source: &str, root: &Path, public: &PublicKey) -> Result<()> {
    write_new(
        &root.join(LIBRARY_FILE),
        &scaffold::library_json(
            namespace,
            source,
            FIRST_VERSION,
            &format!("^{}", env!("CARGO_PKG_VERSION")),
            &public.encode(),
        ),
    )?;
    let component = root.join(COMPONENTS_DIR).join(scaffold::EXAMPLE);
    create_dir_all(&component)?;
    for (name, contents) in scaffold::example_component(namespace) {
        write_new(&component.join(name), &contents)?;
    }
    write_preview(namespace, &library::namespace_module(namespace), root)
}

/// Generates the preview application as `suprnova new` generates an
/// application, then adds what makes it render the library's components
/// from where they sit, with nothing copied and no symbolic link (the Live
/// tooling reads only link-free template roots): `../components/` as a
/// second Askama root with a one-line stub that includes each view, Rust by
/// `#[path]`, stylesheets and scripts by the namespace's asset route over
/// `../components/`, and the registration `live:add` would write.
fn write_preview(namespace: &str, namespace_module: &str, root: &Path) -> Result<()> {
    let preview = root.join(PREVIEW_DIR);
    crate::commands::new::create_project(
        &preview,
        &format!("{namespace_module}_preview"),
        &format!("The preview application of the {namespace} Live component library"),
        "",
        true,
        crate::templates::Frontend::Svelte,
        false,
    )
    .map_err(RegistryError::Io)?;
    std::fs::write(
        preview.join("src/live/mod.rs"),
        scaffold::preview_live_module(namespace, namespace_module),
    )
    .map_err(|error| {
        RegistryError::Io(format!("cannot write the preview's Live module: {error}"))
    })?;
    let namespace_dir = preview.join("src/live").join(namespace_module);
    create_dir_all(&namespace_dir)?;
    write_new(
        &namespace_dir.join("mod.rs"),
        &scaffold::preview_namespace_module(namespace),
    )?;
    write_new(
        &preview.join("src/preview.rs"),
        &scaffold::preview_pages(namespace, namespace_module),
    )?;
    let lib_path = preview.join("src/lib.rs");
    let lib = std::fs::read_to_string(&lib_path).map_err(|error| {
        RegistryError::Io(format!("cannot read {}: {error}", lib_path.display()))
    })?;
    let with_preview = lib.replacen(
        "pub mod routes;\n",
        "pub mod preview;\npub mod routes;\n",
        1,
    );
    if with_preview == lib {
        return Err(RegistryError::Invalid(
            "the generated src/lib.rs has no `pub mod routes;` to declare `pub mod preview;` beside"
                .to_owned(),
        ));
    }
    std::fs::write(&lib_path, with_preview).map_err(|error| {
        RegistryError::Io(format!("cannot write {}: {error}", lib_path.display()))
    })?;
    write_new(
        &preview.join("askama.toml"),
        &scaffold::preview_askama_toml(namespace),
    )?;
    let page_dir = preview.join("templates/_preview");
    create_dir_all(&page_dir)?;
    write_new(
        &page_dir.join("page.html"),
        &scaffold::preview_page_view(namespace),
    )?;
    let stub_dir = preview
        .join("templates")
        .join(format!("{namespace}-ui"))
        .join(scaffold::EXAMPLE);
    create_dir_all(&stub_dir)?;
    for (name, _) in scaffold::example_component(namespace) {
        if name.ends_with(".html") {
            write_new(
                &stub_dir.join(name),
                &scaffold::preview_view_stub(namespace, scaffold::EXAMPLE, name),
            )?;
        }
    }
    let tests = preview.join("tests");
    create_dir_all(&tests)?;
    write_new(
        &tests.join("counter.rs"),
        &scaffold::preview_counter_test(namespace, &format!("{namespace_module}_preview")),
    )?;
    match registration::edits(
        &preview,
        namespace_module,
        &[],
        &[EXAMPLE_REGISTER.to_owned()],
        &[],
    )? {
        RegistrationEdits::Write(files) => {
            for (path, source) in files {
                let target = preview.join(&path);
                std::fs::write(&target, source).map_err(|error| {
                    RegistryError::Io(format!("cannot write {}: {error}", target.display()))
                })?;
            }
            Ok(())
        }
        RegistrationEdits::Report(lines) => Err(RegistryError::Invalid(format!(
            "the preview's registry is not in the scaffold's form: {}",
            lines.join("; ")
        ))),
    }
}

fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .map_err(|error| RegistryError::Io(format!("cannot create {}: {error}", path.display())))
}

/// Writes a file that must not exist yet.
fn write_new(path: &Path, contents: &str) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(|error| RegistryError::Io(format!("cannot write {}: {error}", path.display())))
}

/// What `rotate-key` did.
struct Rotated {
    /// The key the library named before.
    former: Fingerprint,
    /// The key it names now.
    new: Fingerprint,
    /// Where the new private key is.
    key_path: PathBuf,
    /// The version `library.json` names now.
    version: semver::Version,
    /// How many components were signed with the new key.
    signed: usize,
    /// Each key that signed a statement naming the new key, in
    /// `previousKeys` order.
    vouching: Vec<Fingerprint>,
    /// Each former key whose statement was dropped.
    dropped: Vec<PublicKey>,
    /// The new key, as `library.json` and a pin write it.
    new_key: PublicKey,
    /// The library's `source`, the address a published library is pinned
    /// under.
    source: String,
}

/// Hands a library to a new key (REG-033). Every former key `library.json`
/// lists and the current key each sign a statement naming the new key,
/// because a handover is one hop: an application pinned to any of them then
/// follows to the new key. A former key whose private key file is not in
/// the configuration directory is refused, by fingerprint, unless `drop`
/// names it. Then every component is re-signed with the new key, the
/// version advances one patch, since the same version with other signatures
/// is a changed release that `live:add` refuses (REG-026), and the new key,
/// `library.json` and every `manifest.sig` are written, all or none.
fn rotate_with_key_file(
    root: &Path,
    named: Option<OsString>,
    config: &Path,
    drop: &BTreeSet<Fingerprint>,
    tools: &dyn Tools,
    fetcher: &dyn Fetcher,
) -> Result<Rotated> {
    let inspection = inspect(root, false, tools, fetcher)?;
    let library = refuse_problems(&inspection, "rotated")?;
    let placeholder = placeholder_source(&library.namespace);
    if library.source == placeholder {
        return Err(RegistryError::Invalid(format!(
            "library.json names the source {placeholder}, the one live:registry new writes; set it to the repository you publish at first, because every handover statement names it. Nothing was rotated."
        )));
    }
    let current_fingerprint = library.public_key.fingerprint();
    let key_path = author_key::key_file_for(root, &library.public_key, named, config)?;
    let (current_secret, current_public) = author_key::read_key_file(&key_path)?;
    if current_public != library.public_key {
        return Err(RegistryError::Invalid(format!(
            "the key at {} is {}, but library.json names {current_fingerprint}; nothing was rotated",
            key_path.display(),
            current_public.fingerprint()
        )));
    }

    let mut former_keys: Vec<PublicKey> = Vec::new();
    for handover in &library.previous_keys {
        if handover.from != library.public_key && !former_keys.contains(&handover.from) {
            former_keys.push(handover.from.clone());
        }
    }
    let former_fingerprints: BTreeSet<Fingerprint> =
        former_keys.iter().map(PublicKey::fingerprint).collect();
    for fingerprint in drop {
        if !former_fingerprints.contains(fingerprint) {
            return Err(RegistryError::Invalid(format!(
                "--drop-key {fingerprint} names no former key of library.json's previousKeys; nothing was rotated"
            )));
        }
    }
    let mut signers: Vec<(PublicKey, SecretKey)> = Vec::new();
    let mut dropped: Vec<PublicKey> = Vec::new();
    let mut missing = Vec::new();
    for key in former_keys {
        let fingerprint = key.fingerprint();
        if drop.contains(&fingerprint) {
            dropped.push(key);
            continue;
        }
        let path = author_key::key_path_in(config, &key);
        if std::fs::symlink_metadata(&path).is_err() {
            missing.push(format!(
                "the former key {fingerprint} has no private key file at {}",
                path.display()
            ));
            continue;
        }
        let (secret, public) = author_key::read_key_file(&path)?;
        if public != key {
            return Err(RegistryError::Invalid(format!(
                "{} holds the key {}, not {fingerprint}; nothing was rotated",
                path.display(),
                public.fingerprint()
            )));
        }
        signers.push((key, secret));
    }
    if !missing.is_empty() {
        return Err(RegistryError::Invalid(format!(
            "{}. Each former key signs a statement naming the new key, so an application still pinned to it can follow. Restore its file, or pass --drop-key <fingerprint> to drop its statement: an application still pinned to a dropped key refuses the library until its developer pins the new key by hand. Nothing was rotated.",
            missing.join("; ")
        )));
    }
    signers.push((library.public_key.clone(), current_secret));

    let (new_secret, new_public) = tools.generate()?;
    if signers.iter().any(|(key, _)| key == &new_public) || dropped.contains(&new_public) {
        return Err(RegistryError::Invalid(
            "the new key is one the library already used; nothing was rotated".to_owned(),
        ));
    }
    let mut handovers = Vec::new();
    for (public, secret) in &signers {
        let handover = tools.sign_handover(secret, &new_public, &library.source)?;
        if &handover.from != public {
            return Err(RegistryError::Invalid(format!(
                "the key file for {} holds a private key that is not that key's; nothing was rotated",
                public.fingerprint()
            )));
        }
        tools.verify_handover(&handover, &new_public, &library.source)?;
        handovers.push(handover);
    }

    let version = semver::Version::new(
        library.version.major,
        library.version.minor,
        library.version.patch.saturating_add(1),
    );
    let bytes = rotated_library_json(root, &new_public, &handovers, &version)?;
    let rotated = tools.parse_library_json(&bytes)?;
    if rotated.public_key != new_public
        || rotated.version != version
        || rotated.previous_keys.len() != handovers.len()
    {
        return Err(RegistryError::Invalid(
            "the rewritten library.json does not read back as written; nothing was rotated"
                .to_owned(),
        ));
    }
    let library_digest = Digest::of(&bytes);
    let statements: Vec<(String, Statement)> = inspection
        .statements
        .iter()
        .map(|(directory, statement)| {
            let mut statement = statement.clone();
            statement.library_json = library_digest.clone();
            statement.version = version.clone();
            (directory.clone(), statement)
        })
        .collect();
    let mut writes = signature_writes(root, &statements, &new_secret, &new_public, tools)?;
    let signed = writes.len();
    writes.push((root.join(LIBRARY_FILE), bytes));

    let new_key_path = author_key::key_path_in(config, &new_public);
    author_key::refuse_inside_library(root, &new_key_path)?;
    author_key::write_key_file(&new_key_path, &new_secret, &new_public)?;
    if let Err(error) = write_all_or_nothing(&writes) {
        // The library still names the former key, so the new one signs
        // nothing anyone pinned: keeping it would only invite a mix-up.
        return Err(match std::fs::remove_file(&new_key_path) {
            Ok(()) => error,
            Err(cleanup) => RegistryError::Io(format!(
                "{error}; the unused new key at {} could not be removed ({cleanup})",
                new_key_path.display()
            )),
        });
    }
    Ok(Rotated {
        former: current_fingerprint,
        new: new_public.fingerprint(),
        key_path: new_key_path,
        version,
        signed,
        vouching: signers.iter().map(|(key, _)| key.fingerprint()).collect(),
        dropped,
        new_key: new_public,
        source: library.source.clone(),
    })
}

/// The library's `library.json` with the new key, the handover statements
/// and the next version; every other value is kept as written.
fn rotated_library_json(
    root: &Path,
    new_key: &PublicKey,
    handovers: &[KeyHandover],
    version: &semver::Version,
) -> Result<Vec<u8>> {
    let path = root.join(LIBRARY_FILE);
    let raw = read_regular(&path, MAX_JSON_BYTES as u64)
        .map_err(|problem| RegistryError::Io(format!("{LIBRARY_FILE} {problem}")))?;
    let object = library::strict_json_object(&raw)?;
    let text = |key: &str| object.get(key).and_then(serde_json::Value::as_str);
    let required = |key: &str| {
        text(key)
            .ok_or_else(|| RegistryError::Invalid(format!("{LIBRARY_FILE} has no `{key}` string")))
    };
    let encoded: Vec<(String, String, String)> = handovers
        .iter()
        .map(|handover| {
            (
                handover.from.encode(),
                handover.to.as_str().to_owned(),
                handover.signature.encode(),
            )
        })
        .collect();
    let previous_keys: Vec<scaffold::PreviousKeyText<'_>> = encoded
        .iter()
        .map(|(public_key, next, signature)| scaffold::PreviousKeyText {
            public_key,
            next,
            signature,
        })
        .collect();
    let version = version.to_string();
    let public_key = new_key.encode();
    Ok(scaffold::library_json_text(&scaffold::LibraryText {
        namespace: required("namespace")?,
        source: required("source")?,
        version: &version,
        framework: required("framework")?,
        public_key: &public_key,
        previous_keys: &previous_keys,
        title: text("title"),
        description: text("description"),
    })
    .into_bytes())
}

/// The source `live:registry new` writes until the author names where the
/// library is published.
fn placeholder_source(namespace: &str) -> String {
    format!("github.com/{namespace}/{namespace}")
}

/// Signs every component of a library with the key file `named` names, or
/// the library's key under `config`.
fn sign_with_key_file(
    root: &Path,
    named: Option<OsString>,
    config: &Path,
    tools: &dyn Tools,
    fetcher: &dyn Fetcher,
) -> Result<(Inspection, usize)> {
    let inspection = inspect(root, false, tools, fetcher)?;
    let library = refuse_problems(&inspection, "signed")?;
    let key_path = author_key::key_file_for(root, &library.public_key, named, config)?;
    let (secret, public) = author_key::read_key_file(&key_path)?;
    let signed = sign_inspection(root, &inspection, &secret, &public, tools)?;
    Ok((inspection, signed))
}

/// The library of an inspection that found nothing to refuse.
fn refuse_problems<'a>(inspection: &'a Inspection, what: &str) -> Result<&'a LibraryJson> {
    match &inspection.library {
        Some(library) if inspection.report.passed() => Ok(library),
        _ => Err(RegistryError::Invalid(format!(
            "nothing was {what}:\n{}",
            report_lines(&inspection.report)
        ))),
    }
}

/// Signs every component the inspection read, verifies each new signature,
/// and writes them all or none.
fn sign_inspection(
    root: &Path,
    inspection: &Inspection,
    secret: &SecretKey,
    public: &PublicKey,
    tools: &dyn Tools,
) -> Result<usize> {
    let library = refuse_problems(inspection, "signed")?;
    if &library.public_key != public {
        return Err(RegistryError::Invalid(format!(
            "the signing key is {}, but library.json names {}; sign with the key library.json names, or name this one there as publicKey",
            public.fingerprint(),
            library.public_key.fingerprint()
        )));
    }
    let writes = signature_writes(root, &inspection.statements, secret, public, tools)?;
    write_all_or_nothing(&writes)?;
    Ok(writes.len())
}

/// Signs each statement with `secret` and verifies the signature with
/// `public`, returning each component's `manifest.sig` path and bytes; a
/// signature that does not verify refuses them all.
fn signature_writes(
    root: &Path,
    statements: &[(String, Statement)],
    secret: &SecretKey,
    public: &PublicKey,
    tools: &dyn Tools,
) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut writes = Vec::new();
    for (directory, statement) in statements {
        let hash = statement.verification_hash();
        let signature = tools.sign(secret, &hash)?;
        tools.verify(public, &hash, &signature).map_err(|error| {
            RegistryError::Invalid(format!(
                "the new signature of {directory} does not verify ({error}); nothing was signed"
            ))
        })?;
        writes.push((
            root.join(COMPONENTS_DIR)
                .join(directory)
                .join(SIGNATURE_FILE),
            signature.encode().into_bytes(),
        ));
    }
    Ok(writes)
}

/// Writes every file, all or none: each new file is written beside its
/// target first, and only then moved into place; a move that fails puts
/// back every file already moved. A target that is anything but a regular
/// file or absent is refused before anything moves.
fn write_all_or_nothing(writes: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    struct Pending {
        target: PathBuf,
        temporary: PathBuf,
        prior: Option<Vec<u8>>,
    }
    fn discard(pending: &[Pending]) {
        for item in pending {
            let _ = std::fs::remove_file(&item.temporary);
        }
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (target, contents) in writes {
        let target = target.clone();
        let Some(dir) = target.parent() else {
            discard(&pending);
            return Err(RegistryError::Invalid(format!(
                "{} has no directory; nothing was written",
                target.display()
            )));
        };
        let file_name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let prior = match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_file() => match std::fs::read(&target) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    discard(&pending);
                    return Err(RegistryError::Io(format!(
                        "cannot read {}: {error}; nothing was written",
                        target.display()
                    )));
                }
            },
            Ok(_) => {
                discard(&pending);
                return Err(RegistryError::Invalid(format!(
                    "{} is not a regular file; nothing was written",
                    target.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                discard(&pending);
                return Err(RegistryError::Io(format!(
                    "cannot read {}: {error}; nothing was written",
                    target.display()
                )));
            }
        };
        let temporary = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&temporary);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| {
                file.write_all(contents)?;
                file.sync_all()
            });
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temporary);
            discard(&pending);
            return Err(RegistryError::Io(format!(
                "cannot write {}: {error}; nothing was written",
                temporary.display()
            )));
        }
        pending.push(Pending {
            target,
            temporary,
            prior,
        });
    }
    for (index, item) in pending.iter().enumerate() {
        if let Err(error) = std::fs::rename(&item.temporary, &item.target) {
            let mut unrestored = Vec::new();
            for moved in pending[..index].iter().rev() {
                let restored = match &moved.prior {
                    Some(bytes) => std::fs::write(&moved.target, bytes),
                    None => std::fs::remove_file(&moved.target),
                };
                if let Err(restore) = restored {
                    unrestored.push(format!("{} ({restore})", moved.target.display()));
                }
            }
            discard(&pending[index..]);
            let outcome = if unrestored.is_empty() {
                "every file was put back as it was".to_owned()
            } else {
                format!(
                    "these could not be put back and need attention: {}",
                    unrestored.join(", ")
                )
            };
            return Err(RegistryError::Io(format!(
                "cannot write {}: {error}; {outcome}",
                item.target.display()
            )));
        }
    }
    Ok(())
}

/// Reads and checks a library: `library.json`, every component directory,
/// and everything `live:add` checks of each component for a reason the
/// library alone decides. `signatures` decides whether each `manifest.sig`
/// is verified: `check` verifies them, `sign` is about to replace them.
fn inspect(
    root: &Path,
    signatures: bool,
    tools: &dyn Tools,
    fetcher: &dyn Fetcher,
) -> Result<Inspection> {
    let mut report = LibraryReport::default();
    let library_bytes = match read_regular(&root.join(LIBRARY_FILE), MAX_JSON_BYTES as u64) {
        Ok(bytes) => bytes,
        Err(problem) => {
            report.problems.push(format!(
                "{LIBRARY_FILE} {problem}; run this in the library's root"
            ));
            return Ok(Inspection {
                report,
                library: None,
                statements: Vec::new(),
            });
        }
    };
    let library = match tools.parse_library_json(&library_bytes) {
        Ok(library) => library,
        Err(error) => {
            report.problems.push(format!("{LIBRARY_FILE}: {error}"));
            return Ok(Inspection {
                report,
                library: None,
                statements: Vec::new(),
            });
        }
    };
    if !library::valid_namespace(&library.namespace)
        || RESERVED_NAMESPACES.contains(&library.namespace.as_str())
    {
        report.problems.push(format!(
            "{LIBRARY_FILE}: {:?} is not a namespace a third-party library may take",
            library.namespace
        ));
        return Ok(Inspection {
            report,
            library: Some(library),
            statements: Vec::new(),
        });
    }
    let directories = match component_directories(root) {
        Ok((directories, problems)) => {
            report.problems.extend(problems);
            directories
        }
        Err(problem) => {
            report.problems.push(problem);
            return Ok(Inspection {
                report,
                library: Some(library),
                statements: Vec::new(),
            });
        }
    };

    let library_digest = Digest::of(&library_bytes);
    let mut reports = Vec::new();
    let mut components: Vec<Option<LocalComponent>> = Vec::new();
    for directory in &directories {
        let (component_report, component) = read_component(
            root,
            directory,
            &library,
            &library_digest,
            signatures,
            tools,
        );
        reports.push(component_report);
        components.push(component);
    }

    let namespace_module = library::namespace_module(&library.namespace);
    // REG-003: two components of one library never install the same path,
    // and only Rust files share a directory.
    let mut rust_files: BTreeMap<&str, &str> = BTreeMap::new();
    for (component_report, component) in reports.iter_mut().zip(&components) {
        let Some(component) = component else {
            continue;
        };
        for (name, _) in &component.files {
            if FileKind::of(name) != Some(FileKind::Rust) {
                continue;
            }
            match rust_files.get(name.as_str()) {
                Some(first) => component_report.problems.push(format!(
                    "{name} is also a file of {first}: both would install src/live/{namespace_module}/{name}"
                )),
                None => {
                    rust_files.insert(name, &component_report.directory);
                }
            }
        }
    }

    let preview_registrations = if components
        .iter()
        .flatten()
        .any(|component| !component.manifest.register.is_empty())
    {
        match registration::registered_components(&root.join(PREVIEW_DIR)) {
            Ok(Some(registered)) => Some(registered),
            Ok(None) => {
                report.problems.push(format!(
                    "{PREVIEW_DIR}/src/live/mod.rs is missing or its registry() is not in the scaffold's form, so no register entry can be matched against the preview"
                ));
                None
            }
            Err(error) => {
                report
                    .problems
                    .push(format!("{PREVIEW_DIR}/src/live/mod.rs: {error}"));
                None
            }
        }
    } else {
        None
    };

    let local: BTreeMap<&str, &LocalComponent> = directories
        .iter()
        .zip(&components)
        .filter_map(|(directory, component)| {
            component
                .as_ref()
                .map(|component| (directory.as_str(), component))
        })
        .collect();
    let shipped_views = shipped_views();
    for (component_report, component) in reports.iter_mut().zip(&components) {
        let Some(component) = component else {
            continue;
        };
        let directory = component_report.directory.clone();
        let mut plan = Plan::new(tools, fetcher, &library, &local);
        plan.visited_local.insert(directory.clone());
        let mut dependency_modules = Vec::new();
        let mut importable_views = shipped_views.clone();
        for dependency in &component.manifest.dependencies {
            if let Some(reached) = plan.resolve(&Origin::Local, &directory, dependency) {
                dependency_modules.extend(reached.modules);
                importable_views.extend(reached.views);
            }
        }
        component_report.problems.extend(plan.problems);

        let own_modules: BTreeSet<&str> = component
            .files
            .iter()
            .filter_map(|(name, _)| name.strip_suffix(".rs"))
            .collect();
        for entry in &component.manifest.register {
            let module = entry.split("::").next().unwrap_or_default();
            if !own_modules.contains(module) {
                component_report.problems.push(format!(
                    "register entry {entry} names the module {module}, which is not one of the component's Rust files"
                ));
            }
        }

        let files = ComponentFiles {
            namespace: &library.namespace,
            directory: &directory,
            files: &component.files,
            dependency_modules: &dependency_modules,
            importable_views: &importable_views,
        };
        // The scan holds the Rust to `register` and the scripts to
        // `elements`, as `live:add`'s does.
        match tools.scan(&files, &component.manifest) {
            Ok(scan) => {
                component_report.capabilities = scan.capabilities;
                component_report
                    .problems
                    .extend(scan.findings.iter().map(ToString::to_string));
            }
            Err(error) => component_report
                .problems
                .push(format!("the scan failed: {error}")),
        }

        if let Some(registered) = &preview_registrations {
            for entry in &component.manifest.register {
                let full = format!("crate::live::{namespace_module}::{entry}");
                if !registered.contains(&full) {
                    component_report.problems.push(format!(
                        "the preview does not register {full}; add `.register::<{full}>()?` to {PREVIEW_DIR}/src/live/mod.rs, the line live:add writes into an application"
                    ));
                }
            }
        }
    }

    let statements = reports
        .iter()
        .zip(&components)
        .filter_map(|(component_report, component)| match component {
            Some(component) if component_report.problems.is_empty() => Some((
                component_report.directory.clone(),
                component.statement.clone(),
            )),
            _ => None,
        })
        .collect();
    report.components = reports;
    Ok(Inspection {
        report,
        library: Some(library),
        statements,
    })
}

/// Every component directory under `components/`, sorted, with a problem
/// for each entry that cannot be one. Dot entries are not components.
fn component_directories(root: &Path) -> std::result::Result<(Vec<String>, Vec<String>), String> {
    let components = root.join(COMPONENTS_DIR);
    match std::fs::symlink_metadata(&components) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "{COMPONENTS_DIR}/ is a symbolic link; a library's components sit in its own tree"
            ));
        }
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(format!("{COMPONENTS_DIR} is not a directory")),
        Err(_) => {
            return Err(format!(
                "{COMPONENTS_DIR}/ is missing; every component is a directory under it"
            ));
        }
    }
    let entries = std::fs::read_dir(&components)
        .map_err(|error| format!("cannot read {COMPONENTS_DIR}/: {error}"))?;
    let mut directories = Vec::new();
    let mut problems = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {COMPONENTS_DIR}/: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            problems.push(format!(
                "{COMPONENTS_DIR}/{} is not a component directory name",
                name.to_string_lossy()
            ));
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if !library::valid_directory_name(name) {
            problems.push(format!(
                "{COMPONENTS_DIR}/{name:?} is not a component directory name: 1 to 64 bytes of lowercase letters, digits and hyphens, neither starting nor ending with a hyphen"
            ));
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot read {COMPONENTS_DIR}/{name}: {error}"))?;
        if file_type.is_symlink() {
            problems.push(format!(
                "{COMPONENTS_DIR}/{name} is a symbolic link; a component is a directory of the library"
            ));
        } else if !file_type.is_dir() {
            problems.push(format!("{COMPONENTS_DIR}/{name} is not a directory"));
        } else {
            directories.push(name.to_owned());
        }
    }
    directories.sort();
    if directories.is_empty() && problems.is_empty() {
        problems.push(format!("{COMPONENTS_DIR}/ holds no component"));
    }
    Ok((directories, problems))
}

/// Reads one component: its manifest, every file it names, its
/// verification hash, and, when asked, its signature's verdict.
fn read_component(
    root: &Path,
    directory: &str,
    library: &LibraryJson,
    library_digest: &Digest,
    signatures: bool,
    tools: &dyn Tools,
) -> (ComponentReport, Option<LocalComponent>) {
    let mut report = ComponentReport {
        directory: directory.to_owned(),
        ..ComponentReport::default()
    };
    let dir = root.join(COMPONENTS_DIR).join(directory);
    let manifest_bytes = match read_regular(&dir.join(MANIFEST_FILE), MAX_JSON_BYTES as u64) {
        Ok(bytes) => bytes,
        Err(problem) => {
            report.problems.push(format!("{MANIFEST_FILE} {problem}"));
            return (report, None);
        }
    };
    let manifest = match tools.parse_manifest(&manifest_bytes, directory, &library.namespace) {
        Ok(manifest) => manifest,
        Err(error) => {
            report.problems.push(format!("{MANIFEST_FILE}: {error}"));
            return (report, None);
        }
    };
    let mut files = Vec::new();
    for name in &manifest.files {
        if !plain_file_name(name) {
            report.problems.push(format!(
                "{name:?} is not a file name in the component's directory"
            ));
            continue;
        }
        match read_regular(&dir.join(name), MAX_FILE_BYTES as u64) {
            Ok(bytes) => files.push((name.clone(), bytes)),
            Err(problem) => report.problems.push(format!("{name} {problem}")),
        }
    }
    if !report.problems.is_empty() {
        return (report, None);
    }
    let statement = Statement {
        library: library.source.clone(),
        version: library.version.clone(),
        component: directory.to_owned(),
        library_json: library_digest.clone(),
        manifest: Digest::of(&manifest_bytes),
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), Digest::of(bytes)))
            .collect(),
    };
    if signatures
        && let Err(problem) = verify_signature(&dir, library, &statement.verification_hash(), tools)
    {
        report.problems.push(problem);
    }
    (
        report,
        Some(LocalComponent {
            manifest,
            files,
            statement,
        }),
    )
}

/// Verifies a component's `manifest.sig` against the key `library.json`
/// names.
fn verify_signature(
    dir: &Path,
    library: &LibraryJson,
    hash: &Digest,
    tools: &dyn Tools,
) -> std::result::Result<(), String> {
    let stale = "run `suprnova live:registry sign`";
    let bytes = read_regular(&dir.join(SIGNATURE_FILE), MAX_SIGNATURE_BYTES)
        .map_err(|problem| format!("{SIGNATURE_FILE} {problem}; {stale}"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| format!("{SIGNATURE_FILE} is not base64 text; {stale}"))?;
    let signature =
        Signature::parse(text).map_err(|error| format!("{SIGNATURE_FILE}: {error}; {stale}"))?;
    tools
        .verify(&library.public_key, hash, &signature)
        .map_err(|error| {
            format!(
                "{SIGNATURE_FILE} does not verify against the key library.json names ({error}): the component changed since it was signed, or another key signed it; {stale}"
            )
        })
}

/// Reads a file that must be a regular file, never a symbolic link
/// (UI-023), of at most `max` bytes. The problem reads after the file's
/// name.
fn read_regular(path: &Path, max: u64) -> std::result::Result<Vec<u8>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("is missing".to_owned());
        }
        Err(error) => return Err(format!("cannot be read: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err(
            "is a symbolic link; every file of a library is a regular file inside it".to_owned(),
        );
    }
    if !metadata.is_file() {
        return Err("is not a regular file".to_owned());
    }
    let too_large = || format!("is larger than {max} bytes");
    if metadata.len() > max {
        return Err(too_large());
    }
    let file = std::fs::File::open(path).map_err(|error| format!("cannot be read: {error}"))?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot be read: {error}"))?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    Ok(bytes)
}

/// A name that stays in its component's directory: no separator, no
/// leading dot, nothing empty. The manifest parser holds names to REG-003;
/// this guards the path join on its own.
fn plain_file_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0', ':'])
}

/// Every view of the shipped library, as a view includes it.
fn shipped_views() -> Vec<String> {
    COMPONENTS
        .iter()
        .flat_map(|component| {
            component
                .files
                .iter()
                .filter(|(name, _)| name.ends_with(".html"))
                .map(move |(name, _)| format!("suprnova-ui/{}/{name}", component.directory))
        })
        .collect()
}

/// What a component that depends on another may name of it.
#[derive(Debug, Clone, Default)]
struct Reached {
    /// Its Rust modules, as paths under `crate::live::`.
    modules: Vec<String>,
    /// Its views, as a view includes them.
    views: Vec<String>,
}

/// Where a dependency is written, which decides what `./` means.
enum Origin {
    /// A component of the library being checked.
    Local,
    /// A component of a fetched library at one version.
    Remote {
        library: LibraryAddress,
        version: semver::Version,
    },
}

/// The kinds of dependency REG-010 names.
enum Dependency {
    /// A bare name: a component of the shipped library.
    Shipped(String),
    /// `./<component>`: a component of the same library at the same version.
    Sibling(String),
    /// A full address.
    Full(String),
}

fn classify(spec: &str) -> std::result::Result<Dependency, String> {
    if let Some(name) = spec.strip_prefix("./") {
        return if library::valid_directory_name(name) {
            Ok(Dependency::Sibling(name.to_owned()))
        } else {
            Err("`./` names a component directory of the same library".to_owned())
        };
    }
    if library::valid_directory_name(spec) {
        return Ok(Dependency::Shipped(spec.to_owned()));
    }
    if spec.contains('/') && !spec.starts_with(['.', '/']) {
        return Ok(Dependency::Full(spec.to_owned()));
    }
    Err("is not a shipped component's name, `./<component>`, or a full address".to_owned())
}

/// One component's dependencies, resolved as `live:add` resolves them
/// (REG-010): each at most once, one version per component address, at
/// most 64 components, every remote component fetched and verified.
struct Plan<'a> {
    tools: &'a dyn Tools,
    fetcher: &'a dyn Fetcher,
    library: &'a LibraryJson,
    local: &'a BTreeMap<&'a str, &'a LocalComponent>,
    visited_local: BTreeSet<String>,
    remote_versions: BTreeMap<String, semver::Version>,
    reached: BTreeMap<String, Reached>,
    problems: Vec<String>,
    /// Above zero while walking a sibling's own dependencies: their
    /// problems are the sibling's to report, and only what concerns the
    /// whole plan is reported here.
    quiet: usize,
}

impl<'a> Plan<'a> {
    fn new(
        tools: &'a dyn Tools,
        fetcher: &'a dyn Fetcher,
        library: &'a LibraryJson,
        local: &'a BTreeMap<&'a str, &'a LocalComponent>,
    ) -> Self {
        Plan {
            tools,
            fetcher,
            library,
            local,
            visited_local: BTreeSet::new(),
            remote_versions: BTreeMap::new(),
            reached: BTreeMap::new(),
            problems: Vec::new(),
            quiet: 0,
        }
    }

    fn problem(&mut self, problem: String, whole_plan: bool) {
        if whole_plan || self.quiet == 0 {
            self.problems.push(problem);
        }
    }

    fn size(&self) -> usize {
        self.visited_local.len() + self.remote_versions.len()
    }

    fn resolve(&mut self, origin: &Origin, from: &str, spec: &str) -> Option<Reached> {
        match classify(spec) {
            Err(problem) => {
                self.problem(format!("dependency {spec:?} of {from} {problem}"), false);
                None
            }
            Ok(Dependency::Shipped(name)) => {
                if COMPONENTS
                    .iter()
                    .any(|component| component.directory == name)
                {
                    Some(Reached::default())
                } else {
                    self.problem(
                        format!(
                            "dependency {spec:?} of {from} is not a component of the shipped library; a bare name is always a shipped component, and a component of this library is `./{name}`"
                        ),
                        false,
                    );
                    None
                }
            }
            Ok(Dependency::Sibling(name)) => match origin {
                Origin::Local => self.sibling(from, &name),
                Origin::Remote { library, version } => {
                    self.remote(from, spec, library.clone(), name, version.clone())
                }
            },
            Ok(Dependency::Full(text)) => {
                let remote = match self.tools.resolve_address(&text) {
                    Ok(remote) => remote,
                    Err(error) => {
                        self.problem(format!("dependency {spec:?} of {from}: {error}"), false);
                        return None;
                    }
                };
                let version = match remote.version {
                    Some(version) => version,
                    None => match self.fetcher.versions(&remote.library) {
                        Ok(versions) => match versions.into_iter().max() {
                            Some(version) => version,
                            None => {
                                self.problem(
                                    format!(
                                        "dependency {spec:?} of {from}: {} has no release tag",
                                        remote.library
                                    ),
                                    false,
                                );
                                return None;
                            }
                        },
                        Err(error) => {
                            self.problem(
                                format!("dependency {spec:?} of {from} does not fetch: {error}"),
                                false,
                            );
                            return None;
                        }
                    },
                };
                self.remote(from, spec, remote.library, remote.component, version)
            }
        }
    }

    fn sibling(&mut self, from: &str, name: &str) -> Option<Reached> {
        let Some(component) = self.local.get(name).copied() else {
            self.problem(
                format!("dependency \"./{name}\" of {from}: this library has no component {name}"),
                false,
            );
            return None;
        };
        if self.visited_local.insert(name.to_owned()) {
            if self.size() > MAX_PLAN_COMPONENTS {
                self.problem(
                    format!("the plan holds more than {MAX_PLAN_COMPONENTS} components"),
                    true,
                );
                return None;
            }
            self.quiet += 1;
            for dependency in &component.manifest.dependencies {
                self.resolve(&Origin::Local, name, dependency);
            }
            self.quiet -= 1;
        }
        let namespace_module = library::namespace_module(&self.library.namespace);
        let mut reached = Reached::default();
        for (file, _) in &component.files {
            if let Some(stem) = file.strip_suffix(".rs") {
                reached.modules.push(format!("{namespace_module}::{stem}"));
            } else if file.ends_with(".html") {
                reached
                    .views
                    .push(format!("{}-ui/{name}/{file}", self.library.namespace));
            }
        }
        Some(reached)
    }

    fn remote(
        &mut self,
        from: &str,
        spec: &str,
        library: LibraryAddress,
        component: String,
        version: semver::Version,
    ) -> Option<Reached> {
        let address = format!("{library}/{component}");
        if let Some(existing) = self.remote_versions.get(&address) {
            if existing != &version {
                let problem = format!(
                    "{address} is needed at {existing} and at {version} (by {from}); one plan installs one version of a component"
                );
                self.problem(problem, true);
                return None;
            }
            return self.reached.get(&address).cloned();
        }
        self.remote_versions
            .insert(address.clone(), version.clone());
        if self.size() > MAX_PLAN_COMPONENTS {
            self.problem(
                format!("the plan holds more than {MAX_PLAN_COMPONENTS} components"),
                true,
            );
            return None;
        }
        match self.fetch(&library, &component, &version) {
            Ok((reached, dependencies)) => {
                self.reached.insert(address.clone(), reached.clone());
                let origin = Origin::Remote { library, version };
                for dependency in &dependencies {
                    self.resolve(&origin, &address, dependency);
                }
                Some(reached)
            }
            Err(problem) => {
                self.problem(
                    format!(
                        "dependency {spec:?} of {from} ({address}@{version}) does not fetch and verify: {problem}"
                    ),
                    false,
                );
                None
            }
        }
    }

    /// Fetches one remote component at the tag's commit and verifies its
    /// signature over the statement of what arrived, as `live:add` does.
    fn fetch(
        &self,
        library: &LibraryAddress,
        component: &str,
        version: &semver::Version,
    ) -> std::result::Result<(Reached, Vec<String>), String> {
        let text = |error: RegistryError| error.to_string();
        let commit = self.fetcher.resolve(library, version).map_err(text)?;
        let library_bytes = self
            .fetcher
            .file(library, &commit, LIBRARY_FILE)
            .map_err(text)?;
        let json = self
            .tools
            .parse_library_json(&library_bytes)
            .map_err(text)?;
        if json.source != library.0 {
            return Err(format!(
                "its library.json names the source {}, not {library}",
                json.source
            ));
        }
        if json.version != *version {
            return Err(format!(
                "its library.json says version {}, not the tag's {version}",
                json.version
            ));
        }
        let base = format!("{COMPONENTS_DIR}/{component}");
        let manifest_bytes = self
            .fetcher
            .file(library, &commit, &format!("{base}/{MANIFEST_FILE}"))
            .map_err(text)?;
        let manifest = self
            .tools
            .parse_manifest(&manifest_bytes, component, &json.namespace)
            .map_err(text)?;
        let mut files = BTreeMap::new();
        for name in &manifest.files {
            if !plain_file_name(name) {
                return Err(format!(
                    "{name:?} is not a file name in the component's directory"
                ));
            }
            let bytes = self
                .fetcher
                .file(library, &commit, &format!("{base}/{name}"))
                .map_err(text)?;
            files.insert(name.clone(), Digest::of(&bytes));
        }
        let signature_bytes = self
            .fetcher
            .file(library, &commit, &format!("{base}/{SIGNATURE_FILE}"))
            .map_err(text)?;
        let signature = std::str::from_utf8(&signature_bytes)
            .map_err(|_| format!("{SIGNATURE_FILE} is not base64 text"))
            .and_then(|signature| Signature::parse(signature).map_err(text))?;
        let statement = Statement {
            library: json.source.clone(),
            version: json.version.clone(),
            component: component.to_owned(),
            library_json: Digest::of(&library_bytes),
            manifest: Digest::of(&manifest_bytes),
            files,
        };
        self.tools
            .verify(&json.public_key, &statement.verification_hash(), &signature)
            .map_err(text)?;
        let namespace_module = library::namespace_module(&json.namespace);
        let mut reached = Reached::default();
        for name in statement.files.keys() {
            if let Some(stem) = name.strip_suffix(".rs") {
                reached.modules.push(format!("{namespace_module}::{stem}"));
            } else if name.ends_with(".html") {
                reached
                    .views
                    .push(format!("{}-ui/{component}/{name}", json.namespace));
            }
        }
        Ok((reached, manifest.dependencies))
    }
}

fn print_report(report: &LibraryReport) {
    for problem in &report.problems {
        ui::error(problem);
    }
    for component in &report.components {
        let line = format!(
            "{}: capabilities: {}",
            component.directory,
            capability_list(&component.capabilities)
        );
        if component.problems.is_empty() {
            ui::success(&line);
        } else {
            ui::info(&line);
            for problem in &component.problems {
                ui::error(&format!("{}: {problem}", component.directory));
            }
        }
    }
}

/// Capabilities as the plan shows them: lowercase names, or `none`.
fn capability_list(capabilities: &BTreeSet<Capability>) -> String {
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

fn report_lines(report: &LibraryReport) -> String {
    let mut lines: Vec<String> = report
        .problems
        .iter()
        .map(|problem| format!("  {problem}"))
        .collect();
    for component in &report.components {
        for problem in &component.problems {
            lines.push(format!("  {}: {problem}", component.directory));
        }
    }
    lines.join("\n")
}

fn problem_count(report: &LibraryReport) -> usize {
    report.problems.len()
        + report
            .components
            .iter()
            .map(|component| component.problems.len())
            .sum::<usize>()
}

/// The line `check` ends with when every component passes.
fn passed_summary(count: usize) -> String {
    let verb = if count == 1 { "passes" } else { "pass" };
    format!(
        "{} {verb} every check live:add makes for the library",
        plural(count, "component")
    )
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    use super::{
        Created, LibraryReport, Registry, RemoteSpec, Tools, create_library, inspect,
        rotate_with_key_file, sign_with_key_file,
    };
    use crate::registry::address::LibraryAddress;
    use crate::registry::author_key;
    use crate::registry::fetch::FakeFetcher;
    use crate::registry::library::{ComponentManifest, LibraryJson};
    use crate::registry::scan::{ComponentFiles, Finding, ScanReport};
    use crate::registry::signing::{
        Fingerprint, KeyHandover, PublicKey, SecretKey, Signature, handover_statement,
    };
    use crate::registry::statement::{Digest, Statement};
    use crate::registry::{Capability, RegistryError, Result};

    /// Stands in for lane A's parsing and signing and lane B's scan, so the
    /// authoring steps are tested on their own. A fake key's public bytes
    /// are its secret bytes; a fake signature is the hash's bytes then the
    /// key's, so a stale hash or another key fails to verify. The fake scan
    /// reports `files` for a file naming `Storage::`, refuses a file holding
    /// `REFUSE_ME`, and reads each `pub struct` after `#[live(` as a defined
    /// component.
    struct Fake {
        seed: u8,
    }

    const FAKE: Fake = Fake { seed: 7 };

    fn hash_bytes(hash: &Digest) -> [u8; 32] {
        let hex = hash.as_str().trim_start_matches("sha256:");
        let mut out = [0u8; 32];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex");
        }
        out
    }

    fn json_strings(value: Option<&serde_json::Value>) -> Vec<String> {
        value
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    impl Tools for Fake {
        fn parse_library_json(&self, bytes: &[u8]) -> Result<LibraryJson> {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| RegistryError::Invalid(error.to_string()))?;
            let text = |key: &str| {
                value[key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| RegistryError::Invalid(format!("no {key}")))
            };
            Ok(LibraryJson {
                namespace: text("namespace")?,
                source: text("source")?,
                version: semver::Version::parse(&text("version")?)
                    .map_err(|error| RegistryError::Invalid(error.to_string()))?,
                framework: semver::VersionReq::parse(&text("framework")?)
                    .map_err(|error| RegistryError::Invalid(error.to_string()))?,
                public_key: PublicKey::parse(&text("publicKey")?)?,
                previous_keys: Vec::new(),
                title: value["title"].as_str().map(str::to_owned),
                description: value["description"].as_str().map(str::to_owned),
            })
        }

        fn parse_manifest(
            &self,
            bytes: &[u8],
            directory: &str,
            namespace: &str,
        ) -> Result<ComponentManifest> {
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|error| RegistryError::Invalid(error.to_string()))?;
            let name = value["name"].as_str().unwrap_or_default().to_owned();
            if name != format!("{namespace}.{directory}") {
                return Err(RegistryError::Invalid(format!("name `{name}`")));
            }
            Ok(ComponentManifest {
                name,
                root: value["root"].as_str().map(str::to_owned),
                title: value["title"].as_str().map(str::to_owned),
                description: value["description"].as_str().map(str::to_owned),
                files: json_strings(value.get("files")),
                elements: json_strings(value.get("elements")),
                register: json_strings(value.get("register")),
                dependencies: json_strings(value.get("dependencies")),
            })
        }

        fn verify(&self, key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()> {
            let mut expected = [0u8; 64];
            expected[..32].copy_from_slice(&hash_bytes(hash));
            expected[32..].copy_from_slice(key.bytes());
            if signature.bytes() == &expected {
                Ok(())
            } else {
                Err(RegistryError::Invalid(
                    "the signature does not verify".to_owned(),
                ))
            }
        }

        fn sign(&self, key: &SecretKey, hash: &Digest) -> Result<Signature> {
            let mut bytes = [0u8; 64];
            bytes[..32].copy_from_slice(&hash_bytes(hash));
            bytes[32..].copy_from_slice(key.bytes());
            Signature::parse(&STANDARD.encode(bytes))
        }

        fn generate(&self) -> Result<(SecretKey, PublicKey)> {
            let bytes = [self.seed; 32];
            Ok((SecretKey::from_bytes(bytes), PublicKey::from_bytes(bytes)))
        }

        fn sign_handover(
            &self,
            former: &SecretKey,
            new_key: &PublicKey,
            library: &str,
        ) -> Result<KeyHandover> {
            let to = new_key.fingerprint();
            let mut bytes = [0u8; 64];
            bytes[..32].copy_from_slice(&hash_bytes(&Digest::of(
                handover_statement(library, &to).as_bytes(),
            )));
            bytes[32..].copy_from_slice(former.bytes());
            Ok(KeyHandover {
                from: PublicKey::from_bytes(*former.bytes()),
                to,
                signature: Signature::parse(&STANDARD.encode(bytes))?,
            })
        }

        fn verify_handover(
            &self,
            handover: &KeyHandover,
            new_key: &PublicKey,
            library: &str,
        ) -> Result<()> {
            let mut expected = [0u8; 64];
            expected[..32].copy_from_slice(&hash_bytes(&Digest::of(
                handover_statement(library, &new_key.fingerprint()).as_bytes(),
            )));
            expected[32..].copy_from_slice(handover.from.bytes());
            if handover.to == new_key.fingerprint() && handover.signature.bytes() == &expected {
                Ok(())
            } else {
                Err(RegistryError::Invalid(
                    "the handover does not verify".to_owned(),
                ))
            }
        }

        fn scan(
            &self,
            component: &ComponentFiles<'_>,
            manifest: &ComponentManifest,
        ) -> Result<ScanReport> {
            let mut report = ScanReport::default();
            for (name, bytes) in component.files {
                let text = String::from_utf8_lossy(bytes);
                if text.contains("Storage::") {
                    report.capabilities.insert(Capability::Files);
                }
                if let Some(line) = text.lines().position(|line| line.contains("REFUSE_ME")) {
                    report.findings.push(Finding {
                        check: "fake",
                        file: name.clone(),
                        line: u32::try_from(line + 1).ok(),
                        message: "refused".to_owned(),
                    });
                }
                if let Some(stem) = name.strip_suffix(".rs") {
                    let mut after_live = false;
                    for line in text.lines() {
                        if line.trim_start().starts_with("#[live(") {
                            after_live = true;
                        } else if after_live
                            && let Some(rest) = line.trim_start().strip_prefix("pub struct ")
                        {
                            let ty: String = rest
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            report.defined_components.push(format!("{stem}::{ty}"));
                            after_live = false;
                        }
                    }
                }
            }
            for declared in &manifest.register {
                if !report.defined_components.contains(declared) {
                    report.findings.push(Finding {
                        check: "fake-register",
                        file: "manifest.json".to_owned(),
                        line: None,
                        message: format!(
                            "register names {declared}, which the Rust does not define"
                        ),
                    });
                }
            }
            for defined in &report.defined_components {
                if !manifest.register.contains(defined) {
                    report.findings.push(Finding {
                        check: "fake-register",
                        file: "manifest.json".to_owned(),
                        line: None,
                        message: format!(
                            "the Rust defines {defined}, which register does not name"
                        ),
                    });
                }
            }
            Ok(report)
        }

        fn resolve_address(&self, spec: &str) -> Result<RemoteSpec> {
            let (path, version) = match spec.split_once('@') {
                Some((path, version)) => (
                    path,
                    Some(
                        semver::Version::parse(version)
                            .map_err(|error| RegistryError::Invalid(error.to_string()))?,
                    ),
                ),
                None => (spec, None),
            };
            let segments: Vec<&str> = path.split('/').collect();
            let [host, owner, library, component] = segments.as_slice() else {
                return Err(RegistryError::Invalid(format!(
                    "`{spec}` is not an address"
                )));
            };
            Ok(RemoteSpec {
                library: LibraryAddress(format!("{host}/{owner}/{library}").to_lowercase()),
                component: (*component).to_owned(),
                version,
            })
        }
    }

    const FORBIDDEN_MARKERS: [&str; 4] = ["TODO", "FIXME", "unimplemented!", "panic!("];

    struct Library {
        _dir: tempfile::TempDir,
        root: PathBuf,
        config: PathBuf,
        created: Created,
    }

    fn library() -> Library {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("acme");
        let config = dir.path().join("config");
        let created = create_library("acme", &root, &config, &FAKE, &FakeFetcher::default())
            .expect("live:registry new");
        Library {
            _dir: dir,
            root,
            config,
            created,
        }
    }

    fn check(root: &Path) -> LibraryReport {
        inspect(root, true, &FAKE, &FakeFetcher::default())
            .expect("inspects")
            .report
    }

    fn sign(library: &Library) -> Result<usize> {
        sign_with_key_file(
            &library.root,
            None,
            &library.config,
            &FAKE,
            &FakeFetcher::default(),
        )
        .map(|(_, signed)| signed)
    }

    fn problems(report: &LibraryReport) -> String {
        let mut lines = report.problems.clone();
        for component in &report.components {
            for problem in &component.problems {
                lines.push(format!("{}: {problem}", component.directory));
            }
        }
        lines.join("\n")
    }

    fn component(library: &Library) -> PathBuf {
        library.root.join("components/counter")
    }

    fn edit(path: &Path, from: &str, to: &str) {
        let text = std::fs::read_to_string(path).expect("read");
        assert!(text.contains(from), "{} lacks {from:?}", path.display());
        std::fs::write(path, text.replacen(from, to, 1)).expect("write");
    }

    fn add_component(library: &Library, name: &str, manifest_extra: &str, files: &[(&str, &str)]) {
        let dir = library.root.join("components").join(name);
        std::fs::create_dir_all(&dir).expect("dir");
        let names: Vec<String> = files
            .iter()
            .map(|(file, _)| format!("\"{file}\""))
            .collect();
        std::fs::write(
            dir.join("manifest.json"),
            format!(
                "{{\"name\": \"acme.{name}\", \"files\": [{}]{manifest_extra}}}",
                names.join(", ")
            ),
        )
        .expect("manifest");
        for (file, content) in files {
            std::fs::write(dir.join(file), content).expect("file");
        }
    }

    #[test]
    fn reg_018_new_scaffolds_the_tree_signs_it_and_keeps_the_key_outside_the_project() {
        let library = library();
        let root = &library.root;
        let library_json = std::fs::read_to_string(root.join("library.json")).expect("library");
        let parsed: serde_json::Value = serde_json::from_str(&library_json).expect("json");
        assert_eq!(parsed["namespace"], "acme");
        assert_eq!(parsed["version"], "0.1.0");
        assert_eq!(parsed["source"], library.created.source.as_str());
        assert_eq!(
            parsed["framework"],
            format!("^{}", env!("CARGO_PKG_VERSION")).as_str()
        );
        let public = PublicKey::parse(parsed["publicKey"].as_str().expect("key")).expect("key");
        assert_eq!(public.fingerprint(), library.created.fingerprint);
        let keys: BTreeSet<&str> = parsed
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "namespace",
                "source",
                "version",
                "framework",
                "publicKey",
                "title",
                "description"
            ])
        );

        for file in [
            "manifest.json",
            "manifest.sig",
            "counter.html",
            "counter.css",
            "counter.js",
            "counter.rs",
        ] {
            assert!(component(&library).join(file).is_file(), "{file}");
        }
        assert_eq!(library.created.signed, 1);

        // The private key is in the configuration directory, named by its
        // fingerprint, and nowhere inside the project.
        assert!(library.created.key_path.starts_with(&library.config));
        assert!(library.created.key_path.is_file());
        let (_, key_public) =
            author_key::read_key_file(&library.created.key_path).expect("key file");
        assert_eq!(key_public, public);
        let secret_text = STANDARD.encode([7u8; 32]);
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.expect("walk");
            if entry.file_type().is_file() {
                let bytes = std::fs::read(entry.path()).expect("read");
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(&format!("secret {secret_text}")),
                    "{} holds the private key",
                    entry.path().display()
                );
            }
        }

        // The preview compiles each component from where it sits.
        let preview = root.join("preview");
        let live = std::fs::read_to_string(preview.join("src/live/mod.rs")).expect("live");
        assert!(
            live.contains(".register::<crate::live::acme::counter::Counter>()?"),
            "{live}"
        );
        assert_eq!(live.matches("pub mod acme;").count(), 1, "{live}");
        let namespace =
            std::fs::read_to_string(preview.join("src/live/acme/mod.rs")).expect("namespace");
        assert!(
            namespace.contains(
                "#[path = \"../../../../components/counter/counter.rs\"]\npub mod counter;"
            ),
            "{namespace}"
        );
        assert!(
            preview
                .join("src/live/acme")
                .join("../../../../components/counter/counter.rs")
                .is_file()
        );
        let askama = std::fs::read_to_string(preview.join("askama.toml")).expect("askama.toml");
        assert!(
            askama.contains("dirs = [\"templates\", \"../components\"]"),
            "{askama}"
        );
        let stub = std::fs::read_to_string(preview.join("templates/acme-ui/counter/counter.html"))
            .expect("the view stub");
        assert!(
            stub.contains("{% include \"counter/counter.html\" %}"),
            "{stub}"
        );
        assert!(preview.join("../components/counter/counter.html").is_file());
        assert!(preview.join("templates/_preview/page.html").is_file());
        for entry in walkdir::WalkDir::new(&preview).follow_links(false) {
            let entry = entry.expect("walk");
            assert!(
                !entry.path_is_symlink(),
                "{} is a link; the Live tooling refuses one under a template root",
                entry.path().display()
            );
        }
        assert!(preview.join("tests/counter.rs").is_file());
        let lib = std::fs::read_to_string(preview.join("src/lib.rs")).expect("lib");
        assert_eq!(lib.matches("pub mod preview;").count(), 1, "{lib}");
        let pages = std::fs::read_to_string(preview.join("src/preview.rs")).expect("pages");
        assert!(
            pages.contains(
                "try_live_ui_assets_for_from(\"acme\", suprnova::base_path(\"../components\"))"
            ),
            "{pages}"
        );
        let cargo = std::fs::read_to_string(preview.join("Cargo.toml")).expect("cargo");
        assert!(cargo.contains("name = \"acme_preview\""), "{cargo}");
        for path in [
            preview.join("src/live/mod.rs"),
            preview.join("src/live/acme/mod.rs"),
            preview.join("src/preview.rs"),
            preview.join("tests/counter.rs"),
            component(&library).join("counter.rs"),
        ] {
            let source = std::fs::read_to_string(&path).expect("read");
            syn::parse_file(&source)
                .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
        }

        // No stub markers anywhere in what was generated.
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.expect("walk");
            if !entry.file_type().is_file() {
                continue;
            }
            let text =
                String::from_utf8_lossy(&std::fs::read(entry.path()).expect("read")).into_owned();
            for marker in FORBIDDEN_MARKERS {
                assert!(
                    !text.contains(marker),
                    "{} holds {marker}",
                    entry.path().display()
                );
            }
        }

        // As generated, the tree passes its own check.
        let report = check(root);
        assert!(report.passed(), "{}", problems(&report));
        assert_eq!(report.components.len(), 1);
        assert!(report.components[0].capabilities.is_empty());
    }

    #[test]
    fn reg_018_new_refuses_a_bad_namespace_or_an_existing_directory_and_leaves_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config");
        for namespace in ["suprnova", "sn", "live", "Acme", "1acme", "self", "a_b"] {
            let target = dir.path().join("lib");
            assert!(
                create_library(namespace, &target, &config, &FAKE, &FakeFetcher::default())
                    .is_err(),
                "{namespace}"
            );
            assert!(!target.exists(), "{namespace} left a directory");
        }
        let existing = dir.path().join("taken");
        std::fs::create_dir(&existing).expect("dir");
        std::fs::write(existing.join("keep"), "mine").expect("file");
        assert!(
            create_library("acme", &existing, &config, &FAKE, &FakeFetcher::default()).is_err()
        );
        assert_eq!(
            std::fs::read_to_string(existing.join("keep")).expect("kept"),
            "mine"
        );
        assert!(!config.exists(), "no key was written");
    }

    /// A tool failure after the tree is written removes the tree, and no key
    /// is kept for a library that does not exist.
    #[test]
    fn reg_018_a_failed_new_leaves_no_tree_and_no_key() {
        struct ScanFails;
        impl Tools for ScanFails {
            fn parse_library_json(&self, bytes: &[u8]) -> Result<LibraryJson> {
                FAKE.parse_library_json(bytes)
            }
            fn parse_manifest(
                &self,
                bytes: &[u8],
                directory: &str,
                namespace: &str,
            ) -> Result<ComponentManifest> {
                FAKE.parse_manifest(bytes, directory, namespace)
            }
            fn verify(&self, key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()> {
                FAKE.verify(key, hash, signature)
            }
            fn sign(&self, key: &SecretKey, hash: &Digest) -> Result<Signature> {
                FAKE.sign(key, hash)
            }
            fn generate(&self) -> Result<(SecretKey, PublicKey)> {
                FAKE.generate()
            }
            fn sign_handover(
                &self,
                former: &SecretKey,
                new_key: &PublicKey,
                library: &str,
            ) -> Result<KeyHandover> {
                FAKE.sign_handover(former, new_key, library)
            }
            fn verify_handover(
                &self,
                handover: &KeyHandover,
                new_key: &PublicKey,
                library: &str,
            ) -> Result<()> {
                FAKE.verify_handover(handover, new_key, library)
            }
            fn scan(
                &self,
                _component: &ComponentFiles<'_>,
                _manifest: &ComponentManifest,
            ) -> Result<ScanReport> {
                Err(RegistryError::Io("the component scan failed".to_owned()))
            }
            fn resolve_address(&self, spec: &str) -> Result<RemoteSpec> {
                FAKE.resolve_address(spec)
            }
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("acme");
        let config = dir.path().join("config");
        let error = create_library("acme", &root, &config, &ScanFails, &FakeFetcher::default())
            .err()
            .expect("refused");
        assert!(error.to_string().contains("scan"), "{error}");
        assert!(!root.exists());
        let keys = config.join("suprnova/library-keys");
        let kept = std::fs::read_dir(&keys).map_or(0, |entries| entries.count());
        assert_eq!(kept, 0, "a key was kept for a library that does not exist");
    }

    #[test]
    fn reg_019_check_fails_a_stale_signature_and_sign_re_signs_the_edit() {
        let library = library();
        edit(
            &component(&library).join("counter.css"),
            "gap: 0.75rem;",
            "gap: 1rem;",
        );
        let report = check(&library.root);
        assert!(!report.passed());
        assert!(
            problems(&report).contains("manifest.sig"),
            "{}",
            problems(&report)
        );
        assert_eq!(sign(&library).expect("signs"), 1);
        let report = check(&library.root);
        assert!(report.passed(), "{}", problems(&report));
    }

    #[test]
    fn reg_019_a_register_entry_the_preview_does_not_register_fails() {
        let library = library();
        edit(
            &library.root.join("preview/src/live/mod.rs"),
            "\n        .register::<crate::live::acme::counter::Counter>()?",
            "",
        );
        let report = check(&library.root);
        assert!(
            problems(&report).contains("crate::live::acme::counter::Counter"),
            "{}",
            problems(&report)
        );
        assert!(sign(&library).is_err(), "sign refuses what check refuses");
    }

    #[test]
    fn reg_019_register_must_match_the_components_its_rust_defines() {
        let library = library();
        edit(
            &component(&library).join("manifest.json"),
            "\"counter::Counter\"",
            "\"counter::Counter\", \"counter::Tally\"",
        );
        let report = check(&library.root);
        assert!(
            problems(&report).contains("counter::Tally"),
            "{}",
            problems(&report)
        );
    }

    #[test]
    fn reg_019_dependencies_resolve_as_live_add_resolves_them() {
        let library = library();
        // `chip` is not a shipped component, so a bare `chip` must not be
        // satisfied by this library's own `chip`.
        add_component(
            &library,
            "chip",
            "",
            &[("chip.html", "<span class=\"acme-chip\"></span>\n")],
        );
        for (dependencies, expected) in [
            ("[\"field\"]", None),
            ("[\"./chip\"]", None),
            ("[\"./missing\"]", Some("./missing")),
            ("[\"chip\"]", Some("chip")),
            (
                "[\"no-such-shipped-component\"]",
                Some("no-such-shipped-component"),
            ),
            ("[\"../escape\"]", Some("../escape")),
        ] {
            let manifest = component(&library).join("manifest.json");
            let original = std::fs::read_to_string(&manifest).expect("read");
            std::fs::write(
                &manifest,
                original.replacen(
                    "\n  \"register\"",
                    &format!("\n  \"dependencies\": {dependencies},\n  \"register\""),
                    1,
                ),
            )
            .expect("write");
            let report = inspect(&library.root, false, &FAKE, &FakeFetcher::default())
                .expect("inspects")
                .report;
            let counter = report
                .components
                .iter()
                .find(|component| component.directory == "counter")
                .expect("counter");
            match expected {
                None => assert!(
                    counter.problems.is_empty(),
                    "{dependencies}: {:?}",
                    counter.problems
                ),
                Some(named) => assert!(
                    counter
                        .problems
                        .iter()
                        .any(|problem| problem.contains(named)),
                    "{dependencies}: {:?}",
                    counter.problems
                ),
            }
            std::fs::write(&manifest, original).expect("restore");
        }
    }

    #[test]
    fn reg_019_a_full_address_dependency_must_fetch_and_verify() {
        let library = library();
        let mut fetcher = FakeFetcher::default();
        let remote = LibraryAddress("github.com/other/other-ui".to_owned());
        let version = semver::Version::new(1, 2, 0);
        let public =
            PublicKey::parse(&format!("ed25519:{}", STANDARD.encode([9u8; 32]))).expect("key");
        let library_json = format!(
            "{{\"namespace\": \"other\", \"source\": \"github.com/other/other-ui\", \"version\": \"1.2.0\", \"framework\": \"^3\", \"publicKey\": \"{}\"}}",
            public.encode()
        );
        let manifest = "{\"name\": \"other.chip\", \"files\": [\"chip.html\"]}";
        let view = "<span class=\"other-chip\"></span>\n";
        let statement = Statement {
            library: "github.com/other/other-ui".to_owned(),
            version: version.clone(),
            component: "chip".to_owned(),
            library_json: Digest::of(library_json.as_bytes()),
            manifest: Digest::of(manifest.as_bytes()),
            files: BTreeMap::from([("chip.html".to_owned(), Digest::of(view.as_bytes()))]),
        };
        let signature = FAKE
            .sign(
                &SecretKey::from_bytes([9u8; 32]),
                &statement.verification_hash(),
            )
            .expect("sign");
        let files = |signature: String| {
            BTreeMap::from([
                ("library.json".to_owned(), library_json.clone().into_bytes()),
                (
                    "components/chip/manifest.json".to_owned(),
                    manifest.as_bytes().to_vec(),
                ),
                (
                    "components/chip/chip.html".to_owned(),
                    view.as_bytes().to_vec(),
                ),
                (
                    "components/chip/manifest.sig".to_owned(),
                    signature.into_bytes(),
                ),
            ])
        };
        fetcher.add_version(remote.clone(), version.clone(), files(signature.encode()));
        let manifest_path = component(&library).join("manifest.json");
        edit(
            &manifest_path,
            "\n  \"register\"",
            "\n  \"dependencies\": [\"github.com/other/other-ui/chip\"],\n  \"register\"",
        );
        let report = inspect(&library.root, false, &FAKE, &fetcher)
            .expect("inspects")
            .report;
        assert!(report.passed(), "{}", problems(&report));

        let mut tampered = FakeFetcher::default();
        let mut bad = signature.bytes().to_owned();
        bad[0] ^= 1;
        tampered.add_version(remote, version, files(STANDARD.encode(bad)));
        let report = inspect(&library.root, false, &FAKE, &tampered)
            .expect("inspects")
            .report;
        assert!(
            problems(&report).contains("github.com/other/other-ui/chip"),
            "{}",
            problems(&report)
        );
    }

    #[test]
    fn reg_019_check_lists_each_component_s_capabilities_and_refuses_a_finding() {
        let library = library();
        edit(
            &component(&library).join("counter.rs"),
            "use suprnova::live::",
            "// Storage::disk is read elsewhere.\nuse suprnova::live::",
        );
        sign(&library).expect("signs");
        let report = check(&library.root);
        assert!(report.passed(), "{}", problems(&report));
        assert_eq!(
            report.components[0].capabilities,
            BTreeSet::from([Capability::Files])
        );
        edit(
            &component(&library).join("counter.js"),
            "// ",
            "// REFUSE_ME ",
        );
        let report = check(&library.root);
        assert!(
            problems(&report).contains("counter.js:1: [fake] refused"),
            "{}",
            problems(&report)
        );
    }

    #[test]
    fn reg_019_two_components_cannot_install_the_same_rust_file() {
        let library = library();
        add_component(
            &library,
            "tally",
            ", \"register\": [\"counter::Tally\"]",
            &[
                ("tally.html", "<div></div>\n"),
                (
                    "counter.rs",
                    "#[live(name = \"acme.tally\", view = \"acme-ui/tally/tally.html\")]\npub struct Tally;\n",
                ),
            ],
        );
        let report = inspect(&library.root, false, &FAKE, &FakeFetcher::default())
            .expect("inspects")
            .report;
        assert!(
            problems(&report).contains("src/live/acme/counter.rs"),
            "{}",
            problems(&report)
        );
    }

    #[test]
    fn reg_020_sign_writes_nothing_when_one_component_is_invalid() {
        let library = library();
        add_component(
            &library,
            "badge",
            "",
            &[("badge.html", "<span class=\"acme-badge\"></span>\n")],
        );
        assert_eq!(sign(&library).expect("signs both"), 2);
        let counter_signature =
            std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        let badge = library.root.join("components/badge");
        let badge_signature = std::fs::read(badge.join("manifest.sig")).expect("sig");
        edit(
            &component(&library).join("counter.css"),
            "gap: 0.75rem;",
            "gap: 1rem;",
        );
        edit(
            &badge.join("badge.html"),
            "<span",
            "<span data-x=\"REFUSE_ME\"",
        );
        assert!(sign(&library).is_err());
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            counter_signature,
            "the valid component was not re-signed while another failed"
        );
        assert_eq!(
            std::fs::read(badge.join("manifest.sig")).expect("sig"),
            badge_signature
        );
    }

    #[test]
    fn reg_020_a_signature_that_cannot_be_written_leaves_every_other_unchanged() {
        let library = library();
        add_component(
            &library,
            "zeta",
            "",
            &[("zeta.html", "<span class=\"acme-zeta\"></span>\n")],
        );
        std::fs::create_dir(library.root.join("components/zeta/manifest.sig")).expect("dir");
        let before = std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        edit(
            &component(&library).join("counter.css"),
            "gap: 0.75rem;",
            "gap: 1rem;",
        );
        assert!(sign(&library).is_err());
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            before
        );
        let leftovers: Vec<_> = std::fs::read_dir(component(&library))
            .expect("dir")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn reg_020_sign_follows_no_symbolic_link() {
        let library = library();
        let outside = library.root.parent().expect("parent").join("outside.css");
        std::fs::write(&outside, ".acme-counter-body { color: red; }\n").expect("outside");
        let css = component(&library).join("counter.css");
        std::fs::remove_file(&css).expect("remove");
        std::os::unix::fs::symlink(&outside, &css).expect("link");
        let before = std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        let error = sign(&library).expect_err("refused");
        assert!(error.to_string().contains("symbolic link"), "{error}");
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            before
        );
    }

    #[test]
    fn reg_020_the_same_tree_and_key_sign_to_the_same_bytes() {
        let library = library();
        let first = std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        assert!(
            !first.ends_with(b"\n"),
            "manifest.sig holds only the base64"
        );
        Signature::parse(std::str::from_utf8(&first).expect("utf8")).expect("a signature");
        sign(&library).expect("signs");
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            first
        );
    }

    #[test]
    fn reg_020_sign_refuses_a_key_that_is_not_the_library_s() {
        let library = library();
        let other = library.root.parent().expect("parent").join("other.key");
        let other_public =
            PublicKey::parse(&format!("ed25519:{}", STANDARD.encode([8u8; 32]))).expect("key");
        author_key::write_key_file(&other, &SecretKey::from_bytes([8u8; 32]), &other_public)
            .expect("key");
        let before = std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        let error = sign_with_key_file(
            &library.root,
            Some(other.into()),
            &library.config,
            &FAKE,
            &FakeFetcher::default(),
        )
        .err()
        .expect("refused");
        assert!(error.to_string().contains("library.json"), "{error}");
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            before
        );
    }

    const PUBLISHED: &str = "github.com/acme-test/acme-ui";

    /// Names where the library is published, as an author does before a
    /// release, and signs it again.
    fn publish_at(library: &Library, source: &str, tools: &dyn Tools) {
        edit(
            &library.root.join("library.json"),
            &format!("\"{}\"", library.created.source),
            &format!("\"{source}\""),
        );
        sign_with_key_file(
            &library.root,
            None,
            &library.config,
            tools,
            &FakeFetcher::default(),
        )
        .expect("signs");
    }

    /// Every file of the library, by path, with its bytes.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry.path().to_path_buf(),
                    std::fs::read(entry.path()).expect("read"),
                )
            })
            .collect()
    }

    fn key_files(config: &Path) -> BTreeSet<PathBuf> {
        std::fs::read_dir(config.join("suprnova/library-keys"))
            .map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn rotate(library: &Library, tools: &dyn Tools) -> Result<super::Rotated> {
        rotate_dropping(library, tools, &[])
    }

    fn rotate_dropping(
        library: &Library,
        tools: &dyn Tools,
        drop: &[Fingerprint],
    ) -> Result<super::Rotated> {
        rotate_with_key_file(
            &library.root,
            None,
            &library.config,
            &drop.iter().cloned().collect(),
            tools,
            &FakeFetcher::default(),
        )
    }

    #[test]
    fn reg_033_rotate_key_hands_the_library_to_a_new_key_and_re_signs_every_component() {
        let library = real_library();
        publish_at(&library, PUBLISHED, &Registry);
        let former = library.created.fingerprint.clone();
        let former_key_file = library.created.key_path.clone();

        let rotated = rotate(&library, &Registry).expect("rotates");
        assert_eq!(rotated.former, former);
        assert_ne!(rotated.new, former);
        assert_eq!(rotated.version, semver::Version::new(0, 1, 1));
        assert_eq!(rotated.signed, 1);

        let bytes = std::fs::read(library.root.join("library.json")).expect("library.json");
        let json = crate::registry::library::parse_library_json(&bytes).expect("parses");
        assert_eq!(json.public_key.fingerprint(), rotated.new);
        assert_eq!(json.version, semver::Version::new(0, 1, 1));
        assert_eq!(json.source, PUBLISHED);
        let [handover] = json.previous_keys.as_slice() else {
            panic!("one previousKeys entry: {:?}", json.previous_keys);
        };
        assert_eq!(handover.from.fingerprint(), former);
        crate::registry::signing::verify_handover(handover, &json.public_key, PUBLISHED)
            .expect("the former key vouches for the new one");
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(
            text.find("\"publicKey\"") < text.find("\"previousKeys\"")
                && text.find("\"previousKeys\"") < text.find("\"title\""),
            "{text}"
        );

        // Every component verifies against the new key.
        let report = inspect(&library.root, true, &Registry, &FakeFetcher::default())
            .expect("inspects")
            .report;
        assert!(report.passed(), "{}", problems(&report));

        // The new private key is in the configuration directory under its
        // fingerprint; the former one is kept; neither is in the library.
        assert_eq!(
            rotated.key_path,
            author_key::key_path_in(&library.config, &json.public_key)
        );
        let (_, public) = author_key::read_key_file(&rotated.key_path).expect("new key");
        assert_eq!(public, json.public_key);
        assert!(former_key_file.is_file(), "the former key file is kept");
        for (path, contents) in snapshot(&library.root) {
            assert!(
                !String::from_utf8_lossy(&contents).contains(author_key::KEY_FILE_FORMAT),
                "{} holds a key file",
                path.display()
            );
        }

        // A handover is one hop (REG-033): after a second rotation every
        // former key, the first included, signs a statement naming the
        // third key, so an application still pinned to the first follows.
        let again = rotate(&library, &Registry).expect("rotates again");
        let json = crate::registry::library::parse_library_json(
            &std::fs::read(library.root.join("library.json")).expect("library.json"),
        )
        .expect("parses");
        assert_eq!(json.public_key.fingerprint(), again.new);
        assert_eq!(json.version, semver::Version::new(0, 1, 2));
        let from: Vec<Fingerprint> = json
            .previous_keys
            .iter()
            .map(|handover| handover.from.fingerprint())
            .collect();
        assert_eq!(from, vec![former.clone(), rotated.new.clone()]);
        for handover in &json.previous_keys {
            assert_eq!(handover.to, again.new);
            crate::registry::signing::verify_handover(handover, &json.public_key, PUBLISHED)
                .expect("each former key vouches for the newest one");
        }
        assert_eq!(again.vouching, from);
        let report = inspect(&library.root, true, &Registry, &FakeFetcher::default())
            .expect("inspects")
            .report;
        assert!(report.passed(), "{}", problems(&report));
    }

    #[test]
    fn reg_033_a_former_key_without_its_file_is_refused_unless_dropped() {
        let library = real_library();
        publish_at(&library, PUBLISHED, &Registry);
        let first = library.created.fingerprint.clone();
        let middle = rotate(&library, &Registry).expect("rotates").new;
        std::fs::remove_file(&library.created.key_path).expect("lose the first key");
        let before = snapshot(&library.root);
        let keys = key_files(&library.config);
        let error = rotate(&library, &Registry)
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains(first.as_str()), "{error}");
        assert!(error.contains("--drop-key"), "{error}");
        assert_eq!(snapshot(&library.root), before);
        assert_eq!(key_files(&library.config), keys);

        let unknown = PublicKey::from_bytes([42; 32]).fingerprint();
        let error = rotate_dropping(&library, &Registry, std::slice::from_ref(&unknown))
            .err()
            .expect("an unknown key cannot be dropped")
            .to_string();
        assert!(error.contains(unknown.as_str()), "{error}");
        assert!(
            rotate_dropping(&library, &Registry, std::slice::from_ref(&middle)).is_err(),
            "the current key is not a former key to drop"
        );
        assert_eq!(snapshot(&library.root), before);

        let rotated =
            rotate_dropping(&library, &Registry, std::slice::from_ref(&first)).expect("rotates");
        assert_eq!(
            rotated
                .dropped
                .iter()
                .map(PublicKey::fingerprint)
                .collect::<Vec<_>>(),
            vec![first]
        );
        let json = crate::registry::library::parse_library_json(
            &std::fs::read(library.root.join("library.json")).expect("library.json"),
        )
        .expect("parses");
        let [handover] = json.previous_keys.as_slice() else {
            panic!("only the middle key vouches: {:?}", json.previous_keys);
        };
        assert_eq!(handover.from.fingerprint(), middle);
        crate::registry::signing::verify_handover(handover, &json.public_key, PUBLISHED)
            .expect("verifies");
    }

    #[test]
    fn reg_033_rotate_key_refuses_without_the_current_key_and_changes_nothing() {
        let library = library();
        publish_at(&library, PUBLISHED, &FAKE);
        std::fs::remove_file(&library.created.key_path).expect("lose the key");
        let before = snapshot(&library.root);
        let error = rotate(&library, &Fake { seed: 11 })
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains("no key file"), "{error}");
        assert_eq!(snapshot(&library.root), before);
        assert!(key_files(&library.config).is_empty());
    }

    #[test]
    fn reg_033_rotate_key_refuses_the_placeholder_source() {
        let library = library();
        let before = snapshot(&library.root);
        let keys = key_files(&library.config);
        let error = rotate(&library, &Fake { seed: 11 })
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains(&library.created.source), "{error}");
        assert!(error.contains("source"), "{error}");
        assert_eq!(snapshot(&library.root), before);
        assert_eq!(key_files(&library.config), keys);
    }

    #[test]
    fn reg_033_rotate_key_refuses_when_a_component_fails_a_check() {
        let library = library();
        publish_at(&library, PUBLISHED, &FAKE);
        edit(
            &component(&library).join("counter.js"),
            "// ",
            "// REFUSE_ME ",
        );
        let before = snapshot(&library.root);
        let keys = key_files(&library.config);
        let error = rotate(&library, &Fake { seed: 11 })
            .err()
            .expect("refused")
            .to_string();
        assert!(error.contains("counter.js:1: [fake] refused"), "{error}");
        assert_eq!(snapshot(&library.root), before);
        assert_eq!(key_files(&library.config), keys);
    }

    #[test]
    fn reg_033_a_rotation_that_cannot_write_leaves_the_former_key_in_charge() {
        let library = library();
        publish_at(&library, PUBLISHED, &FAKE);
        add_component(
            &library,
            "zeta",
            "",
            &[("zeta.html", "<span class=\"acme-zeta\"></span>\n")],
        );
        std::fs::create_dir(library.root.join("components/zeta/manifest.sig")).expect("dir");
        let before = snapshot(&library.root);
        let keys = key_files(&library.config);
        assert!(rotate(&library, &Fake { seed: 11 }).is_err());
        assert_eq!(snapshot(&library.root), before);
        assert_eq!(
            key_files(&library.config),
            keys,
            "the new key is not kept when the library still names the former one"
        );
    }

    #[test]
    fn the_check_summary_agrees_in_number() {
        assert_eq!(
            super::passed_summary(1),
            "1 component passes every check live:add makes for the library"
        );
        assert_eq!(
            super::passed_summary(3),
            "3 components pass every check live:add makes for the library"
        );
    }

    /// A library `new` makes with the real tools: real keys and
    /// signatures, the real scan and parsers.
    fn real_library() -> Library {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("acme");
        let config = dir.path().join("config");
        let created = create_library("acme", &root, &config, &Registry, &FakeFetcher::default())
            .expect("live:registry new with the real tools");
        Library {
            _dir: dir,
            root,
            config,
            created,
        }
    }

    #[test]
    fn reg_018_with_the_real_tools_the_scaffold_passes_its_own_check() {
        let library = real_library();
        let report = inspect(&library.root, true, &Registry, &FakeFetcher::default())
            .expect("inspects")
            .report;
        assert!(report.passed(), "{}", problems(&report));
        assert!(report.components[0].capabilities.is_empty());
    }

    #[test]
    fn reg_032_check_refuses_a_custom_element_the_manifest_does_not_declare() {
        let library = real_library();
        let script = component(&library).join("counter.js");
        let original = std::fs::read_to_string(&script).expect("script");
        std::fs::write(
            &script,
            format!(
                "{original}\nif (!customElements.get(\"acme-extra\")) {{\n  customElements.define(\"acme-extra\", class extends HTMLElement {{}});\n}}\n"
            ),
        )
        .expect("write");
        let report = inspect(&library.root, false, &Registry, &FakeFetcher::default())
            .expect("inspects")
            .report;
        assert!(
            problems(&report).contains("acme-extra"),
            "an undeclared element passed the library's own check:\n{}",
            problems(&report)
        );
        let before = std::fs::read(component(&library).join("manifest.sig")).expect("sig");
        assert!(
            sign_with_key_file(
                &library.root,
                None,
                &library.config,
                &Registry,
                &FakeFetcher::default()
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(component(&library).join("manifest.sig")).expect("sig"),
            before
        );
    }

    #[test]
    fn reg_019_a_library_without_its_tree_fails_naming_what_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report = check(dir.path());
        assert!(
            problems(&report).contains("library.json"),
            "{}",
            problems(&report)
        );
        let library = library();
        std::fs::rename(
            library.root.join("components/counter"),
            library.root.join("components/Counter-"),
        )
        .expect("rename");
        let report = check(&library.root);
        assert!(
            problems(&report).contains("Counter-"),
            "{}",
            problems(&report)
        );
    }
}
