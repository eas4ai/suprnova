//! `registries`: the project file, the library tree, fetching at a tag, the
//! hash, the signature, pins and rotation, the plan and the record
//! (REG-001 to REG-004, REG-006 to REG-015, REG-019, REG-020, REG-023 to
//! REG-029, REG-033). Each test is named for the falsifier it closes.
//!
//! Most tests drive the registry's own API with an in-memory library
//! (`Lib`), a [`FakeFetcher`], a scripted prompter and a registration
//! writer standing in for the `syn` one (REG-005, its own mechanism). The
//! scan is the real one where a test needs a clean component to pass it
//! (`TestScanner`); a test that needs a capability uses `MarkerScanner`,
//! which reads a `// uses: <capability>` line, so the approval flow is
//! tested apart from the scanner that will find capabilities in Rust. The
//! HTTPS client is tested against a loopback server that stands in for a
//! library host and for the three forges.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use suprnova_cli::registry::address::{self, ComponentAddress, LibraryAddress};
use suprnova_cli::registry::fetch::{
    Commit, FakeFetcher, Fetcher, HttpsClient, MAX_RESPONSE_BYTES, SourceFetcher,
};
use suprnova_cli::registry::install::{self, RegistrationEdits};
use suprnova_cli::registry::library::{self, MAX_JSON_BYTES};
use suprnova_cli::registry::plan::{self, FileOutcome, Options, Plan, Prompter, Scanner};
use suprnova_cli::registry::project::{
    self, InstallRecord, Journal, ProjectFile, ProjectLock, verify_installed,
};
use suprnova_cli::registry::scan::allowlist::Allowlist;
use suprnova_cli::registry::scan::{self, ComponentFiles, ScanReport};
use suprnova_cli::registry::signing::{self, PublicKey, SecretKey};
use suprnova_cli::registry::statement::{Digest, Statement};
use suprnova_cli::registry::{Capability, RegistryError};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");
const ADDRESS: &str = "github.com/acme/acme-ui";
const SEED: [u8; 32] = [7; 32];

/// A bare project root: `live:add` needs only `Cargo.toml` to see one. It
/// names the framework at a tag, as the scaffold does, so the framework
/// check (REG-007) has a version to read with no lock.
fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nsuprnova = { git = \"https://github.com/eas4ai/suprnova.git\", tag = \"v3.2.1\" }\n",
    )
    .expect("write Cargo.toml");
    tmp
}

fn live_add(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("live:add")
        .args(args)
        .current_dir(root)
        .env_remove("SUPRNOVA_LIBRARY_KEY")
        .output()
        .expect("run live:add")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn v(text: &str) -> semver::Version {
    semver::Version::parse(text).expect("semver")
}

/// Every file and directory under `root`, for asserting nothing was
/// written.
fn listing(root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    for entry in walk(root) {
        out.insert(entry.strip_prefix(root).expect("inside").to_path_buf());
    }
    out
}

fn walk(directory: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(path.clone());
            if path.is_dir() {
                out.extend(walk(&path));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// An in-memory library, signed the way `live:registry sign` signs one.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Comp {
    manifest: Map<String, Value>,
    files: Vec<(String, Vec<u8>)>,
    signed_as: Option<String>,
    signature: Option<Option<String>>,
}

#[derive(Clone)]
struct Lib {
    library_json: Map<String, Value>,
    seed: [u8; 32],
    components: BTreeMap<String, Comp>,
}

fn public_key(seed: [u8; 32]) -> PublicKey {
    SecretKey::from_bytes(seed).public_key()
}

impl Lib {
    fn new(address: &str, namespace: &str, version: &str) -> Self {
        let mut library_json = Map::new();
        library_json.insert("namespace".into(), json!(namespace));
        library_json.insert("source".into(), json!(address));
        library_json.insert("version".into(), json!(version));
        library_json.insert("framework".into(), json!(">=3.0.0, <4.0.0"));
        library_json.insert("publicKey".into(), json!(public_key(SEED).encode()));
        Lib {
            library_json,
            seed: SEED,
            components: BTreeMap::new(),
        }
    }

    fn acme() -> Self {
        Lib::new(ADDRESS, "acme", "1.0.0")
    }

    fn source(&self) -> String {
        self.library_json["source"]
            .as_str()
            .expect("source")
            .to_owned()
    }

    fn address(&self) -> LibraryAddress {
        LibraryAddress(self.source())
    }

    fn namespace(&self) -> String {
        self.library_json["namespace"]
            .as_str()
            .expect("namespace")
            .to_owned()
    }

    fn version(&self) -> semver::Version {
        v(self.library_json["version"].as_str().expect("version"))
    }

    fn key(&self) -> PublicKey {
        public_key(self.seed)
    }

    fn at(mut self, version: &str) -> Self {
        self.library_json.insert("version".into(), json!(version));
        self
    }

    fn set(mut self, key: &str, value: Value) -> Self {
        self.library_json.insert(key.into(), value);
        self
    }

    fn signed_by(mut self, seed: [u8; 32]) -> Self {
        self.seed = seed;
        self.library_json
            .insert("publicKey".into(), json!(public_key(seed).encode()));
        self
    }

    fn component(mut self, directory: &str, files: &[(&str, &str)]) -> Self {
        let mut manifest = Map::new();
        manifest.insert(
            "name".into(),
            json!(format!("{}.{directory}", self.namespace())),
        );
        manifest.insert(
            "files".into(),
            json!(files.iter().map(|(name, _)| *name).collect::<Vec<_>>()),
        );
        self.components.insert(
            directory.to_owned(),
            Comp {
                manifest,
                files: files
                    .iter()
                    .map(|(name, text)| ((*name).to_owned(), text.as_bytes().to_vec()))
                    .collect(),
                signed_as: None,
                signature: None,
            },
        );
        self
    }

    fn widget(self) -> Self {
        self.component(
            "widget",
            &[
                ("widget.html", "<div class=\"acme-widget\">Widget</div>\n"),
                ("widget.css", ".acme-widget { display: block; }\n"),
            ],
        )
    }

    fn manifest(mut self, directory: &str, key: &str, value: Value) -> Self {
        self.components
            .get_mut(directory)
            .expect("component")
            .manifest
            .insert(key.into(), value);
        self
    }

    fn depends(self, directory: &str, dependencies: &[&str]) -> Self {
        self.manifest(directory, "dependencies", json!(dependencies))
    }

    fn library_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(&Value::Object(self.library_json.clone())).expect("json")
    }

    fn statement(&self, directory: &str) -> Statement {
        let component = &self.components[directory];
        let manifest = serde_json::to_vec_pretty(&Value::Object(component.manifest.clone()))
            .expect("manifest json");
        Statement {
            library: self.source(),
            version: self.version(),
            component: component
                .signed_as
                .clone()
                .unwrap_or_else(|| directory.to_owned()),
            library_json: Digest::of(&self.library_json_bytes()),
            manifest: Digest::of(&manifest),
            files: component
                .files
                .iter()
                .map(|(name, bytes)| (name.clone(), Digest::of(bytes)))
                .collect(),
        }
    }

    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        let mut tree = BTreeMap::new();
        tree.insert("library.json".to_owned(), self.library_json_bytes());
        for (directory, component) in &self.components {
            let manifest = serde_json::to_vec_pretty(&Value::Object(component.manifest.clone()))
                .expect("manifest json");
            tree.insert(format!("components/{directory}/manifest.json"), manifest);
            for (name, bytes) in &component.files {
                tree.insert(format!("components/{directory}/{name}"), bytes.clone());
            }
            let signature = signing::sign(
                &SecretKey::from_bytes(self.seed),
                &self.statement(directory).verification_hash(),
            )
            .expect("sign")
            .encode();
            match &component.signature {
                None => {
                    tree.insert(
                        format!("components/{directory}/manifest.sig"),
                        signature.into_bytes(),
                    );
                }
                Some(Some(other)) => {
                    tree.insert(
                        format!("components/{directory}/manifest.sig"),
                        other.clone().into_bytes(),
                    );
                }
                Some(None) => {}
            }
        }
        tree
    }

    fn write_to(&self, root: &Path) {
        for (path, bytes) in self.tree() {
            let full = root.join(path);
            fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            fs::write(full, bytes).expect("write");
        }
    }
}

fn fetcher(libraries: &[&Lib]) -> FakeFetcher {
    let mut fetcher = FakeFetcher::default();
    for library in libraries {
        fetcher.add_version(library.address(), library.version(), library.tree());
    }
    fetcher
}

fn fetcher_with(trees: &[(&Lib, BTreeMap<String, Vec<u8>>)]) -> FakeFetcher {
    let mut fetcher = FakeFetcher::default();
    for (library, tree) in trees {
        fetcher.add_version(library.address(), library.version(), tree.clone());
    }
    fetcher
}

fn pin(root: &Path, library: &str, key: &PublicKey) {
    let mut project = ProjectFile::load(root).expect("load");
    project
        .set_library(&LibraryAddress(library.to_owned()), None, Some(key))
        .expect("pin");
    project.save().expect("save");
}

fn pinned(library: &Lib) -> tempfile::TempDir {
    let root = project();
    pin(root.path(), &library.source(), &library.key());
    root
}

// ---------------------------------------------------------------------------
// Stand-ins: the scan, the prompter, the registration writer.
// ---------------------------------------------------------------------------

/// The real scan with an empty test allowlist. Until the scanners land
/// (lane B) `scan_component` is a placeholder, and a clean component
/// (views without expressions, a stylesheet, no Rust or scripts) stands
/// for itself; once they land these tests run the real scan.
struct TestScanner;

impl Scanner for TestScanner {
    fn scan(&self, component: &ComponentFiles<'_>) -> Result<ScanReport, RegistryError> {
        match scan::scan_component(component, &Allowlist::default()) {
            Err(RegistryError::NotBuilt(_)) => Ok(ScanReport::default()),
            other => other,
        }
    }
}

/// Reports the capabilities a component's Rust names on `// uses: <name>`
/// lines, the Live components on `// live: <Type>` lines, and refuses a
/// `// refuse` line, so the approval flow is tested apart from the scanner
/// that finds them in syntax.
struct MarkerScanner;

impl Scanner for MarkerScanner {
    fn scan(&self, component: &ComponentFiles<'_>) -> Result<ScanReport, RegistryError> {
        let mut report = ScanReport::default();
        for (name, bytes) in component.files {
            if !name.ends_with(".rs") {
                continue;
            }
            let module = name.trim_end_matches(".rs");
            for (index, line) in String::from_utf8_lossy(bytes).lines().enumerate() {
                if let Some(type_name) = line.trim().strip_prefix("// live: ") {
                    report
                        .defined_components
                        .push(format!("{module}::{type_name}"));
                }
                if let Some(capability) = line.trim().strip_prefix("// uses: ") {
                    report
                        .capabilities
                        .insert(Capability::parse(capability).expect("capability"));
                }
                if line.trim() == "// refuse" {
                    report.findings.push(scan::Finding {
                        check: "rust-path",
                        file: name.clone(),
                        line: Some(index as u32 + 1),
                        message: "refused by the test scanner".to_owned(),
                    });
                }
            }
        }
        Ok(report)
    }
}

struct Script {
    terminal: bool,
    answers: VecDeque<bool>,
    asked: Vec<String>,
}

impl Prompter for Script {
    fn is_terminal(&self) -> bool {
        self.terminal
    }

    fn confirm(&mut self, question: &str) -> Result<bool, RegistryError> {
        self.asked.push(question.to_owned());
        self.answers
            .pop_front()
            .ok_or_else(|| RegistryError::Declined(format!("unexpected question: {question}")))
    }
}

fn no_terminal() -> Script {
    Script {
        terminal: false,
        answers: VecDeque::new(),
        asked: Vec::new(),
    }
}

fn terminal(answers: &[bool]) -> Script {
    Script {
        terminal: true,
        answers: answers.iter().copied().collect(),
        asked: Vec::new(),
    }
}

/// Writes `pub mod` lines and `// register <path>` lines as plain text,
/// standing in for the `syn` writer, which has its own mechanism.
fn registrar(
    root: &Path,
    namespace_module: &str,
    modules: &[String],
    register: &[String],
    unregister: &[String],
) -> Result<RegistrationEdits, RegistryError> {
    let live = PathBuf::from("src/live/mod.rs");
    let mut text = fs::read_to_string(root.join(&live)).unwrap_or_default();
    let declaration = format!("pub mod {namespace_module};\n");
    if !text.contains(&declaration) {
        text.push_str(&declaration);
    }
    for path in register {
        let line = format!("// register {path}\n");
        if !text.contains(&line) {
            text.push_str(&line);
        }
    }
    for path in unregister {
        text = text.replace(&format!("// register {path}\n"), "");
    }
    let namespace = PathBuf::from(format!("src/live/{namespace_module}/mod.rs"));
    let mut inner = fs::read_to_string(root.join(&namespace)).unwrap_or_default();
    for module in modules {
        let line = format!("pub mod {module};\n");
        if !inner.contains(&line) {
            inner.push_str(&line);
        }
    }
    Ok(RegistrationEdits::Write(vec![
        (live, text),
        (namespace, inner),
    ]))
}

fn yes() -> Options {
    Options {
        yes: true,
        ..Options::default()
    }
}

/// What `live:add` does, through the library: lock, load, resolve,
/// confirm, apply.
fn install_with(
    root: &Path,
    source: &str,
    fetcher: &dyn Fetcher,
    options: &Options,
    scanner: &dyn Scanner,
    prompter: &mut Script,
) -> Result<Plan, RegistryError> {
    let lock = ProjectLock::acquire(root)?;
    let mut project = ProjectFile::load(root)?;
    let source = address::parse(source)?;
    let plan = plan::resolve_with(&source, options, fetcher, &project, scanner)?;
    let decisions = plan::confirm(&plan, options, &project, prompter)?;
    install::apply_with(&plan, &mut project, options, &decisions, &registrar)?;
    lock.release()?;
    Ok(plan)
}

fn add(root: &Path, source: &str, fetcher: &dyn Fetcher) -> Result<Plan, RegistryError> {
    install_with(
        root,
        source,
        fetcher,
        &yes(),
        &TestScanner,
        &mut no_terminal(),
    )
}

fn resolve(root: &Path, source: &str, fetcher: &dyn Fetcher) -> Result<Plan, RegistryError> {
    let project = ProjectFile::load(root)?;
    plan::resolve_with(
        &address::parse(source)?,
        &yes(),
        fetcher,
        &project,
        &TestScanner,
    )
}

fn records(root: &Path) -> BTreeMap<ComponentAddress, project::ComponentRecord> {
    ProjectFile::load(root)
        .expect("load")
        .components()
        .expect("records")
}

fn component(address: &str) -> ComponentAddress {
    ComponentAddress::parse(address).expect("address")
}

fn expect_refused(result: Result<Plan, RegistryError>, needle: &str) -> String {
    let error = match result {
        Ok(_) => panic!("installed, expected a refusal naming `{needle}`"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains(needle), "`{needle}` not in: {error}");
    error
}

// ---------------------------------------------------------------------------
// A loopback HTTP server standing in for a library host and the forges.
// ---------------------------------------------------------------------------

enum Reply {
    Body(Vec<u8>),
    Redirect(String),
    Stall,
    Status(u16),
    Unbounded(usize),
}

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

fn serve(route: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let base = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let route: Arc<dyn Fn(&str) -> Reply + Send + Sync> = Arc::new(route);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let route = Arc::clone(&route);
            let recorded = Arc::clone(&recorded);
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let path = text
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                recorded.lock().expect("requests").push(text);
                let head = |status: &str, extra: &str| {
                    format!("HTTP/1.1 {status}\r\n{extra}Connection: close\r\n\r\n")
                };
                let _ = match route(&path) {
                    Reply::Body(body) => stream
                        .write_all(
                            head("200 OK", &format!("Content-Length: {}\r\n", body.len()))
                                .as_bytes(),
                        )
                        .and_then(|()| stream.write_all(&body)),
                    Reply::Redirect(location) => stream.write_all(
                        head(
                            "302 Found",
                            &format!("Location: {location}\r\nContent-Length: 0\r\n"),
                        )
                        .as_bytes(),
                    ),
                    Reply::Stall => {
                        let _ =
                            stream.write_all(head("200 OK", "Content-Length: 64\r\n").as_bytes());
                        std::thread::sleep(Duration::from_secs(20));
                        Ok(())
                    }
                    Reply::Status(code) => stream.write_all(
                        head(&format!("{code} Status"), "Content-Length: 0\r\n").as_bytes(),
                    ),
                    Reply::Unbounded(length) => stream
                        .write_all(head("200 OK", "").as_bytes())
                        .and_then(|()| stream.write_all(&vec![b'x'; length])),
                };
            });
        }
    });
    Server { base, requests }
}

fn client() -> HttpsClient {
    HttpsClient::new(Duration::from_secs(10))
}

// ===========================================================================
// REG-001: the project file
// ===========================================================================

/// REG-001: a project holding `Suprnova.toml` and no `suprnova.toml` makes
/// `live:add` fail with a message that names the rename.
#[test]
fn reg_001_a_project_holding_only_the_legacy_project_file_is_refused_naming_the_rename() {
    let root = project();
    fs::write(root.path().join("Suprnova.toml"), "[serve]\n").expect("write");
    let output = live_add(root.path(), &["button"]);
    let text = combined(&output);
    assert!(
        !output.status.success(),
        "live:add installed over a legacy project file:\n{text}"
    );
    assert!(
        text.contains("rename `Suprnova.toml` to `suprnova.toml`"),
        "the refusal does not name the rename:\n{text}"
    );
    assert!(
        !root.path().join("templates").exists(),
        "a refused install wrote files"
    );
}

/// REG-001: `serve` refuses a project holding only `Suprnova.toml`, naming
/// the rename, before it starts anything.
#[test]
fn reg_001_serve_refuses_a_project_holding_only_the_legacy_project_file() {
    let root = project();
    fs::write(
        root.path().join("Suprnova.toml"),
        "[[serve.process]]\nname = \"queue\"\ncommand = \"true\"\n",
    )
    .expect("write");
    let output = Command::new(BIN)
        .args(["serve", "--backend-only"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .output()
        .expect("run serve");
    let text = combined(&output);
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("rename `Suprnova.toml` to `suprnova.toml`"),
        "{text}"
    );
}

/// REG-001: `serve` reads its extra dev processes from `suprnova.toml`: a
/// broken entry there is reported against that file.
#[test]
fn reg_001_serve_reads_its_dev_processes_from_suprnova_toml() {
    let root = project();
    fs::write(
        root.path().join("suprnova.toml"),
        "# dev processes\n[[serve.process]]\ncommand = \"true\"\n",
    )
    .expect("write");
    let output = Command::new(BIN)
        .args(["serve", "--backend-only"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .output()
        .expect("run serve");
    let text = combined(&output);
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("suprnova.toml: serve.process[0] is missing a non-empty `name`"),
        "{text}"
    );
}

const PROJECT_FILE_WITH_COMMENTS: &str = "# The project's dev processes and Live records.\n\n[serve] # serve's own table\n\n[[serve.process]]\nname = \"queue\"   # aligned by hand\ncommand = \"cargo\"\nargs = [\"run\", \"--bin\", \"console\", \"--\", \"queue:work\"]\n\n# a trailing comment the CLI does not own\n";

/// REG-001: `live:add` creates `suprnova.toml` when it is absent, and edits
/// it in place keeping every comment, key order and table it does not own.
#[test]
fn reg_001_an_install_keeps_every_comment_and_serve_entry_byte_for_byte() {
    let root = project();
    let first = live_add(root.path(), &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let created = fs::read_to_string(root.path().join("suprnova.toml")).expect("created");
    assert!(
        created.contains("[live.components.\"suprnova/field\"]"),
        "{created}"
    );

    let root = project();
    fs::write(
        root.path().join("suprnova.toml"),
        PROJECT_FILE_WITH_COMMENTS,
    )
    .expect("write");
    for name in ["field", "button", "field"] {
        let output = live_add(root.path(), &[name]);
        assert!(output.status.success(), "{}", combined(&output));
        let text = fs::read_to_string(root.path().join("suprnova.toml")).expect("read");
        assert!(
            text.starts_with(PROJECT_FILE_WITH_COMMENTS),
            "the install changed bytes it does not own:\n{text}"
        );
    }

    // A third-party record is edited the same way.
    let library = Lib::acme().widget();
    let root = pinned(&library);
    let pinned_text = fs::read_to_string(root.path().join("suprnova.toml")).expect("pinned");
    let with_comments = format!("{PROJECT_FILE_WITH_COMMENTS}{pinned_text}");
    fs::write(root.path().join("suprnova.toml"), &with_comments).expect("write");
    add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("installs");
    let text = fs::read_to_string(root.path().join("suprnova.toml")).expect("read");
    assert!(text.starts_with(PROJECT_FILE_WITH_COMMENTS), "{text}");
}

/// REG-001: no English manual chapter presents `Suprnova.toml` as the file
/// to write.
#[test]
fn reg_001_the_manual_names_suprnova_toml_as_the_project_file() {
    let manual = Path::new(env!("CARGO_MANIFEST_DIR")).join("../manual");
    for entry in fs::read_dir(&manual).expect("manual") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("chapter");
        for line in text.lines() {
            assert!(
                !line.contains("Suprnova.toml") || line.contains("rename"),
                "{} presents Suprnova.toml as the project file: {line}",
                path.display()
            );
        }
    }
}

// ===========================================================================
// REG-002, REG-003, REG-004, REG-025: the tree, the manifests, the names
// ===========================================================================

fn manifest_bytes(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).expect("json")
}

/// REG-002: a manifest with a key outside the set, `cargoDependencies` and
/// a `version` included, is refused.
#[test]
fn reg_002_a_manifest_with_a_key_outside_the_set_is_refused() {
    for (key, value) in [
        ("cargoDependencies", json!({"serde": "1"})),
        ("version", json!(1)),
        ("scripts", json!({"postinstall": "rm -rf /"})),
    ] {
        let manifest = manifest_bytes(json!({
            "name": "acme.widget",
            "files": ["widget.html"],
            key: value,
        }));
        let error = library::parse_manifest(&manifest, "widget", "acme").expect_err(key);
        assert!(error.to_string().contains(key), "{error}");
    }
    let library =
        Lib::acme()
            .widget()
            .manifest("widget", "cargoDependencies", json!({"serde": "1"}));
    let root = pinned(&library);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "cargoDependencies",
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-002: a manifest of more than 1 MiB is refused.
#[test]
fn reg_002_a_manifest_over_one_mib_is_refused() {
    let big = manifest_bytes(json!({
        "name": "acme.widget",
        "files": ["widget.html"],
        "description": "x".repeat(MAX_JSON_BYTES),
    }));
    let error = library::parse_manifest(&big, "widget", "acme").expect_err("too big");
    assert!(error.to_string().contains("limit"), "{error}");
}

/// REG-002: a `name` or `root` that does not match its directory is
/// refused.
#[test]
fn reg_002_a_name_or_root_that_does_not_match_its_directory_is_refused() {
    for manifest in [
        json!({"name": "acme.gadget", "files": ["widget.html"]}),
        json!({"name": "other.widget", "files": ["widget.html"]}),
        json!({"name": "acme.widget", "root": "acme-ui/gadget", "files": ["widget.html"]}),
        json!({"name": "acme.widget", "root": "suprnova-ui/widget", "files": ["widget.html"]}),
    ] {
        assert!(
            library::parse_manifest(&manifest_bytes(manifest.clone()), "widget", "acme").is_err(),
            "{manifest}"
        );
    }
    assert!(
        library::parse_manifest(
            &manifest_bytes(
                json!({"name": "acme.widget", "root": "acme-ui/widget", "files": ["widget.html"]})
            ),
            "widget",
            "acme"
        )
        .is_ok()
    );
}

/// REG-002: every shipped manifest holds only keys a third-party manifest
/// may. The shipped manifests' integer `version`, which the shipped tree
/// drops in its own change, is the one key the shipped reader sets aside.
#[test]
fn reg_002_every_shipped_manifest_holds_only_keys_a_third_party_manifest_may() {
    let components =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/suprnova-live/components");
    let mut count = 0;
    for entry in fs::read_dir(&components).expect("components") {
        let directory = entry.expect("entry").path();
        let manifest = directory.join("manifest.json");
        if !manifest.is_file() {
            continue;
        }
        let name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        library::parse_shipped_manifest(&fs::read(&manifest).expect("read"), name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        count += 1;
    }
    assert!(count > 50, "only {count} shipped manifests");
}

/// REG-003: every file lands by its kind: views, stylesheets and scripts
/// under `templates/<namespace>-ui/<directory>/`, Rust under
/// `src/live/<namespace_module>/`.
#[test]
fn reg_003_each_file_lands_in_its_kinds_directory() {
    let library = Lib::new("github.com/acme/kit", "acme-kit", "1.0.0")
        .component(
            "date-picker",
            &[
                ("date-picker.html", "<div>Date</div>\n"),
                ("date-picker.css", ".d {}\n"),
                ("date-picker.js", "export const ready = true;\n"),
                (
                    "date_picker.rs",
                    "// live: DatePicker\npub struct DatePicker;\n",
                ),
            ],
        )
        .manifest(
            "date-picker",
            "register",
            json!(["date_picker::DatePicker"]),
        );
    let root = pinned(&library);
    let project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/kit/date-picker").expect("source"),
        &yes(),
        &fetcher(&[&library]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    let destinations: BTreeMap<String, PathBuf> = plan.components[0]
        .files
        .iter()
        .map(|file| (file.name.clone(), file.destination.clone()))
        .collect();
    assert_eq!(
        destinations["date-picker.html"],
        PathBuf::from("templates/acme-kit-ui/date-picker/date-picker.html")
    );
    assert_eq!(
        destinations["date-picker.css"],
        PathBuf::from("templates/acme-kit-ui/date-picker/date-picker.css")
    );
    assert_eq!(
        destinations["date-picker.js"],
        PathBuf::from("templates/acme-kit-ui/date-picker/date-picker.js")
    );
    assert_eq!(
        destinations["date_picker.rs"],
        PathBuf::from("src/live/acme_kit/date_picker.rs")
    );
    assert_eq!(
        plan.components[0].registrations,
        vec!["crate::live::acme_kit::date_picker::DatePicker".to_owned()]
    );
}

/// REG-003: a file outside its kind's closed set, of another extension,
/// listed twice, `mod.rs`, or holding `..`, a leading dot, an uppercase
/// letter or a separator, is refused; so is a component with no view.
#[test]
fn reg_003_a_file_name_outside_the_closed_sets_is_refused() {
    let hostile: &[&[&str]] = &[
        &["widget.html", "widget.wasm"],
        &["widget.html", "widget.html"],
        &["widget.html", "mod.rs"],
        &["widget.html", "lib.rs"],
        &["widget.html", "Widget.rs"],
        &["../widget.html"],
        &[".widget.html"],
        &["Widget.html"],
        &["widget.html", "a/b.css"],
        &["widget.html", "Bad.css"],
        &["widget.html", "widget-.js"],
        &["widget.html", "w..css"],
        &["widget.css"],
    ];
    for files in hostile {
        let manifest = manifest_bytes(json!({"name": "acme.widget", "files": files}));
        assert!(
            library::parse_manifest(&manifest, "widget", "acme").is_err(),
            "{files:?} was admitted"
        );
    }
}

/// REG-003: a named file of more than 1 MiB is refused.
#[test]
fn reg_003_a_file_over_one_mib_is_refused() {
    let big = "x".repeat(library::MAX_FILE_BYTES + 1);
    let library = Lib::acme().component("widget", &[("widget.html", &big)]);
    let root = pinned(&library);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "limit",
    );
}

/// REG-003: a second component of one library may not install the Rust
/// file the first one installed, in one plan or across two.
#[test]
fn reg_003_a_second_component_may_not_overwrite_a_rust_file_the_first_installed() {
    let library = Lib::acme()
        .component(
            "one",
            &[
                ("one.html", "<p>One</p>\n"),
                ("state.rs", "pub struct One;\n"),
            ],
        )
        .component(
            "two",
            &[
                ("two.html", "<p>Two</p>\n"),
                ("state.rs", "pub struct Two;\n"),
            ],
        );
    let fake = fetcher(&[&library]);
    let root = pinned(&library);
    install_with(
        root.path(),
        "acme/acme-ui/one",
        &fake,
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("the first installs");
    let error = expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/two",
            &fake,
            &yes(),
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "state.rs",
    );
    assert!(error.contains("github.com/acme/acme-ui/one"), "{error}");
    assert_eq!(
        fs::read_to_string(root.path().join("src/live/acme/state.rs")).expect("state"),
        "pub struct One;\n"
    );

    let both = Lib::acme()
        .component(
            "one",
            &[
                ("one.html", "<p>One</p>\n"),
                ("state.rs", "pub struct One;\n"),
            ],
        )
        .component(
            "two",
            &[
                ("two.html", "<p>Two</p>\n"),
                ("state.rs", "pub struct Two;\n"),
            ],
        )
        .depends("two", &["./one"]);
    let root = pinned(&both);
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/two",
            &fetcher(&[&both]),
            &yes(),
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "state.rs",
    );
    assert!(!root.path().join("src").exists());
}

/// REG-004: the namespaces `suprnova`, `sn` and `live` are refused from
/// any library but the shipped one, as is a namespace whose module form is
/// a keyword, so no third-party component installs under
/// `templates/suprnova-ui/` or `src/live/suprnova/`.
#[test]
fn reg_004_a_reserved_or_keyword_namespace_is_refused() {
    for namespace in ["suprnova", "sn", "live", "self", "crate", "Acme", "1acme"] {
        let library = Lib::new(ADDRESS, namespace, "1.0.0").widget();
        let root = pinned(&library);
        let result = add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library]));
        assert!(result.is_err(), "the namespace `{namespace}` was admitted");
        assert!(!root.path().join("templates").exists());
        assert!(!root.path().join("src").exists());
    }
}

/// REG-004: every element a third-party manifest declares starts with
/// `<namespace>-`; the shipped library's tags keep `sn-`.
#[test]
fn reg_004_an_element_without_the_namespace_prefix_is_refused_and_shipped_tags_keep_sn() {
    for element in ["sn-widget", "widget", "acmewidget", "acme-", "Acme-widget"] {
        let manifest = manifest_bytes(json!({
            "name": "acme.widget",
            "files": ["widget.html"],
            "elements": [element],
        }));
        assert!(
            library::parse_manifest(&manifest, "widget", "acme").is_err(),
            "{element} was admitted"
        );
    }
    let manifest = manifest_bytes(json!({
        "name": "acme.widget",
        "files": ["widget.html"],
        "elements": ["acme-widget"],
    }));
    library::parse_manifest(&manifest, "widget", "acme").expect("prefixed element");

    for component in suprnova_cli::registry::fetch::COMPONENTS {
        let manifest =
            library::parse_shipped_manifest(component.manifest.as_bytes(), component.directory)
                .expect(component.directory);
        for element in manifest.elements {
            assert!(element.starts_with("sn-"), "{element}");
        }
    }
}

/// REG-025: `live:add` refuses a tree with no `library.json`.
#[test]
fn reg_025_a_tree_without_library_json_is_refused() {
    let library = Lib::acme().widget();
    let mut tree = library.tree();
    tree.remove("library.json");
    let root = pinned(&library);
    expect_refused(
        add(
            root.path(),
            "acme/acme-ui/widget",
            &fetcher_with(&[(&library, tree)]),
        ),
        "library.json",
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-025: a `library.json` key outside the set is refused.
#[test]
fn reg_025_a_library_json_key_outside_the_set_is_refused() {
    let library = Lib::acme().widget().set("postInstall", json!("curl evil"));
    let root = pinned(&library);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "postInstall",
    );
}

/// REG-025: a component directory of 65 bytes, or one ending in a hyphen,
/// is refused.
#[test]
fn reg_025_a_component_directory_outside_the_closed_set_is_refused() {
    for directory in [
        "a".repeat(65),
        "widget-".to_owned(),
        "-widget".to_owned(),
        "Widget".to_owned(),
    ] {
        assert!(
            address::parse(&format!("acme/acme-ui/{directory}")).is_err(),
            "{directory}"
        );
    }
    assert!(address::parse(&format!("acme/acme-ui/{}", "a".repeat(64))).is_ok());
}

/// REG-025: a path source to a component that does not sit under
/// `components/` beside `library.json` is refused.
#[test]
fn reg_025_a_path_to_a_component_outside_components_is_refused() {
    let tree = tempfile::tempdir().expect("tree");
    let library = Lib::acme().widget();
    library.write_to(tree.path());
    fs::create_dir_all(tree.path().join("widgets/widget")).expect("mkdir");
    fs::copy(
        tree.path().join("components/widget/widget.html"),
        tree.path().join("widgets/widget/widget.html"),
    )
    .expect("copy");
    let source = tree.path().join("widgets/widget");
    let error = address::parse(source.to_str().expect("utf-8")).expect_err("refused");
    assert!(error.to_string().contains("components/"), "{error}");
}

// ===========================================================================
// REG-006, REG-012: the plan and the developer's decisions
// ===========================================================================

fn capability_library(version: &str, uses: &[&str]) -> Lib {
    let rust: String = uses
        .iter()
        .map(|capability| format!("// uses: {capability}\n"))
        .chain(std::iter::once(
            "// live: Store\npub struct Store;\n".to_owned(),
        ))
        .collect();
    Lib::acme()
        .at(version)
        .component(
            "store",
            &[("store.html", "<div>Store</div>\n"), ("store.rs", &rust)],
        )
        .manifest("store", "register", json!(["store::Store"]))
}

/// REG-006: `--yes` alone approves no capability, and the refusal writes
/// nothing.
#[test]
fn reg_006_yes_alone_approves_no_capability_and_writes_nothing() {
    let library = capability_library("1.0.0", &["files"]);
    let root = pinned(&library);
    let before = listing(root.path());
    let error = expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/store",
            &fetcher(&[&library]),
            &yes(),
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "--allow files",
    );
    assert!(error.contains("`files`"), "{error}");
    let after: BTreeSet<PathBuf> = listing(root.path())
        .into_iter()
        .filter(|path| path != Path::new(project::LOCK_FILE))
        .collect();
    assert_eq!(after, before, "a refused install left files behind");
}

/// REG-006: `--allow <capability>` approves it by name, and the record
/// holds the approval.
#[test]
fn reg_006_allow_approves_a_capability_by_name_and_the_record_holds_it() {
    let library = capability_library("1.0.0", &["files", "cache"]);
    let root = pinned(&library);
    let partly = Options {
        yes: true,
        allow: BTreeSet::from([Capability::Files]),
        ..Options::default()
    };
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/store",
            &fetcher(&[&library]),
            &partly,
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "`cache`",
    );
    let both = Options {
        yes: true,
        allow: BTreeSet::from([Capability::Files, Capability::Cache]),
        ..Options::default()
    };
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&library]),
        &both,
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("installs");
    let record = &records(root.path())[&component("github.com/acme/acme-ui/store")];
    assert_eq!(
        record.capabilities,
        BTreeMap::from([
            (Capability::Files, project::Approval::Flag),
            (Capability::Cache, project::Approval::Flag),
        ])
    );
}

/// REG-006: on a terminal each capability is asked by name, and a denial
/// refuses the install with nothing written.
#[test]
fn reg_006_a_capability_is_asked_by_name_and_a_denial_writes_nothing() {
    let library = capability_library("1.0.0", &["database"]);
    let root = pinned(&library);
    let mut prompter = terminal(&[false]);
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/store",
            &fetcher(&[&library]),
            &Options::default(),
            &MarkerScanner,
            &mut prompter,
        ),
        "denied",
    );
    assert_eq!(prompter.asked.len(), 1);
    assert!(
        prompter.asked[0].contains("`database`"),
        "{:?}",
        prompter.asked
    );
    assert!(!root.path().join("templates").exists());
    assert!(!root.path().join("src").exists());

    let mut prompter = terminal(&[true, true]);
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&library]),
        &Options::default(),
        &MarkerScanner,
        &mut prompter,
    )
    .expect("approved on the terminal");
    assert_eq!(
        records(root.path())[&component("github.com/acme/acme-ui/store")].capabilities
            [&Capability::Database],
        project::Approval::Terminal
    );
}

/// REG-006: the plan shows every capability the scan found.
#[test]
fn reg_006_the_plan_shows_every_capability_the_scan_found() {
    let library = capability_library("1.0.0", &["mail", "queue"]);
    let root = pinned(&library);
    let project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/acme-ui/store").expect("source"),
        &Options::default(),
        &fetcher(&[&library]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    assert_eq!(
        plan.capabilities(),
        BTreeSet::from([Capability::Mail, Capability::Queue])
    );
    let rendered = plan::render_with(&plan, &Options::default(), &project);
    assert!(
        rendered.contains("github.com/acme/acme-ui/store: capabilities: mail, queue"),
        "{rendered}"
    );
    assert!(
        rendered.contains("approval    mail: needs your approval"),
        "{rendered}"
    );
}

/// REG-006: an update that uses a capability the recorded approval does not
/// hold asks again; one it holds is not asked.
#[test]
fn reg_006_an_update_that_adds_a_capability_asks_again() {
    let first = capability_library("1.0.0", &["files"]);
    let root = pinned(&first);
    let allow_files = Options {
        yes: true,
        allow: BTreeSet::from([Capability::Files]),
        ..Options::default()
    };
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&first]),
        &allow_files,
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("first");
    let second = capability_library("1.1.0", &["files", "network"]);
    let both = fetcher(&[&first, &second]);
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/store",
            &both,
            &yes(),
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "`network`",
    );
    let mut prompter = terminal(&[true]);
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &both,
        &yes(),
        &MarkerScanner,
        &mut prompter,
    )
    .expect("the update installs once network is approved");
    assert_eq!(prompter.asked.len(), 1, "{:?}", prompter.asked);
    assert!(prompter.asked[0].contains("`network`"));
    let record = &records(root.path())[&component("github.com/acme/acme-ui/store")];
    assert_eq!(
        record.capabilities[&Capability::Files],
        project::Approval::Flag
    );
    assert_eq!(
        record.capabilities[&Capability::Network],
        project::Approval::Terminal
    );
}

/// REG-006: a component that uses no capability needs no approval.
#[test]
fn reg_006_a_component_with_no_capability_needs_no_approval() {
    let library = capability_library("1.0.0", &[]);
    let root = pinned(&library);
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&library]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("installs with no approval");
}

/// REG-006, REG-015: `live:add` never changes `Cargo.toml` or writes
/// `Cargo.lock`: no crate enters the application's build.
#[test]
fn reg_006_live_add_never_changes_cargo_toml_or_cargo_lock() {
    let library = capability_library("1.0.0", &[]);
    let root = pinned(&library);
    let manifest = fs::read(root.path().join("Cargo.toml")).expect("manifest");
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&library]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("installs");
    let output = live_add(root.path(), &["field"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(
        fs::read(root.path().join("Cargo.toml")).expect("manifest"),
        manifest
    );
    assert!(!root.path().join("Cargo.lock").exists());
}

/// REG-012: the plan reports every file with its destination and outcome,
/// every module declaration and registration, and the key it would pin.
#[test]
fn reg_012_the_plan_reports_files_modules_registrations_and_the_key() {
    let library = capability_library("1.0.0", &["session"]);
    let root = project();
    let project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/acme-ui/store").expect("source"),
        &Options::default(),
        &fetcher(&[&library]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    let rendered = plan::render_with(&plan, &Options::default(), &project);
    for needle in [
        "templates/acme-ui/store/store.html  new",
        "templates/acme-ui/store/manifest.json  new",
        "src/live/acme/store.rs  new",
        "module      src/live/mod.rs: pub mod acme;",
        "module      src/live/acme/mod.rs: pub mod store;",
        "register    crate::live::acme::store::Store",
        "github.com/acme/acme-ui/store: capabilities: session",
        library.key().fingerprint().as_str(),
        "try_live_ui_assets_for(\"acme\")",
    ] {
        assert!(rendered.contains(needle), "`{needle}` not in:\n{rendered}");
    }
}

/// REG-012: a third-party plan without a terminal is refused unless
/// `--yes` is given, and a declined plan writes nothing.
#[test]
fn reg_012_a_third_party_plan_needs_a_terminal_or_yes() {
    let library = Lib::acme().widget();
    let root = pinned(&library);
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/widget",
            &fetcher(&[&library]),
            &Options::default(),
            &TestScanner,
            &mut no_terminal(),
        ),
        "--yes",
    );
    let mut declined = terminal(&[false]);
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/widget",
            &fetcher(&[&library]),
            &Options::default(),
            &TestScanner,
            &mut declined,
        ),
        "declined",
    );
    assert!(!root.path().join("templates").exists());
    let mut accepted = terminal(&[true]);
    install_with(
        root.path(),
        "acme/acme-ui/widget",
        &fetcher(&[&library]),
        &Options::default(),
        &TestScanner,
        &mut accepted,
    )
    .expect("confirmed on the terminal");
    assert!(
        root.path()
            .join("templates/acme-ui/widget/widget.html")
            .is_file()
    );
}

/// REG-012: a library that changed an installed component's script has the
/// change reported in the plan before it is written.
#[test]
fn reg_012_a_changed_script_is_reported_before_it_is_written() {
    let first = Lib::acme().component(
        "toggle",
        &[
            ("toggle.html", "<button>T</button>\n"),
            ("toggle.js", "export const a = 1;\n"),
        ],
    );
    let root = pinned(&first);
    install_with(
        root.path(),
        "acme/acme-ui/toggle",
        &fetcher(&[&first]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("first");
    let second = Lib::acme().at("1.1.0").component(
        "toggle",
        &[
            ("toggle.html", "<button>T</button>\n"),
            ("toggle.js", "export const a = 2;\n"),
        ],
    );
    let project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/acme-ui/toggle").expect("source"),
        &yes(),
        &fetcher(&[&first, &second]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    let script = plan.components[0]
        .files
        .iter()
        .find(|file| file.name == "toggle.js")
        .expect("script");
    assert_eq!(script.outcome, FileOutcome::Replaced);
    let rendered = plan::render_with(&plan, &yes(), &project);
    assert!(
        rendered.contains("templates/acme-ui/toggle/toggle.js  replaced"),
        "{rendered}"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("templates/acme-ui/toggle/toggle.js")).expect("js"),
        "export const a = 1;\n",
        "resolving wrote the script"
    );
}

// ===========================================================================
// REG-007: the framework check
// ===========================================================================

fn locked_project(framework: &str) -> tempfile::TempDir {
    let root = project();
    fs::write(
        root.path().join("Cargo.lock"),
        format!(
            "version = 4\n\n[[package]]\nname = \"fixture\"\nversion = \"0.1.0\"\ndependencies = [\n \"serde\",\n \"suprnova\",\n]\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"suprnova\"\nversion = \"{framework}\"\nsource = \"git+https://github.com/eas4ai/suprnova.git?tag=v{framework}#0123456789abcdef0123456789abcdef01234567\"\n"
        ),
    )
    .expect("lock");
    root
}

/// REG-007: a component from a library that does not admit the locked
/// framework is refused, naming both.
#[test]
fn reg_007_a_library_the_locked_framework_does_not_admit_is_refused_naming_both() {
    let library = Lib::acme().widget().set("framework", json!(">=4.0.0"));
    let root = locked_project("3.0.0");
    pin(root.path(), ADDRESS, &library.key());
    let error = expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        ">=4.0.0",
    );
    assert!(
        error.contains("3.0.0") && error.contains("Cargo.lock"),
        "{error}"
    );
    let lock = fs::read(root.path().join("Cargo.lock")).expect("lock");
    let admitted = Lib::acme()
        .widget()
        .set("framework", json!(">=3.0.0, <4.0.0"));
    add(root.path(), "acme/acme-ui/widget", &fetcher(&[&admitted])).expect("admitted");
    assert_eq!(
        fs::read(root.path().join("Cargo.lock")).expect("lock"),
        lock,
        "live:add changed Cargo.lock"
    );
}

/// REG-007: with no `Cargo.lock`, the version is the scaffold's
/// `v<version>` tag.
#[test]
fn reg_007_with_no_lock_the_scaffold_tag_is_checked() {
    let library = Lib::acme().widget().set("framework", json!(">=4.0.0"));
    let root = tempfile::tempdir().expect("tempdir");
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n\n[dependencies]\nsuprnova = { git = \"https://github.com/eas4ai/suprnova.git\", tag = \"v3.0.0\" }\n",
    )
    .expect("manifest");
    pin(root.path(), ADDRESS, &library.key());
    let error = expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "3.0.0",
    );
    assert!(error.contains("tag v3.0.0"), "{error}");
    assert!(!root.path().join("Cargo.lock").exists());
}

/// REG-007: with no lock and a framework dependency on a branch, live:add
/// refuses, saying to run `cargo generate-lockfile`.
#[test]
fn reg_007_with_neither_a_lock_nor_a_tag_live_add_refuses() {
    let library = Lib::acme().widget();
    let root = tempfile::tempdir().expect("tempdir");
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n\n[dependencies]\nsuprnova = { git = \"https://github.com/eas4ai/suprnova.git\", branch = \"main\" }\n",
    )
    .expect("manifest");
    pin(root.path(), ADDRESS, &library.key());
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "cargo generate-lockfile",
    );
    assert!(!root.path().join("Cargo.lock").exists());
}

// ===========================================================================
// REG-008: sources and addresses
// ===========================================================================

/// REG-008: `<owner>/<library>/<component>` is a component of the GitHub
/// repository; the forge stand-in shows every request went to GitHub's API
/// and raw hosts.
#[test]
fn reg_008_a_source_with_no_host_is_fetched_from_github() {
    let source = address::parse("acme/acme-ui/date-picker").expect("source");
    assert_eq!(
        source.library_address().expect("library").0,
        "github.com/acme/acme-ui"
    );
    let commit = "c".repeat(40);
    let tags = serde_json::to_vec(&json!([
        {"name": "v1.0.0", "commit": {"sha": commit}},
    ]))
    .expect("tags");
    let tags_path = "/api.github.com/repos/acme/acme-ui/tags?per_page=100&page=1".to_owned();
    let raw = format!("/raw.githubusercontent.com/acme/acme-ui/{commit}/library.json");
    let server = serve(move |path| {
        if path == tags_path {
            Reply::Body(tags.clone())
        } else if path == raw {
            Reply::Body(b"{}".to_vec())
        } else {
            Reply::Status(404)
        }
    });
    let client = client().with_forge_base(&server.base).expect("loopback");
    let library = LibraryAddress(ADDRESS.to_owned());
    assert_eq!(
        client.versions(&library).expect("versions"),
        vec![v("1.0.0")]
    );
    let resolved = client.resolve(&library, &v("1.0.0")).expect("commit");
    assert_eq!(resolved, Commit(commit.clone()));
    assert_eq!(
        client
            .file(&library, &resolved, "library.json")
            .expect("file"),
        b"{}"
    );
    for request in server.requests.lock().expect("requests").iter() {
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        assert!(
            path.starts_with("/api.github.com/") || path.starts_with("/raw.githubusercontent.com/"),
            "{path}"
        );
    }
}

/// REG-008: two spellings of one component record one address: case, a
/// trailing slash, the directory or its `manifest.json`.
#[test]
fn reg_008_spellings_of_one_component_record_one_address() {
    let same = |a: &str, b: &str| {
        assert_eq!(
            address::parse(a).expect(a).component_address().expect(a),
            address::parse(b).expect(b).component_address().expect(b),
            "{a} and {b}"
        );
    };
    same("Acme/Acme-UI/date-picker", "acme/acme-ui/date-picker");
    same(
        "GITHUB.COM/acme/acme-ui/date-picker",
        "acme/acme-ui/date-picker@1.0.0",
    );
    same(
        "https://Example.test/lib/components/widget/",
        "https://example.test:443/lib/components/widget",
    );
    let tree = tempfile::tempdir().expect("tree");
    Lib::acme().widget().write_to(tree.path());
    let directory = tree.path().join("components/widget");
    let manifest = directory.join("manifest.json");
    same(
        directory.to_str().expect("utf-8"),
        manifest.to_str().expect("utf-8"),
    );
    let canonical = fs::canonicalize(tree.path()).expect("canonical");
    assert_eq!(
        address::parse(manifest.to_str().expect("utf-8"))
            .expect("path")
            .library_address()
            .expect("library")
            .0,
        canonical.to_str().expect("utf-8")
    );
}

/// REG-008: a URL carrying a user, a query, a fragment, a percent escape,
/// or a `.` or `..` segment is refused before anything is fetched.
#[test]
fn reg_008_a_hostile_url_is_refused_before_anything_is_fetched() {
    for hostile in [
        "https://user@example.test/components/widget",
        "https://user:pass@example.test/components/widget",
        "https://example.test/components/widget?ref=main",
        "https://example.test/components/widget#x",
        "https://example.test/a%2F..%2Fb/components/widget",
        "https://example.test/./components/widget",
        "https://example.test/a/../components/widget",
        "https://example.test/.hidden/components/widget",
        "https://exa mple.test/components/widget",
    ] {
        assert!(address::parse(hostile).is_err(), "{hostile}");
    }
}

/// REG-008: a URL source fetches only inside its library's base.
#[test]
fn reg_008_a_url_source_fetches_only_inside_its_library_base() {
    // The library's address names the server it is served from, so the
    // server is started first with an empty tree, then given the tree.
    let tree: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
    let served = Arc::clone(&tree);
    let server = serve(move |path| {
        match path
            .strip_prefix("/vendor/acme-ui/")
            .and_then(|rest| served.lock().expect("tree").get(rest).cloned())
        {
            Some(bytes) => Reply::Body(bytes),
            None => Reply::Status(404),
        }
    });
    let base = format!("{}/vendor/acme-ui", server.base);
    let library = Lib::new(&base, "acme", "1.0.0").widget();
    *tree.lock().expect("tree") = library.tree();
    let root = pinned(&library);
    let fetcher = SourceFetcher::new(client());
    add(root.path(), &format!("{base}/components/widget"), &fetcher)
        .expect("installs from the URL");
    assert!(
        root.path()
            .join("templates/acme-ui/widget/widget.html")
            .is_file()
    );
    for request in server.requests.lock().expect("requests").iter() {
        let path = request.split_whitespace().nth(1).unwrap_or_default();
        assert!(path.starts_with("/vendor/acme-ui/"), "{path}");
    }
}

/// REG-008: a redirect that changes the repository path, as a renamed
/// repository's does, is refused, naming the new address.
#[test]
fn reg_008_a_redirect_to_another_repository_path_is_refused_naming_it() {
    let server = serve(|path| {
        if path == "/acme/acme-ui/library.json" {
            Reply::Redirect("/acme/renamed-ui/library.json".to_owned())
        } else {
            Reply::Status(404)
        }
    });
    let library = LibraryAddress(format!("{}/acme/acme-ui", server.base));
    let error = client()
        .file(&library, &Commit(String::new()), "library.json")
        .expect_err("refused");
    assert!(
        error.to_string().contains("/acme/renamed-ui/library.json"),
        "{error}"
    );
}

/// REG-008: a path source installs from its library tree, the author's own:
/// its `source` names where it will be published, not where it sits.
#[test]
fn reg_008_a_path_source_installs_from_the_fixture_library_tree() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/acme-ui");
    let canonical = fs::canonicalize(&fixture).expect("fixture");
    let root = project();
    pin(
        root.path(),
        canonical.to_str().expect("utf-8"),
        &fixture_secret().public_key(),
    );
    let source = fixture.join("components/panel/manifest.json");
    let plan = add(
        root.path(),
        source.to_str().expect("utf-8"),
        &SourceFetcher::default(),
    )
    .expect("installs from the tree on disk");
    let order: Vec<String> = plan
        .components
        .iter()
        .map(|component| component.address.component.clone())
        .collect();
    assert_eq!(order, ["widget", "panel"]);
    for file in [
        "widget/widget.html",
        "widget/widget.css",
        "panel/panel.html",
    ] {
        assert!(
            root.path().join("templates/acme-ui").join(file).is_file(),
            "{file}"
        );
    }
}

// ===========================================================================
// REG-009: fetching
// ===========================================================================

/// REG-009: plain HTTP is refused except from a loopback host.
#[test]
fn reg_009_plain_http_is_refused_except_from_a_loopback_host() {
    assert!(address::parse("http://example.test/components/widget").is_err());
    assert!(address::parse("http://10.0.0.1/components/widget").is_err());
    let library = LibraryAddress("http://example.test".to_owned());
    assert!(
        client()
            .file(&library, &Commit(String::new()), "library.json")
            .is_err()
    );
    assert!(
        HttpsClient::default()
            .with_forge_base("http://example.test")
            .is_err()
    );
}

/// REG-009: a redirect to another origin is refused.
#[test]
fn reg_009_a_redirect_to_another_origin_is_refused() {
    let other = serve(|_| Reply::Body(b"{}".to_vec()));
    let target = format!("{}/lib/library.json", other.base);
    let server = serve(move |_| Reply::Redirect(target.clone()));
    let library = LibraryAddress(format!("{}/lib", server.base));
    let error = client()
        .file(&library, &Commit(String::new()), "library.json")
        .expect_err("refused");
    assert!(error.to_string().contains(&other.base), "{error}");
    assert!(
        other.requests.lock().expect("requests").is_empty(),
        "the redirect was followed"
    );
}

/// REG-009: a response of more than 2 MiB is refused, with or without a
/// `Content-Length`.
#[test]
fn reg_009_a_three_mib_response_is_refused() {
    let three = 3 * 1024 * 1024;
    let server = serve(move |path| {
        if path.ends_with("declared.json") {
            Reply::Body(vec![b' '; three])
        } else {
            Reply::Unbounded(three)
        }
    });
    let library = LibraryAddress(format!("{}/lib", server.base));
    for file in ["declared.json", "streamed.json"] {
        let error = client()
            .file(&library, &Commit(String::new()), file)
            .expect_err(file);
        assert!(
            error.to_string().contains(&MAX_RESPONSE_BYTES.to_string()),
            "{error}"
        );
    }
}

/// REG-009: a response that stalls past the timeout is refused.
#[test]
fn reg_009_a_stalled_response_is_refused() {
    let server = serve(|_| Reply::Stall);
    let library = LibraryAddress(format!("{}/lib", server.base));
    let started = Instant::now();
    let error = HttpsClient::new(Duration::from_millis(500))
        .file(&library, &Commit(String::new()), "library.json")
        .expect_err("refused");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(error.to_string().contains("took more than"), "{error}");
}

/// REG-009: a JSON document that holds a duplicate key is refused, a
/// manifest that names `files` twice included.
#[test]
fn reg_009_a_manifest_naming_files_twice_is_refused() {
    let library = Lib::acme().widget();
    let mut tree = library.tree();
    tree.insert(
        "components/widget/manifest.json".to_owned(),
        br#"{"name":"acme.widget","files":["widget.html"],"files":["widget.html","evil.rs"]}"#
            .to_vec(),
    );
    let root = pinned(&library);
    expect_refused(
        add(
            root.path(),
            "acme/acme-ui/widget",
            &fetcher_with(&[(&library, tree)]),
        ),
        "appears twice",
    );
    assert!(library::strict_json_object(br#"{"a":{"b":1,"b":2}}"#).is_err());
    assert!(library::strict_json_object(br#"[{"a":1}]"#).is_err());
}

/// REG-009: no request carries credentials: no authorization, cookie or
/// proxy header, whatever the environment holds.
#[test]
fn reg_009_no_request_carries_credentials() {
    let server = serve(|_| Reply::Body(b"{}".to_vec()));
    let library = LibraryAddress(format!("{}/lib", server.base));
    client()
        .file(&library, &Commit(String::new()), "library.json")
        .expect("fetch");
    let requests = server.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    let lowered = requests[0].to_ascii_lowercase();
    for header in ["authorization:", "cookie:", "proxy-authorization:"] {
        assert!(!lowered.contains(header), "{}", requests[0]);
    }
}

/// REG-009, REG-026: a repository's files are fetched raw at the resolved
/// commit, from each forge, never as release assets.
#[test]
fn reg_009_repository_files_are_fetched_raw_at_the_commit_from_each_forge() {
    let commit = "d".repeat(40);
    let raw_paths = [
        format!("/raw.githubusercontent.com/acme/acme-ui/{commit}/library.json"),
        format!("/gitlab.com/acme/acme-ui/-/raw/{commit}/library.json"),
        format!("/codeberg.org/acme/acme-ui/raw/commit/{commit}/library.json"),
    ];
    let expected = raw_paths.clone();
    let server = serve(move |path| {
        if expected.iter().any(|raw| raw == path) {
            Reply::Body(b"raw".to_vec())
        } else {
            Reply::Status(404)
        }
    });
    let client = client().with_forge_base(&server.base).expect("loopback");
    for host in ["github.com", "gitlab.com", "codeberg.org"] {
        let library = LibraryAddress(format!("{host}/acme/acme-ui"));
        assert_eq!(
            client
                .file(&library, &Commit(commit.clone()), "library.json")
                .expect(host),
            b"raw"
        );
    }
    for request in server.requests.lock().expect("requests").iter() {
        assert!(!request.contains("releases"), "{request}");
    }
}

// ===========================================================================
// REG-010: dependencies
// ===========================================================================

/// REG-010: dependencies install first, each once, and a dependency that
/// names its parent back neither hangs nor installs twice.
#[test]
fn reg_010_dependencies_install_first_and_once_even_in_a_cycle() {
    let library = Lib::acme()
        .component("a", &[("a.html", "<p>A</p>\n")])
        .component("b", &[("b.html", "<p>B</p>\n")])
        .component("c", &[("c.html", "<p>C</p>\n")])
        .depends("a", &["./b", "./c"])
        .depends("b", &["./a", "./c", "field"])
        .depends("c", &["./b"]);
    let root = pinned(&library);
    let plan = add(root.path(), "acme/acme-ui/a", &fetcher(&[&library])).expect("installs");
    let order: Vec<String> = plan
        .components
        .iter()
        .map(|component| component.address.to_string())
        .collect();
    let unique: BTreeSet<&String> = order.iter().collect();
    assert_eq!(unique.len(), order.len(), "{order:?}");
    assert_eq!(
        order.last().map(String::as_str),
        Some("github.com/acme/acme-ui/a")
    );
    let position = |name: &str| {
        order
            .iter()
            .position(|address| address == name)
            .expect(name)
    };
    assert!(position("suprnova/field") < position("github.com/acme/acme-ui/b"));
    assert!(
        root.path()
            .join("templates/suprnova-ui/field/field.html")
            .is_file()
    );
}

/// REG-010: a plan of more than 64 components is refused; 64 is admitted.
#[test]
fn reg_010_a_plan_of_more_than_64_components_is_refused() {
    let chain = |length: usize| {
        let mut library = Lib::acme();
        for index in 0..length {
            library = library.component(
                &format!("c{index}"),
                &[(&format!("c{index}.html"), "<p>c</p>\n")],
            );
            if index + 1 < length {
                library = library.depends(&format!("c{index}"), &[&format!("./c{}", index + 1)]);
            }
        }
        library
    };
    let long = chain(65);
    let root = pinned(&long);
    expect_refused(
        resolve(root.path(), "acme/acme-ui/c0", &fetcher(&[&long])),
        "64",
    );
    let fits = chain(64);
    let plan = resolve(root.path(), "acme/acme-ui/c0", &fetcher(&[&fits])).expect("64 fit");
    assert_eq!(plan.components.len(), 64);
}

/// REG-010: a `./` dependency installs from the same version of its
/// library, not the newest one.
#[test]
fn reg_010_a_sibling_dependency_installs_from_the_same_version() {
    let old = Lib::acme()
        .component("a", &[("a.html", "<p>A1</p>\n")])
        .component("b", &[("b.html", "<p>B1</p>\n")])
        .depends("a", &["./b"]);
    let new = Lib::acme()
        .at("2.0.0")
        .component("a", &[("a.html", "<p>A2</p>\n")])
        .component("b", &[("b.html", "<p>B2</p>\n")])
        .depends("a", &["./b"]);
    let root = pinned(&old);
    add(root.path(), "acme/acme-ui/a@1.0.0", &fetcher(&[&old, &new])).expect("installs");
    assert_eq!(
        fs::read_to_string(root.path().join("templates/acme-ui/b/b.html")).expect("b"),
        "<p>B1</p>\n"
    );
    let record = &records(root.path())[&component("github.com/acme/acme-ui/a")];
    assert_eq!(
        record.dependencies,
        BTreeMap::from([(component("github.com/acme/acme-ui/b"), v("1.0.0"))])
    );
}

/// REG-010: a plan that resolves one address to two versions is refused
/// before anything is written, naming the components that require each.
#[test]
fn reg_010_one_address_at_two_versions_is_refused_naming_both_requirers() {
    let helper_one = Lib::new("github.com/kit/helpers", "kit", "1.0.0")
        .component("helper", &[("helper.html", "<p>1</p>\n")]);
    let helper_two = Lib::new("github.com/kit/helpers", "kit", "2.0.0")
        .component("helper", &[("helper.html", "<p>2</p>\n")]);
    let library = Lib::acme()
        .component("root", &[("root.html", "<p>R</p>\n")])
        .component("left", &[("left.html", "<p>L</p>\n")])
        .component("right", &[("right.html", "<p>R</p>\n")])
        .depends("root", &["./left", "./right"])
        .depends("left", &["kit/helpers/helper@1.0.0"])
        .depends("right", &["kit/helpers/helper@2.0.0"]);
    let root = pinned(&library);
    pin(root.path(), "github.com/kit/helpers", &helper_one.key());
    let error = expect_refused(
        add(
            root.path(),
            "acme/acme-ui/root",
            &fetcher(&[&library, &helper_one, &helper_two]),
        ),
        "two versions",
    );
    assert!(
        error.contains("github.com/acme/acme-ui/left")
            && error.contains("github.com/acme/acme-ui/right"),
        "{error}"
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-010: replacing a component with a version other than the one an
/// installed dependent recorded names the dependent and is refused unless
/// `--force` is given.
#[test]
fn reg_010_replacing_a_dependency_an_installed_dependent_recorded_is_refused() {
    let helper_one = Lib::new("github.com/kit/helpers", "kit", "1.0.0")
        .component("helper", &[("helper.html", "<p>1</p>\n")]);
    let helper_two = Lib::new("github.com/kit/helpers", "kit", "2.0.0")
        .component("helper", &[("helper.html", "<p>2</p>\n")]);
    let library = Lib::acme()
        .component("user", &[("user.html", "<p>U</p>\n")])
        .depends("user", &["kit/helpers/helper@1.0.0"]);
    let root = pinned(&library);
    pin(root.path(), "github.com/kit/helpers", &helper_one.key());
    let all = fetcher(&[&library, &helper_one, &helper_two]);
    add(root.path(), "acme/acme-ui/user", &all).expect("installs");
    let error = expect_refused(add(root.path(), "kit/helpers/helper", &all), "--force");
    assert!(error.contains("github.com/acme/acme-ui/user"), "{error}");
    let forced = Options {
        force: true,
        yes: true,
        ..Options::default()
    };
    install_with(
        root.path(),
        "kit/helpers/helper",
        &all,
        &forced,
        &TestScanner,
        &mut no_terminal(),
    )
    .expect("forced");
}

// ===========================================================================
// REG-011: namespaces
// ===========================================================================

/// REG-011: a namespace stays with the library it was first installed
/// from, including two libraries on one host under different ports or
/// paths, and within one plan.
#[test]
fn reg_011_a_namespace_owned_by_another_library_is_refused_naming_both() {
    let original = Lib::acme().widget();
    let impostor = Lib::new("github.com/evil/acme-ui", "acme", "1.0.0").widget();
    let root = pinned(&original);
    pin(root.path(), "github.com/evil/acme-ui", &impostor.key());
    let both = fetcher(&[&original, &impostor]);
    add(root.path(), "acme/acme-ui/widget", &both).expect("first");
    let error = expect_refused(
        add(root.path(), "evil/acme-ui/widget", &both),
        "github.com/acme/acme-ui",
    );
    assert!(error.contains("github.com/evil/acme-ui"), "{error}");

    let first = Lib::new("http://127.0.0.1:8001/lib", "acme", "1.0.0").widget();
    let second = Lib::new("http://127.0.0.1:8002/lib", "acme", "1.0.0").widget();
    let root = pinned(&first);
    pin(root.path(), &second.source(), &second.key());
    let ports = fetcher(&[&first, &second]);
    add(
        root.path(),
        "http://127.0.0.1:8001/lib/components/widget",
        &ports,
    )
    .expect("first");
    expect_refused(
        add(
            root.path(),
            "http://127.0.0.1:8002/lib/components/widget",
            &ports,
        ),
        "http://127.0.0.1:8001/lib",
    );

    let claimant = Lib::new("github.com/other/kit", "acme", "1.0.0")
        .component("thing", &[("thing.html", "<p>T</p>\n")]);
    let parent = Lib::new("github.com/team/app-ui", "team", "1.0.0")
        .component("page", &[("page.html", "<p>P</p>\n")])
        .depends("page", &["acme/acme-ui/widget", "other/kit/thing"]);
    let root = pinned(&parent);
    pin(root.path(), ADDRESS, &original.key());
    pin(root.path(), "github.com/other/kit", &claimant.key());
    expect_refused(
        add(
            root.path(),
            "team/app-ui/page",
            &fetcher(&[&parent, &original, &claimant]),
        ),
        "both claim the namespace `acme`",
    );
}

// ===========================================================================
// REG-013: the provenance record
// ===========================================================================

/// REG-013: the record holds exactly what arrived: the source, version,
/// commit, the digests of `library.json`, the manifest and each file, the
/// hash and the signature.
#[test]
fn reg_013_the_record_holds_exactly_what_arrived() {
    let library = Lib::acme().widget();
    let tree = library.tree();
    let root = pinned(&library);
    let fake = fetcher(&[&library]);
    add(root.path(), "acme/acme-ui/widget", &fake).expect("installs");
    let record = &records(root.path())[&component("github.com/acme/acme-ui/widget")];
    assert_eq!(record.source, ADDRESS);
    assert_eq!(record.version, v("1.0.0"));
    assert_eq!(
        Commit(record.commit.clone()),
        fake.resolve(&library.address(), &v("1.0.0"))
            .expect("commit")
    );
    assert_eq!(record.library_json, Digest::of(&tree["library.json"]));
    assert_eq!(
        record.manifest,
        Digest::of(&tree["components/widget/manifest.json"])
    );
    assert_eq!(
        record.files,
        BTreeMap::from([
            (
                "widget.css".to_owned(),
                Digest::of(&tree["components/widget/widget.css"])
            ),
            (
                "widget.html".to_owned(),
                Digest::of(&tree["components/widget/widget.html"])
            ),
        ])
    );
    assert_eq!(record.hash, library.statement("widget").verification_hash());
    assert_eq!(
        record.signature.encode(),
        String::from_utf8(tree["components/widget/manifest.sig"].clone()).expect("sig")
    );
    assert!(record.capabilities.is_empty() && record.kept.is_empty());
    let text = fs::read_to_string(root.path().join("suprnova.toml")).expect("read");
    assert!(text.contains("namespace = \"acme\""), "{text}");
}

/// REG-013: a shipped component's table holds the CLI's version and its
/// file digests, and no hash or signature.
#[test]
fn reg_013_a_shipped_component_records_the_cli_version_and_digests_only() {
    let root = project();
    let output = live_add(root.path(), &["field"]);
    assert!(output.status.success(), "{}", combined(&output));
    let project = ProjectFile::load(root.path()).expect("load");
    let shipped = project.shipped().expect("shipped");
    let record = &shipped[&component("suprnova/field")];
    assert_eq!(record.version, v(env!("CARGO_PKG_VERSION")));
    let installed =
        fs::read(root.path().join("templates/suprnova-ui/field/field.html")).expect("view");
    assert_eq!(record.files["field.html"], Digest::of(&installed));
    let text = fs::read_to_string(root.path().join("suprnova.toml")).expect("read");
    let table = text
        .split("[live.components.\"suprnova/field\"]")
        .nth(1)
        .expect("table");
    assert!(
        !table.contains("hash") && !table.contains("signature"),
        "{text}"
    );
    assert!(project.components().expect("records").is_empty());
}

/// REG-013, REG-005: the recorded registration is the full path the
/// registration line wrote, and an update that drops a type unregisters it.
#[test]
fn reg_013_the_recorded_registration_is_the_line_written() {
    let first = Lib::acme()
        .component(
            "chart",
            &[
                ("chart.html", "<div>C</div>\n"),
                (
                    "chart.rs",
                    "// live: Chart\n// live: Legend\npub struct Chart;\npub struct Legend;\n",
                ),
            ],
        )
        .manifest(
            "chart",
            "register",
            json!(["chart::Chart", "chart::Legend"]),
        );
    let root = pinned(&first);
    install_with(
        root.path(),
        "acme/acme-ui/chart",
        &fetcher(&[&first]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("installs");
    let written = fs::read_to_string(root.path().join("src/live/mod.rs")).expect("mod.rs");
    let record = &records(root.path())[&component("github.com/acme/acme-ui/chart")];
    for path in &record.registered {
        assert!(
            written.contains(&format!("// register {path}\n")),
            "{written}"
        );
    }
    assert_eq!(
        record.registered,
        vec![
            "crate::live::acme::chart::Chart".to_owned(),
            "crate::live::acme::chart::Legend".to_owned()
        ]
    );
    let second = Lib::acme()
        .at("1.1.0")
        .component(
            "chart",
            &[
                ("chart.html", "<div>C</div>\n"),
                ("chart.rs", "// live: Chart\npub struct Chart;\n"),
            ],
        )
        .manifest("chart", "register", json!(["chart::Chart"]));
    install_with(
        root.path(),
        "acme/acme-ui/chart",
        &fetcher(&[&first, &second]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("update");
    let written = fs::read_to_string(root.path().join("src/live/mod.rs")).expect("mod.rs");
    assert!(!written.contains("Legend"), "{written}");
    assert_eq!(
        records(root.path())[&component("github.com/acme/acme-ui/chart")].registered,
        vec!["crate::live::acme::chart::Chart".to_owned()]
    );
}

/// REG-013, REG-028: an install that keeps an edited Rust file records the
/// capabilities of the Rust that ends up installed and names the kept
/// file, while the digests stay those of what arrived.
#[test]
fn reg_013_a_kept_rust_file_records_its_own_capabilities_and_is_named() {
    let first = capability_library("1.0.0", &[]);
    let root = pinned(&first);
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&first]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("first");
    let rust = root.path().join("src/live/acme/store.rs");
    fs::write(
        &rust,
        "// uses: files\npub struct Store;\n// edited by the application\n",
    )
    .expect("edit");
    let second = capability_library("1.1.0", &["mail"]);
    let options = Options {
        yes: true,
        allow: BTreeSet::from([Capability::Files]),
        ..Options::default()
    };
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&first, &second]),
        &options,
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("update keeping the edit");
    assert!(
        fs::read_to_string(&rust)
            .expect("rust")
            .contains("edited by the application"),
        "the edited file was overwritten"
    );
    let record = &records(root.path())[&component("github.com/acme/acme-ui/store")];
    assert_eq!(
        record.capabilities.keys().copied().collect::<Vec<_>>(),
        vec![Capability::Files],
        "the capabilities are the incoming file's, not the kept one's"
    );
    assert_eq!(record.kept, vec!["store.rs".to_owned()]);
    let incoming = second.tree();
    assert_eq!(
        record.files["store.rs"],
        Digest::of(&incoming["components/store/store.rs"]),
        "the record holds the digest of the edited file"
    );
}

/// REG-005: an update that changes what a kept Rust file registers is
/// refused before writing, naming the file to reconcile.
#[test]
fn reg_005_an_update_that_renames_a_type_in_a_kept_file_is_refused() {
    let first = capability_library("1.0.0", &[]);
    let root = pinned(&first);
    install_with(
        root.path(),
        "acme/acme-ui/store",
        &fetcher(&[&first]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect("first");
    let rust = root.path().join("src/live/acme/store.rs");
    fs::write(&rust, "pub struct Store; // edited\n").expect("edit");
    let renamed = Lib::acme()
        .at("1.1.0")
        .component(
            "store",
            &[
                ("store.html", "<div>Store</div>\n"),
                ("store.rs", "// live: Shop\npub struct Shop;\n"),
            ],
        )
        .manifest("store", "register", json!(["store::Shop"]));
    expect_refused(
        install_with(
            root.path(),
            "acme/acme-ui/store",
            &fetcher(&[&first, &renamed]),
            &yes(),
            &MarkerScanner,
            &mut no_terminal(),
        ),
        "src/live/acme/store.rs",
    );
    let written = fs::read_to_string(root.path().join("src/live/mod.rs")).expect("mod.rs");
    assert!(!written.contains("Shop"), "{written}");
}

/// REG-005, REG-030: each `#[live]` type the scan finds must be exactly one
/// `register` entry, and each entry a type the Rust defines.
#[test]
fn reg_005_a_register_that_differs_from_the_live_types_is_refused() {
    for (rust, register) in [
        (
            "// live: Card\n// live: Extra\npub struct Card;\n",
            json!(["card::Card"]),
        ),
        (
            "// live: Card\npub struct Card;\n",
            json!(["card::Card", "card::Ghost"]),
        ),
        ("pub struct Card;\n", json!(["card::Card"])),
    ] {
        let library = Lib::acme()
            .component(
                "card",
                &[("card.html", "<div>Card</div>\n"), ("card.rs", rust)],
            )
            .manifest("card", "register", register);
        let root = pinned(&library);
        expect_refused(
            install_with(
                root.path(),
                "acme/acme-ui/card",
                &fetcher(&[&library]),
                &yes(),
                &MarkerScanner,
                &mut no_terminal(),
            ),
            "exactly one `register` entry",
        );
        assert!(!root.path().join("src").exists());
    }
}

// ===========================================================================
// REG-014: dry runs
// ===========================================================================

/// REG-014: a dry run writes nothing: no file, no `suprnova.toml`, no lock
/// and no journal.
#[test]
fn reg_014_a_dry_run_writes_nothing() {
    let root = project();
    let before = listing(root.path());
    let output = live_add(root.path(), &["field", "--dry-run"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("templates/suprnova-ui/field/field.html  new"));
    assert_eq!(listing(root.path()), before);

    let library = Lib::acme().widget();
    let root = pinned(&library);
    let before = listing(root.path());
    let plan = resolve(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("plan");
    assert_eq!(plan.components.len(), 1);
    assert_eq!(listing(root.path()), before, "resolving wrote files");
}

/// REG-014: a dry run makes the same framework check an install makes.
#[test]
fn reg_014_a_dry_run_makes_the_framework_check_an_install_makes() {
    let library = Lib::acme().widget().set("framework", json!(">=9.0.0"));
    let root = pinned(&library);
    let project = ProjectFile::load(root.path()).expect("load");
    let source = address::parse("acme/acme-ui/widget").expect("source");
    let dry = Options {
        dry_run: true,
        ..Options::default()
    };
    let dry_error =
        plan::resolve_with(&source, &dry, &fetcher(&[&library]), &project, &TestScanner)
            .expect_err("refused");
    let install_error =
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect_err("refused");
    assert_eq!(dry_error.to_string(), install_error.to_string());
    let fine = Lib::acme().widget();
    let plan = plan::resolve_with(&source, &dry, &fetcher(&[&fine]), &project, &TestScanner)
        .expect("plan");
    assert_eq!(plan.framework_version, v("3.2.1"));
}

// ===========================================================================
// REG-015: nothing runs
// ===========================================================================

/// REG-015: `live:add` starts no process: no source on its path names a
/// process, and an install succeeds with no program reachable on `PATH`.
#[test]
fn reg_015_live_add_starts_no_process() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![
        source.join("commands/live_add.rs"),
        source.join("secure_fs.rs"),
    ];
    for entry in walk(&source.join("registry")) {
        if entry.extension().and_then(|extension| extension.to_str()) == Some("rs")
            && entry.file_name().and_then(|name| name.to_str()) != Some("registry_commands.rs")
        {
            files.push(entry);
        }
    }
    for file in files {
        let text = fs::read_to_string(&file).expect("source");
        for needle in ["process::Command", "Command::new", "process::Stdio"] {
            assert!(!text.contains(needle), "{} names {needle}", file.display());
        }
    }
    let root = project();
    let output = Command::new(BIN)
        .args(["live:add", "field"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .env("CARGO", "/nonexistent/cargo")
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", combined(&output));
}

// ===========================================================================
// REG-023, REG-024, REG-033: the hash, the signature, the key
// ===========================================================================

fn fixture_secret() -> SecretKey {
    use base64::Engine as _;
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/registry/acme-ui.test-only-secret-key"),
    )
    .expect("fixture key");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .expect("base64");
    SecretKey::from_bytes(bytes.try_into().expect("32 bytes"))
}

/// REG-024: the fixture library is signed by its fixture key: its
/// `library.json` names the key, and each `manifest.sig` verifies.
#[test]
fn reg_024_the_fixture_library_is_signed_by_its_fixture_key() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry/acme-ui");
    let secret = fixture_secret();
    let library_json = fs::read(root.join("library.json")).expect("library.json");
    let library = library::parse_library_json(&library_json).expect("parse");
    assert_eq!(
        library.public_key,
        secret.public_key(),
        "library.json must name publicKey {}",
        secret.public_key().encode()
    );
    for directory in ["widget", "panel"] {
        let component = root.join("components").join(directory);
        let manifest_bytes = fs::read(component.join("manifest.json")).expect("manifest");
        let manifest = library::parse_manifest(&manifest_bytes, directory, &library.namespace)
            .expect("manifest parses");
        let statement = Statement {
            library: library.source.clone(),
            version: library.version.clone(),
            component: directory.to_owned(),
            library_json: Digest::of(&library_json),
            manifest: Digest::of(&manifest_bytes),
            files: manifest
                .files
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        Digest::of(&fs::read(component.join(name)).expect("file")),
                    )
                })
                .collect(),
        };
        let hash = statement.verification_hash();
        let expected = signing::sign(&secret, &hash).expect("sign");
        let written = fs::read_to_string(component.join("manifest.sig")).expect("sig");
        assert_eq!(
            written,
            expected.encode(),
            "components/{directory}/manifest.sig must hold {}",
            expected.encode()
        );
    }
}

/// REG-023: a component whose `library.json`, manifest or any file changed
/// by one byte after signing is refused, before it is scanned.
#[test]
fn reg_023_one_byte_changed_after_signing_is_refused() {
    struct NeverScanner;
    impl Scanner for NeverScanner {
        fn scan(&self, _component: &ComponentFiles<'_>) -> Result<ScanReport, RegistryError> {
            panic!("an unverified component was scanned")
        }
    }
    let library = Lib::acme().widget();
    for path in [
        "library.json",
        "components/widget/manifest.json",
        "components/widget/widget.html",
        "components/widget/widget.css",
    ] {
        let mut tree = library.tree();
        let bytes = tree.get_mut(path).expect(path);
        let at = bytes
            .iter()
            .position(|byte| *byte == b' ')
            .expect("a space");
        bytes[at] = b'\t';
        let root = pinned(&library);
        let project = ProjectFile::load(root.path()).expect("load");
        let error = plan::resolve_with(
            &address::parse("acme/acme-ui/widget").expect("source"),
            &yes(),
            &fetcher_with(&[(&library, tree)]),
            &project,
            &NeverScanner,
        )
        .expect_err(path);
        assert!(
            error.to_string().contains("does not verify"),
            "{path}: {error}"
        );
    }
}

/// REG-023: a component signed under one directory name is refused when
/// another was requested.
#[test]
fn reg_023_a_component_signed_under_another_directory_name_is_refused() {
    let mut library = Lib::acme().component("gadget", &[("gadget.html", "<p>G</p>\n")]);
    library
        .components
        .get_mut("gadget")
        .expect("gadget")
        .signed_as = Some("widget".to_owned());
    let root = pinned(&library);
    expect_refused(
        add(root.path(), "acme/acme-ui/gadget", &fetcher(&[&library])),
        "does not verify",
    );
}

/// REG-023: a fork that keeps the author's `library.json`, files and
/// signatures is refused from the fork's address.
#[test]
fn reg_023_a_fork_keeping_the_authors_library_json_is_refused() {
    let library = Lib::acme().widget();
    let mut fork = FakeFetcher::default();
    fork.add_version(
        LibraryAddress("github.com/fork/acme-ui".to_owned()),
        v("1.0.0"),
        library.tree(),
    );
    let root = pinned(&library);
    pin(root.path(), "github.com/fork/acme-ui", &library.key());
    expect_refused(
        add(root.path(), "fork/acme-ui/widget", &fork),
        "not the address it was fetched from",
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-024: an unsigned component, or one signed by another key, is
/// refused.
#[test]
fn reg_024_an_unsigned_or_wrongly_signed_component_is_refused() {
    let mut unsigned = Lib::acme().widget();
    unsigned
        .components
        .get_mut("widget")
        .expect("widget")
        .signature = Some(None);
    let root = pinned(&unsigned);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&unsigned])),
        "unsigned",
    );
    let stranger = signing::sign(
        &SecretKey::from_bytes([9; 32]),
        &Lib::acme().widget().statement("widget").verification_hash(),
    )
    .expect("sign")
    .encode();
    let mut forged = Lib::acme().widget();
    forged
        .components
        .get_mut("widget")
        .expect("widget")
        .signature = Some(Some(stranger));
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&forged])),
        "does not verify",
    );
    let mut garbage = Lib::acme().widget();
    garbage
        .components
        .get_mut("widget")
        .expect("widget")
        .signature = Some(Some("not base64!".to_owned()));
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&garbage])),
        "manifest.sig",
    );
    // manifest.sig holds only the base64: not even a trailing line break.
    let valid =
        String::from_utf8(Lib::acme().widget().tree()["components/widget/manifest.sig"].clone())
            .expect("signature");
    let mut padded = Lib::acme().widget();
    padded
        .components
        .get_mut("widget")
        .expect("widget")
        .signature = Some(Some(format!("{valid}\n")));
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&padded])),
        "manifest.sig",
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-024: a library whose key differs from its pin is refused, naming
/// both fingerprints.
#[test]
fn reg_024_a_key_that_differs_from_the_pin_is_refused_naming_both() {
    let library = Lib::acme().widget().signed_by([3; 32]);
    let root = project();
    pin(root.path(), ADDRESS, &public_key(SEED));
    let error = expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        public_key([3; 32]).fingerprint().as_str(),
    );
    assert!(
        error.contains(public_key(SEED).fingerprint().as_str()),
        "{error}"
    );
}

/// REG-024: the first install pins the key only once confirmed on a
/// terminal; `--yes` does not pin it, and a key pinned by hand needs no
/// terminal.
#[test]
fn reg_024_a_new_key_is_pinned_only_on_a_terminal() {
    let library = Lib::acme().widget();
    let root = project();
    let error = expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])),
        "needs a terminal",
    );
    assert!(error.contains(&library.key().encode()), "{error}");
    assert!(!root.path().join("templates").exists());
    assert!(
        ProjectFile::load(root.path())
            .expect("load")
            .pinned_key(&library.address())
            .expect("pin")
            .is_none()
    );

    let mut prompter = terminal(&[true, true]);
    install_with(
        root.path(),
        "acme/acme-ui/widget",
        &fetcher(&[&library]),
        &Options::default(),
        &TestScanner,
        &mut prompter,
    )
    .expect("pinned on the terminal");
    assert!(prompter.asked[0].contains(library.key().fingerprint().as_str()));
    assert_eq!(
        ProjectFile::load(root.path())
            .expect("load")
            .pinned_key(&library.address())
            .expect("pin"),
        Some(library.key())
    );

    let by_hand = project();
    fs::write(
        by_hand.path().join("suprnova.toml"),
        format!(
            "[live.libraries.\"{ADDRESS}\"]\nkey = \"{}\"\n",
            library.key().encode()
        ),
    )
    .expect("pin by hand");
    add(by_hand.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("hand-pinned");
}

/// REG-033: a key change vouched for by the pinned key is shown with both
/// fingerprints and re-pinned only on a terminal.
#[test]
fn reg_033_a_vouched_key_change_is_re_pinned_only_on_a_terminal() {
    let old = Lib::acme().widget();
    let root = pinned(&old);
    add(root.path(), "acme/acme-ui/widget", &fetcher(&[&old])).expect("first");
    let new_seed = [11; 32];
    let handover = signing::sign_handover(&SecretKey::from_bytes(SEED), &public_key(new_seed))
        .expect("handover");
    let rotated = Lib::acme().at("1.1.0").widget().signed_by(new_seed).set(
        "previousKeys",
        json!([{
            "publicKey": handover.from.encode(),
            "next": handover.to.as_str(),
            "signature": handover.signature.encode(),
        }]),
    );
    let both = fetcher(&[&old, &rotated]);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &both),
        "needs a terminal",
    );
    let project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/acme-ui/widget").expect("source"),
        &yes(),
        &both,
        &project,
        &TestScanner,
    )
    .expect("a vouched change resolves");
    let rendered = plan::render_with(&plan, &yes(), &project);
    assert!(
        rendered.contains(public_key(SEED).fingerprint().as_str())
            && rendered.contains(public_key(new_seed).fingerprint().as_str()),
        "{rendered}"
    );
    let mut prompter = terminal(&[true]);
    install_with(
        root.path(),
        "acme/acme-ui/widget",
        &both,
        &yes(),
        &TestScanner,
        &mut prompter,
    )
    .expect("re-pinned on the terminal");
    assert_eq!(
        ProjectFile::load(root.path())
            .expect("load")
            .pinned_key(&old.address())
            .expect("pin"),
        Some(public_key(new_seed))
    );
}

/// REG-033: a new key the pinned key does not vouch for is refused.
#[test]
fn reg_033_a_new_key_the_pinned_key_does_not_vouch_for_is_refused() {
    let old = Lib::acme().widget();
    let root = pinned(&old);
    let new_seed = [12; 32];
    let stranger = signing::sign_handover(&SecretKey::from_bytes([13; 32]), &public_key(new_seed))
        .expect("handover");
    let misnamed = signing::sign_handover(&SecretKey::from_bytes(SEED), &public_key([14; 32]))
        .expect("handover");
    for handover in [stranger, misnamed] {
        let rotated = Lib::acme().widget().signed_by(new_seed).set(
            "previousKeys",
            json!([{
                "publicKey": handover.from.encode(),
                "next": handover.to.as_str(),
                "signature": handover.signature.encode(),
            }]),
        );
        let mut prompter = terminal(&[true, true, true]);
        let result = install_with(
            root.path(),
            "acme/acme-ui/widget",
            &fetcher(&[&rotated]),
            &yes(),
            &TestScanner,
            &mut prompter,
        );
        assert!(result.is_err(), "an unvouched key was accepted");
        assert!(prompter.asked.is_empty(), "{:?}", prompter.asked);
    }
    assert!(!root.path().join("templates").exists());
}

// ===========================================================================
// REG-026: versions and tags
// ===========================================================================

/// REG-026: with no version, the highest release tag is taken, in semver
/// order, pre-releases excluded.
#[test]
fn reg_026_no_version_takes_the_highest_release() {
    let one = Lib::acme().at("1.9.0").widget();
    let ten = Lib::acme().at("1.10.0").widget();
    let pre = Lib::acme().at("2.0.0-rc.1").widget();
    let root = pinned(&one);
    let plan = add(
        root.path(),
        "acme/acme-ui/widget",
        &fetcher(&[&one, &ten, &pre]),
    )
    .expect("installs");
    assert_eq!(plan.components[0].version, v("1.10.0"));

    let tags = serde_json::to_vec(&json!([
        {"name": "v1.9.0", "commit": {"sha": "1".repeat(40)}},
        {"name": "v2.0.0-rc.1", "commit": {"sha": "2".repeat(40)}},
        {"name": "v1.10.0", "commit": {"sha": "3".repeat(40)}},
        {"name": "latest", "commit": {"sha": "4".repeat(40)}},
        {"name": "1.11.0", "commit": {"sha": "5".repeat(40)}},
    ]))
    .expect("tags");
    let gitlab_tags = serde_json::to_vec(&json!([
        {"name": "v0.3.0", "commit": {"id": "6".repeat(40)}},
        {"name": "v0.4.0-beta", "commit": {"id": "7".repeat(40)}},
    ]))
    .expect("tags");
    let server = serve(move |path| match path {
        "/api.github.com/repos/acme/acme-ui/tags?per_page=100&page=1" => Reply::Body(tags.clone()),
        "/codeberg.org/api/v1/repos/acme/acme-ui/tags?limit=50&page=1" => Reply::Body(tags.clone()),
        "/gitlab.com/api/v4/projects/acme%2Facme-ui/repository/tags?per_page=100&page=1" => {
            Reply::Body(gitlab_tags.clone())
        }
        _ => Reply::Status(404),
    });
    let client = client().with_forge_base(&server.base).expect("loopback");
    for host in ["github.com", "codeberg.org"] {
        let library = LibraryAddress(format!("{host}/acme/acme-ui"));
        assert_eq!(
            client.versions(&library).expect(host),
            vec![v("1.9.0"), v("1.10.0")]
        );
        assert_eq!(
            client.resolve(&library, &v("1.10.0")).expect(host),
            Commit("3".repeat(40))
        );
    }
    let gitlab = LibraryAddress("gitlab.com/acme/acme-ui".to_owned());
    assert_eq!(client.versions(&gitlab).expect("gitlab"), vec![v("0.3.0")]);
    assert_eq!(
        client.resolve(&gitlab, &v("0.3.0")).expect("gitlab"),
        Commit("6".repeat(40))
    );
}

/// REG-026: a repository with no release tag is refused, saying to tag a
/// release.
#[test]
fn reg_026_a_repository_with_no_release_is_refused_saying_to_tag_one() {
    let pre = Lib::acme().at("1.0.0-rc.1").widget();
    let root = pinned(&pre);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fetcher(&[&pre])),
        "tag a release",
    );
}

/// REG-026: a tag whose `library.json` says another version is refused.
#[test]
fn reg_026_a_tag_whose_library_json_names_another_version_is_refused() {
    let library = Lib::acme().widget();
    let mut fake = FakeFetcher::default();
    fake.add_version(library.address(), v("1.1.0"), library.tree());
    let root = pinned(&library);
    expect_refused(
        add(root.path(), "acme/acme-ui/widget", &fake),
        "says version 1.0.0",
    );
}

/// A fetcher that counts each tag resolution and records the commit of each
/// file it serves.
struct Counting<'a> {
    inner: &'a dyn Fetcher,
    resolved: RefCell<Vec<(LibraryAddress, semver::Version)>>,
    commits: RefCell<BTreeMap<LibraryAddress, BTreeSet<Commit>>>,
}

impl Fetcher for Counting<'_> {
    fn versions(&self, library: &LibraryAddress) -> Result<Vec<semver::Version>, RegistryError> {
        self.inner.versions(library)
    }

    fn resolve(
        &self,
        library: &LibraryAddress,
        version: &semver::Version,
    ) -> Result<Commit, RegistryError> {
        self.resolved
            .borrow_mut()
            .push((library.clone(), version.clone()));
        self.inner.resolve(library, version)
    }

    fn file(
        &self,
        library: &LibraryAddress,
        commit: &Commit,
        path: &str,
    ) -> Result<Vec<u8>, RegistryError> {
        self.commits
            .borrow_mut()
            .entry(library.clone())
            .or_default()
            .insert(commit.clone());
        self.inner.file(library, commit, path)
    }
}

/// REG-026: each tag a plan uses is resolved once, and every file of that
/// library at that version comes from that one commit.
#[test]
fn reg_026_each_tag_resolves_once_and_every_file_comes_from_its_commit() {
    let library = Lib::acme()
        .component("a", &[("a.html", "<p>A</p>\n")])
        .component("b", &[("b.html", "<p>B</p>\n")])
        .component("c", &[("c.html", "<p>C</p>\n")])
        .depends("a", &["./b", "./c"])
        .depends("b", &["./c"]);
    let fake = fetcher(&[&library]);
    let counting = Counting {
        inner: &fake,
        resolved: RefCell::default(),
        commits: RefCell::default(),
    };
    let root = pinned(&library);
    resolve(root.path(), "acme/acme-ui/a", &counting).expect("plan");
    assert_eq!(
        counting.resolved.borrow().as_slice(),
        &[(library.address(), v("1.0.0"))]
    );
    assert_eq!(counting.commits.borrow()[&library.address()].len(), 1);
}

/// REG-026: an older version does not replace a newer recorded one, and a
/// tag moved to different content at the recorded version is refused,
/// unless `--force` is given.
#[test]
fn reg_026_a_downgrade_or_a_moved_tag_is_refused_without_force() {
    let newer = Lib::acme().at("2.0.0").widget();
    let older = Lib::acme().at("1.0.0").widget();
    let root = pinned(&newer);
    add(
        root.path(),
        "acme/acme-ui/widget@2.0.0",
        &fetcher(&[&older, &newer]),
    )
    .expect("newer");
    expect_refused(
        add(
            root.path(),
            "acme/acme-ui/widget@1.0.0",
            &fetcher(&[&older, &newer]),
        ),
        "--force",
    );
    let moved = Lib::acme()
        .at("2.0.0")
        .component("widget", &[("widget.html", "<div>moved</div>\n")]);
    let error = expect_refused(
        add(
            root.path(),
            "acme/acme-ui/widget@2.0.0",
            &fetcher(&[&moved]),
        ),
        "changed a released version",
    );
    assert!(error.contains("--force"), "{error}");
    let forced = Options {
        force: true,
        yes: true,
        ..Options::default()
    };
    install_with(
        root.path(),
        "acme/acme-ui/widget@2.0.0",
        &fetcher(&[&moved]),
        &forced,
        &TestScanner,
        &mut no_terminal(),
    )
    .expect("forced");
    install_with(
        root.path(),
        "acme/acme-ui/widget@1.0.0",
        &fetcher(&[&older, &newer]),
        &forced,
        &TestScanner,
        &mut no_terminal(),
    )
    .expect("forced downgrade");
}

// ===========================================================================
// REG-027: offline verification in live:check
// ===========================================================================

fn installed_widget() -> (tempfile::TempDir, Lib) {
    let library = Lib::acme().widget();
    let root = pinned(&library);
    add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("installs");
    (root, library)
}

fn edit_record(root: &Path, from: &str, to: &str) {
    let path = root.join("suprnova.toml");
    let text = fs::read_to_string(&path).expect("read");
    assert!(text.contains(from), "`{from}` not in {text}");
    fs::write(&path, text.replacen(from, to, 1)).expect("write");
}

/// REG-027: a record whose `source`, version, a digest or the signature was
/// altered by hand fails verification; an intact one verifies.
#[test]
fn reg_027_a_record_altered_by_hand_fails_verification() {
    let (root, _) = installed_widget();
    let intact = verify_installed(root.path()).expect("verify");
    assert_eq!(intact.verified.len(), 1);
    assert!(
        intact.failures.is_empty() && intact.changed.is_empty(),
        "{intact:?}"
    );
    let record = records(root.path())[&component("github.com/acme/acme-ui/widget")].clone();
    let other_signature = signing::sign(&SecretKey::from_bytes(SEED), &Digest::of(b"other"))
        .expect("sign")
        .encode();
    for (from, to) in [
        (
            "source = \"github.com/acme/acme-ui\"".to_owned(),
            "source = \"github.com/evil/acme-ui\"".to_owned(),
        ),
        (
            "version = \"1.0.0\"".to_owned(),
            "version = \"1.0.1\"".to_owned(),
        ),
        (
            record.files["widget.html"].to_string(),
            Digest::of(b"other").to_string(),
        ),
        (record.signature.encode(), other_signature),
    ] {
        let (root, _) = installed_widget();
        edit_record(root.path(), &from, &to);
        let verification = verify_installed(root.path()).expect("verify");
        assert_eq!(
            verification.failures.len(),
            1,
            "{from} -> {to}: {verification:?}"
        );
        assert!(verification.verified.is_empty());
    }
}

/// REG-027: a recorded component with no pinned key fails.
#[test]
fn reg_027_a_missing_pin_fails_verification() {
    let (root, _) = installed_widget();
    let path = root.path().join("suprnova.toml");
    let text = fs::read_to_string(&path).expect("read");
    let without: String = text
        .lines()
        .filter(|line| !line.starts_with("key ="))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&path, without).expect("write");
    let verification = verify_installed(root.path()).expect("verify");
    assert_eq!(verification.failures.len(), 1, "{verification:?}");
    assert!(verification.failures[0].1.contains("no key is pinned"));
}

/// REG-027: a vendored file the application edited is reported as changed
/// since install and does not fail the check.
#[test]
fn reg_027_an_edited_vendored_file_is_reported_and_does_not_fail() {
    let (root, _) = installed_widget();
    fs::write(
        root.path().join("templates/acme-ui/widget/widget.css"),
        ".acme-widget { display: grid; }\n",
    )
    .expect("edit");
    let verification = verify_installed(root.path()).expect("verify");
    assert!(verification.failures.is_empty(), "{verification:?}");
    assert_eq!(
        verification.changed,
        vec![(
            component("github.com/acme/acme-ui/widget"),
            PathBuf::from("templates/acme-ui/widget/widget.css")
        )]
    );
}

/// REG-027: `live:check` fails on a tampered record before it builds the
/// application, and reports an edited file without failing on it.
#[test]
fn reg_027_live_check_verifies_the_records_before_it_builds_anything() {
    let (root, _) = installed_widget();
    edit_record(root.path(), "version = \"1.0.0\"", "version = \"1.0.1\"");
    let output = Command::new(BIN)
        .args(["live:check", "--timeout-secs", "1"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .output()
        .expect("run live:check");
    let text = combined(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("failed verification"), "{text}");
    assert!(!text.contains("Building and running"), "{text}");

    let (root, _) = installed_widget();
    fs::write(
        root.path().join("templates/acme-ui/widget/widget.html"),
        "<p>edited</p>\n",
    )
    .expect("edit");
    let output = Command::new(BIN)
        .args(["live:check", "--timeout-secs", "1"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .output()
        .expect("run live:check");
    let text = combined(&output);
    assert!(text.contains("widget.html changed since install"), "{text}");
    assert!(
        text.contains("Verified 1 recorded third-party component"),
        "{text}"
    );
    assert!(!text.contains("failed verification"), "{text}");
}

// ===========================================================================
// REG-028: the install record
// ===========================================================================

/// A component whose Rust defines the Live component `card::Card`; the
/// marker line is what the stand-in scan reads as its `#[live]` type.
fn rust_library(version: &str, rust: &str, view: &str) -> Lib {
    let rust = format!("// live: Card\n{rust}");
    Lib::acme()
        .at(version)
        .component("card", &[("card.html", view), ("card.rs", &rust)])
        .manifest("card", "register", json!(["card::Card"]))
}

fn install_card(root: &Path, libraries: &[&Lib], options: &Options) -> Result<Plan, RegistryError> {
    install_with(
        root,
        "acme/acme-ui/card",
        &fetcher(libraries),
        options,
        &MarkerScanner,
        &mut no_terminal(),
    )
}

/// REG-028: the record sits in the component's view directory and is keyed
/// by the path from the project root, Rust files included.
#[test]
fn reg_028_the_install_record_is_keyed_by_project_path_rust_included() {
    let library = rust_library("1.0.0", "pub struct Card;\n", "<div>Card</div>\n");
    let root = pinned(&library);
    install_card(root.path(), &[&library], &yes()).expect("installs");
    let raw: BTreeMap<String, String> = serde_json::from_slice(
        &fs::read(
            root.path()
                .join("templates/acme-ui/card/.suprnova-installed.json"),
        )
        .expect("record"),
    )
    .expect("json");
    let keys: Vec<&str> = raw.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "src/live/acme/card.rs",
            "templates/acme-ui/card/card.html",
            "templates/acme-ui/card/manifest.json"
        ]
    );
    assert_eq!(
        raw["src/live/acme/card.rs"],
        Digest::of(b"// live: Card\npub struct Card;\n").to_string()
    );
}

/// REG-028: an edited file, Rust included, is kept without `--force`; an
/// unedited one follows the library; a kept file keeps its digest so a
/// second install keeps it again; `--force` replaces it.
#[test]
fn reg_028_edited_files_are_kept_unedited_ones_replaced_and_force_replaces() {
    let one = rust_library("1.0.0", "pub struct Card;\n", "<div>Card</div>\n");
    let two = rust_library(
        "1.1.0",
        "pub struct Card; // 1.1\n",
        "<div>Card 1.1</div>\n",
    );
    let three = rust_library(
        "1.2.0",
        "pub struct Card; // 1.2\n",
        "<div>Card 1.2</div>\n",
    );
    let root = pinned(&one);
    install_card(root.path(), &[&one], &yes()).expect("one");
    let rust = root.path().join("src/live/acme/card.rs");
    let view = root.path().join("templates/acme-ui/card/card.html");
    fs::write(&rust, "pub struct Card; // edited\n").expect("edit the Rust");

    let plan = install_card(root.path(), &[&one, &two], &yes()).expect("two");
    let outcomes: BTreeMap<String, FileOutcome> = plan.components[0]
        .files
        .iter()
        .map(|file| (file.name.clone(), file.outcome))
        .collect();
    assert_eq!(outcomes["card.rs"], FileOutcome::Kept);
    assert_eq!(outcomes["card.html"], FileOutcome::Replaced);
    assert_eq!(
        fs::read_to_string(&rust).expect("rust"),
        "pub struct Card; // edited\n"
    );
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        "<div>Card 1.1</div>\n"
    );
    let record =
        InstallRecord::load(root.path(), Path::new("templates/acme-ui/card")).expect("record");
    assert_eq!(
        record.digest(Path::new("src/live/acme/card.rs")),
        Some(&Digest::of(b"// live: Card\npub struct Card;\n")),
        "a kept file keeps its previous digest"
    );

    install_card(root.path(), &[&one, &two, &three], &yes()).expect("three");
    assert_eq!(
        fs::read_to_string(&rust).expect("rust"),
        "pub struct Card; // edited\n"
    );

    let forced = Options {
        force: true,
        yes: true,
        ..Options::default()
    };
    install_card(root.path(), &[&one, &two, &three], &forced).expect("forced");
    assert_eq!(
        fs::read_to_string(&rust).expect("rust"),
        "// live: Card\npub struct Card; // 1.2\n"
    );
}

/// REG-028: a record keyed by bare file names, as `live:add` wrote them
/// before, is read as files of its directory and rewritten in the new form.
#[test]
fn reg_028_a_record_in_the_old_form_is_read_and_rewritten() {
    let root = project();
    let first = live_add(root.path(), &["field"]);
    assert!(first.status.success(), "{}", combined(&first));
    let directory = root.path().join("templates/suprnova-ui/field");
    let view = directory.join("field.html");
    let shipped = fs::read_to_string(&view).expect("view");
    let older = "{# an older field #}\n";
    fs::write(&view, older).expect("older view");
    let hex = |bytes: &[u8]| {
        Digest::of(bytes)
            .as_str()
            .trim_start_matches("sha256:")
            .to_owned()
    };
    let mut old_form = BTreeMap::new();
    for entry in fs::read_dir(&directory).expect("dir") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name")
            .to_owned();
        if name.starts_with('.') {
            continue;
        }
        old_form.insert(name, hex(&fs::read(&path).expect("read")));
    }
    old_form.insert("field.html".to_owned(), hex(older.as_bytes()));
    fs::write(
        directory.join(".suprnova-installed.json"),
        serde_json::to_vec(&old_form).expect("encode"),
    )
    .expect("old record");

    let second = live_add(root.path(), &["field"]);
    assert!(second.status.success(), "{}", combined(&second));
    assert_eq!(
        fs::read_to_string(&view).expect("view"),
        shipped,
        "the old record vouched for the older view, so it is replaced"
    );
    let rewritten: BTreeMap<String, String> = serde_json::from_slice(
        &fs::read(directory.join(".suprnova-installed.json")).expect("record"),
    )
    .expect("json");
    assert!(
        rewritten
            .keys()
            .all(|key| key.starts_with("templates/suprnova-ui/field/")),
        "{rewritten:?}"
    );
    assert!(
        rewritten
            .values()
            .all(|digest| digest.starts_with("sha256:"))
    );
}

// ===========================================================================
// REG-029: atomic installs, the lock and the journal
// ===========================================================================

/// REG-029: a dependency that fails validation leaves neither its own nor
/// its parent's files written.
#[test]
fn reg_029_a_dependency_that_fails_validation_writes_nothing() {
    let mut library = Lib::acme()
        .component("parent", &[("parent.html", "<p>P</p>\n")])
        .component("child", &[("child.html", "<p>C</p>\n")])
        .depends("parent", &["./child"]);
    library
        .components
        .get_mut("child")
        .expect("child")
        .signature = Some(None);
    let root = pinned(&library);
    let before = listing(root.path());
    expect_refused(
        add(root.path(), "acme/acme-ui/parent", &fetcher(&[&library])),
        "unsigned",
    );
    let after: BTreeSet<PathBuf> = listing(root.path())
        .into_iter()
        .filter(|path| path != Path::new(project::LOCK_FILE))
        .collect();
    assert_eq!(after, before);

    let refused = Lib::acme()
        .component("parent", &[("parent.html", "<p>P</p>\n")])
        .component(
            "child",
            &[("child.html", "<p>C</p>\n"), ("child.rs", "// refuse\n")],
        )
        .depends("parent", &["./child"]);
    let error = install_with(
        root.path(),
        "acme/acme-ui/parent",
        &fetcher(&[&refused]),
        &yes(),
        &MarkerScanner,
        &mut no_terminal(),
    )
    .expect_err("refused");
    assert!(
        error.to_string().contains("child.rs:1: [rust-path]"),
        "{error}"
    );
    assert!(!root.path().join("templates").exists());
}

/// REG-029: when a step after the first write fails, every changed file,
/// `suprnova.toml` included, gets its prior bytes back, every created file
/// and directory is removed, and the journal is gone.
#[test]
fn reg_029_a_failed_write_restores_every_file_suprnova_toml_included() {
    let library = rust_library("1.0.0", "pub struct Card;\n", "<div>Card</div>\n");
    let root = pinned(&library);
    fs::create_dir_all(root.path().join("src/live")).expect("src/live");
    fs::write(
        root.path().join("src/live/mod.rs"),
        "// the application's registry\n",
    )
    .expect("mod.rs");
    let project_file = fs::read(root.path().join("suprnova.toml")).expect("project file");
    let before = listing(root.path());
    let calls = RefCell::new(0);
    let failing = |root: &Path,
                   module: &str,
                   modules: &[String],
                   register: &[String],
                   unregister: &[String]|
     -> Result<RegistrationEdits, RegistryError> {
        *calls.borrow_mut() += 1;
        if *calls.borrow() > 1 {
            return Err(RegistryError::Io("the disk filled up".to_owned()));
        }
        registrar(root, module, modules, register, unregister)
    };
    let lock = ProjectLock::acquire(root.path()).expect("lock");
    let mut project = ProjectFile::load(root.path()).expect("load");
    let source = address::parse("acme/acme-ui/card").expect("source");
    let plan = plan::resolve_with(
        &source,
        &yes(),
        &fetcher(&[&library]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    let decisions = plan::confirm(&plan, &yes(), &project, &mut no_terminal()).expect("decisions");
    let error = install::apply_with(&plan, &mut project, &yes(), &decisions, &failing)
        .expect_err("the second registration call fails");
    drop(lock);
    assert!(error.to_string().contains("the disk filled up"), "{error}");
    assert_eq!(
        fs::read(root.path().join("suprnova.toml")).expect("project file"),
        project_file
    );
    assert_eq!(
        fs::read_to_string(root.path().join("src/live/mod.rs")).expect("mod.rs"),
        "// the application's registry\n"
    );
    let after: BTreeSet<PathBuf> = listing(root.path())
        .into_iter()
        .filter(|path| path != Path::new(project::LOCK_FILE))
        .collect();
    assert_eq!(after, before, "files were left behind");
    assert!(!Journal::exists(root.path()));
}

/// REG-029: a builder in a form the registration writer does not recognize
/// gets the lines reported and nothing written for the whole install.
#[test]
fn reg_029_an_unrecognized_builder_reports_its_lines_and_writes_nothing() {
    let library = rust_library("1.0.0", "pub struct Card;\n", "<div>Card</div>\n");
    let root = pinned(&library);
    let before = listing(root.path());
    let reporting = |_: &Path,
                     _: &str,
                     _: &[String],
                     register: &[String],
                     _: &[String]|
     -> Result<RegistrationEdits, RegistryError> {
        Ok(RegistrationEdits::Report(
            register
                .iter()
                .map(|path| format!(".register::<{path}>()"))
                .collect(),
        ))
    };
    let mut project = ProjectFile::load(root.path()).expect("load");
    let plan = plan::resolve_with(
        &address::parse("acme/acme-ui/card").expect("source"),
        &yes(),
        &fetcher(&[&library]),
        &project,
        &MarkerScanner,
    )
    .expect("plan");
    let decisions = plan::confirm(&plan, &yes(), &project, &mut no_terminal()).expect("decisions");
    let error = install::apply_with(&plan, &mut project, &yes(), &decisions, &reporting)
        .expect_err("reported");
    assert!(
        error
            .to_string()
            .contains(".register::<crate::live::acme::card::Card>()"),
        "{error}"
    );
    assert_eq!(listing(root.path()), before);
}

/// REG-029: a second install is refused while another holds the project
/// lock, before it reads the records.
#[test]
fn reg_029_a_second_install_is_refused_while_the_lock_is_held() {
    let root = project();
    let lock = ProjectLock::acquire(root.path()).expect("lock");
    assert!(ProjectLock::is_held(root.path()) || cfg!(windows));
    let error = ProjectLock::acquire(root.path()).expect_err("held");
    assert!(error.to_string().contains("another live:add"), "{error}");
    let output = live_add(root.path(), &["field"]);
    assert!(!output.status.success(), "{}", combined(&output));
    assert!(
        combined(&output).contains("another live:add"),
        "{}",
        combined(&output)
    );
    assert!(!root.path().join("templates").exists());
    lock.release().expect("release");
    let output = live_add(root.path(), &["field"]);
    assert!(output.status.success(), "{}", combined(&output));
}

/// A journal as an install killed after its first write leaves it: one
/// created file in a created directory, and `suprnova.toml` changed.
fn leave_a_journal(root: &Path) {
    fs::write(root.join("suprnova.toml"), "# before the install\n").expect("prior");
    let journal = Journal {
        created: vec![
            PathBuf::from("templates/acme-ui"),
            PathBuf::from("templates/acme-ui/stray.html"),
        ],
        changed: BTreeMap::from([(
            PathBuf::from("suprnova.toml"),
            b"# before the install\n".to_vec(),
        )]),
    };
    journal.write(root).expect("journal");
    fs::create_dir_all(root.join("templates/acme-ui")).expect("mkdir");
    fs::write(root.join("templates/acme-ui/stray.html"), "<p>half</p>\n").expect("stray");
    fs::write(root.join("suprnova.toml"), "# half written\n").expect("half");
}

/// REG-029: the next `live:add` finds the journal a killed install left
/// with no lock held, restores it before anything else, and says so.
#[test]
fn reg_029_the_next_live_add_restores_a_journal_left_by_a_killed_install() {
    let root = project();
    leave_a_journal(root.path());
    let dry = live_add(root.path(), &["field", "--dry-run"]);
    assert!(!dry.status.success(), "{}", combined(&dry));
    assert!(
        Journal::exists(root.path()),
        "a dry run restored the journal"
    );
    let output = live_add(root.path(), &["field"]);
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("restored"), "{text}");
    assert!(!root.path().join("templates/acme-ui").exists());
    assert!(!Journal::exists(root.path()));
    let project_file = fs::read_to_string(root.path().join("suprnova.toml")).expect("read");
    assert!(
        project_file.starts_with("# before the install\n"),
        "{project_file}"
    );
}

/// REG-029: `serve` restores a journal it finds with no lock held before
/// anything else, and reports it.
#[test]
fn reg_029_serve_restores_a_journal_it_finds_with_no_lock_held() {
    let root = project();
    leave_a_journal(root.path());
    // A broken dev process stops serve right after the restore, before it
    // would start any process.
    let journal = Journal::read(root.path()).expect("read").expect("journal");
    let mut changed = journal.changed.clone();
    changed.insert(
        PathBuf::from("suprnova.toml"),
        b"[[serve.process]]\ncommand = \"true\"\n".to_vec(),
    );
    Journal {
        created: journal.created,
        changed,
    }
    .write(root.path())
    .expect("journal");
    let output = Command::new(BIN)
        .args(["serve", "--backend-only"])
        .current_dir(root.path())
        .env("PATH", "/nonexistent")
        .output()
        .expect("run serve");
    let text = combined(&output);
    assert!(text.contains("restored"), "{text}");
    assert!(!root.path().join("templates/acme-ui").exists());
    assert!(!Journal::exists(root.path()));
    assert!(text.contains("missing a non-empty `name`"), "{text}");
}

/// REG-029: the command `serve` runs before each build waits while an
/// install holds the lock and returns once it is released, so no build
/// starts during an install and one starts after it.
#[test]
fn reg_029_serve_builds_only_after_the_lock_is_released() {
    let root = project();
    let lock = ProjectLock::acquire(root.path()).expect("lock");
    let mut wait = Command::new(BIN)
        .arg("live:wait")
        .current_dir(root.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn live:wait");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        wait.try_wait().expect("poll").is_none(),
        "live:wait returned while the lock was held"
    );
    lock.release().expect("release");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = wait.try_wait().expect("poll") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "live:wait did not return after the release"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
}

/// REG-029: a write the install makes leaves the record and the files in
/// one consistent state for a second install to plan against.
#[test]
fn reg_029_a_second_install_plans_against_the_first_installs_records() {
    let library = Lib::acme().widget();
    let root = pinned(&library);
    add(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("first");
    let plan = resolve(root.path(), "acme/acme-ui/widget", &fetcher(&[&library])).expect("again");
    assert!(
        plan.components[0]
            .files
            .iter()
            .all(|file| file.outcome == FileOutcome::Unchanged)
    );
}
