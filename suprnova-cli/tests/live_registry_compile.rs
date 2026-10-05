//! `registries-compile`: a library the SDK scaffolds passes its own checks
//! and tests, installs into a scaffolded application, compiles and renders
//! through its Live route, and the manual's commands produce a signed
//! library (REG-005, REG-018, REG-021). Each test scaffolds and compiles a
//! project, so the suite is ignored by default and the gate runs it with
//! `-- --ignored`.
//!
//! Every project these tests compile shares one target directory under this
//! package's `CARGO_TARGET_TMPDIR`, so the framework is compiled once for
//! the preview and the application alike, and stays compiled between runs.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// The markers `scaffold_snapshot.rs` refuses in scaffolder output.
const FORBIDDEN_MARKERS: [&str; 4] = ["TODO", "FIXME", "unimplemented!", "panic!("];

/// The registration line `live:add` writes for the example (REG-005).
const REGISTRATION: &str = ".register::<crate::live::acme::counter::Counter>()?";

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path.as_ref())
        .unwrap_or_else(|error| panic!("read {}: {error}", path.as_ref().display()))
}

/// A scratch directory under this package's own target directory: a
/// generated application builds far too many files for a tmpfs `/tmp`.
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir")
}

/// The target directory every compiled project shares.
fn shared_target() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("registry-compile-target")
}

/// An author's machine: a home and configuration directory of its own, so
/// the key `live:registry new` writes lands in the test, never in the real
/// configuration directory.
struct Author {
    home: PathBuf,
}

impl Author {
    fn new(scratch: &Path) -> Self {
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).expect("home");
        Author { home }
    }

    fn config(&self) -> PathBuf {
        self.home.join(".config")
    }

    fn suprnova(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.config())
            .env_remove("SUPRNOVA_LIBRARY_KEY")
            .output()
            .expect("run suprnova")
    }

    fn succeed(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self.suprnova(cwd, args);
        let text = combined(&output);
        assert!(
            output.status.success(),
            "`suprnova {}` failed:\n{text}",
            args.join(" ")
        );
        text
    }

    /// `live:registry new acme` in `cwd`, returning the library root.
    fn new_library(&self, cwd: &Path) -> PathBuf {
        let text = self.succeed(cwd, &["live:registry", "new", "acme"]);
        assert!(
            text.contains("library-keys"),
            "names the key's place:\n{text}"
        );
        assert!(text.contains("strands every pin"), "{text}");
        cwd.join("acme")
    }
}

/// Points every `suprnova` dependency of a scaffolded manifest at the
/// in-tree framework and detaches it from the surrounding workspace, as
/// `scaffold_snapshot.rs` does.
fn patch_local_suprnova(project: &Path) {
    let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("framework");
    let cargo_toml = project.join("Cargo.toml");
    let original = read(&cargo_toml);
    let mut rewritten = String::with_capacity(original.len() + 64);
    let mut replaced = false;
    let mut has_workspace = false;
    for line in original.lines() {
        if line.trim() == "[workspace]" {
            has_workspace = true;
        }
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                framework.display()
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "{} declares no suprnova dependency",
        cargo_toml.display()
    );
    if !has_workspace {
        rewritten.push_str("\n[workspace]\n");
    }
    std::fs::write(&cargo_toml, rewritten).expect("write the patched Cargo.toml");
}

fn cargo(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO"))
        .args(args)
        .current_dir(project)
        .env("CARGO_TARGET_DIR", shared_target())
        .output()
        .expect("run cargo")
}

fn cargo_succeeds(project: &Path, args: &[&str]) {
    let output = cargo(project, args);
    assert!(
        output.status.success(),
        "`cargo {}` in {} failed:\n{}",
        args.join(" "),
        project.display(),
        combined(&output)
    );
}

fn assert_no_markers(root: &Path) {
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.expect("walk");
        let path = entry.path().to_string_lossy();
        if !entry.file_type().is_file() || path.contains("/target/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for marker in FORBIDDEN_MARKERS {
            assert!(!text.contains(marker), "{path} holds {marker}");
        }
    }
}

/// The capabilities `live:registry check` lists for a component, as
/// `live:add --allow` takes them.
fn capabilities(check_output: &str, component: &str) -> Vec<String> {
    let prefix = format!("{component}: capabilities: ");
    let line = check_output
        .lines()
        .find_map(|line| line.split_once(&prefix).map(|(_, rest)| rest.trim()))
        .unwrap_or_else(|| panic!("check lists no capabilities for {component}:\n{check_output}"));
    if line == "none" {
        Vec::new()
    } else {
        line.split(", ").map(str::to_owned).collect()
    }
}

/// A server binary started on a free port, stopped when the test ends.
struct Server {
    child: Child,
    port: u16,
    log: PathBuf,
}

impl Server {
    fn start(binary: &Path, cwd: &Path, scratch: &Path) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("a free port")
            .port();
        let log = scratch.join(format!("server-{port}.log"));
        let database = scratch.join(format!("server-{port}.db"));
        let child = Command::new(binary)
            .current_dir(cwd)
            .env("SERVER_HOST", "127.0.0.1")
            .env("SERVER_PORT", port.to_string())
            .env(
                "DATABASE_URL",
                format!("sqlite://{}?mode=rwc", database.display()),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).expect("the server log"))
            .spawn()
            .expect("start the application");
        let server = Server { child, port, log };
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut ready = false;
        while !ready && Instant::now() < deadline {
            ready = TcpStream::connect(("127.0.0.1", port)).is_ok();
            if !ready {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        assert!(ready, "the application did not start:\n{}", server.log());
        server
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn get(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("a read timeout");
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .expect("send the request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read the response");
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// REG-018: `live:registry new <namespace>` scaffolds the library tree with
/// one example component and a preview application, writes the public key
/// into `library.json`, keeps the private key outside the project, holds no
/// stub marker, and passes `live:registry check`; `sign` signs it again to
/// the same bytes and refuses a key file inside the project.
#[test]
#[ignore = "scaffolds a project; the gate runs it with -- --ignored"]
fn reg_018_registry_new_scaffolds_the_tree_with_an_example_and_a_key_outside_the_project() {
    let tmp = scratch();
    let author = Author::new(tmp.path());
    let library = author.new_library(tmp.path());
    assert!(library.join("library.json").is_file());
    for file in [
        "manifest.json",
        "manifest.sig",
        "counter.html",
        "counter.css",
        "counter.js",
        "counter.rs",
    ] {
        assert!(
            library.join("components/counter").join(file).is_file(),
            "components/counter/{file}"
        );
    }
    assert!(library.join("preview/Cargo.toml").is_file());
    let secrets: Vec<_> = walkdir::WalkDir::new(&library)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| read_lossy(entry.path()).contains("suprnova-library-key/1"))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    assert!(
        secrets.is_empty(),
        "a key file sits inside the project: {secrets:?}"
    );
    let keys: Vec<PathBuf> = std::fs::read_dir(author.config().join("suprnova/library-keys"))
        .expect("the key directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    assert_eq!(keys.len(), 1, "one key file in the configuration directory");
    assert_no_markers(&library);

    let check = author.succeed(&library, &["live:registry", "check"]);
    assert!(check.contains("counter: capabilities: "), "{check}");

    let signature = std::fs::read(library.join("components/counter/manifest.sig")).expect("sig");
    author.succeed(&library, &["live:registry", "sign"]);
    assert_eq!(
        std::fs::read(library.join("components/counter/manifest.sig")).expect("sig"),
        signature,
        "the same tree and key sign to the same bytes"
    );

    let inside = library.join("signing.key");
    std::fs::copy(&keys[0], &inside).expect("copy the key into the project");
    let refused = Command::new(BIN)
        .args(["live:registry", "sign"])
        .current_dir(&library)
        .env("HOME", &author.home)
        .env("XDG_CONFIG_HOME", author.config())
        .env("SUPRNOVA_LIBRARY_KEY", &inside)
        .output()
        .expect("run sign");
    assert!(!refused.status.success(), "{}", combined(&refused));
    assert!(
        combined(&refused).contains("inside the library"),
        "{}",
        combined(&refused)
    );
}

fn read_lossy(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned()
}

/// REG-018: as generated, the preview application compiles against the
/// framework, passes its own test, which renders the example through its
/// page, and passes `live:check`.
#[test]
#[ignore = "compiles the preview application; the gate runs it with -- --ignored"]
fn reg_018_the_preview_passes_its_own_test_and_live_check() {
    let tmp = scratch();
    let author = Author::new(tmp.path());
    let library = author.new_library(tmp.path());
    let preview = library.join("preview");
    patch_local_suprnova(&preview);
    cargo_succeeds(&preview, &["test"]);

    let database = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("preview-check.sqlite").display()
    );
    let output = Command::new(BIN)
        .args(["live:check", "--timeout-secs", "2400"])
        .current_dir(&preview)
        .env("DATABASE_URL", &database)
        .env("APP_ENV", "testing")
        .env("CARGO_TARGET_DIR", shared_target())
        .output()
        .expect("run live:check");
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("1 component"), "{text}");
}

/// The page the consumer adds to render the installed counter: one public
/// seed island, the namespace's asset route, and its stylesheet and script.
const COUNTER_PAGE: &str = r#"//! The installed counter, rendered by its own island.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

use crate::live::acme::counter::Counter;

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "counter_page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

/// Serves the acme library's stylesheets and scripts and the counter page.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let router = router.try_live_ui_assets_for("acme")?;
    let mount = LiveMount::<Counter>::public_seed("/counter", "counter", "acme-counter")?;
    let handler = mount.clone();
    let router: Router = router
        .get("/counter", move |request: Request| {
            let mount = handler.clone();
            async move { render(request, &mount).await }
        })
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("counter_page.html")
                    .map_err(|_| FrameworkError::internal("view name"))?,
                &Page {
                    bootstrap: bootstrap.html(),
                    counter: counter.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("response intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("counter page: {error}")).status(500))
}
"#;

const COUNTER_PAGE_VIEW: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Counter</title>
<link rel="stylesheet" href="/acme-ui/counter/counter.css">
<script type="module" src="/acme-ui/counter/counter.js"></script>
{{ bootstrap|trusted_html }}
</head>
<body>
<main>
{{ counter|trusted_html }}
</main>
</body>
</html>
"#;

/// Adds the counter page to a scaffolded application, as the consumer's
/// manual chapter does.
fn add_counter_page(app: &Path) {
    std::fs::write(app.join("src/counter_page.rs"), COUNTER_PAGE).expect("page module");
    std::fs::create_dir_all(app.join("templates")).expect("templates");
    std::fs::write(app.join("templates/counter_page.html"), COUNTER_PAGE_VIEW).expect("view");
    let lib = read(app.join("src/lib.rs"));
    std::fs::write(
        app.join("src/lib.rs"),
        lib.replacen(
            "pub mod routes;",
            "pub mod counter_page;\npub mod routes;",
            1,
        ),
    )
    .expect("lib.rs");
    let live = read(app.join("src/live/mod.rs"));
    let needle = "RenderCache::install(routes(router)?, RenderCacheConfig::from_env()?)";
    assert!(live.contains(needle), "{live}");
    std::fs::write(
        app.join("src/live/mod.rs"),
        live.replacen(
            needle,
            "RenderCache::install(\n        crate::counter_page::routes(routes(router)?)?,\n        RenderCacheConfig::from_env()?,\n    )",
            1,
        ),
    )
    .expect("live module");
}

/// REG-005 and REG-021: the manual's commands, in order. The author runs
/// `live:registry new`, `check` and `sign`; the consumer pins the library's
/// key, installs the example from the path source with `--yes` and every
/// capability the check listed allowed, gets the registration line at its
/// full path, compiles, starts the application, and the counter renders
/// through its own island with its stylesheet served. Installing again adds
/// no second declaration or registration.
#[test]
#[ignore = "scaffolds and compiles an application; the gate runs it with -- --ignored"]
fn reg_005_reg_021_a_signed_library_installs_compiles_and_renders_in_a_scaffolded_application() {
    let tmp = scratch();
    let author = Author::new(tmp.path());
    let library = author.new_library(tmp.path());
    let check = author.succeed(&library, &["live:registry", "check"]);
    author.succeed(&library, &["live:registry", "sign"]);
    let allowed = capabilities(&check, "counter");

    author.succeed(
        tmp.path(),
        &[
            "new",
            "shop",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ],
    );
    let app = tmp.path().join("shop");
    patch_local_suprnova(&app);
    cargo_succeeds(&app, &["generate-lockfile"]);

    // Trust on first use needs a terminal; a developer may pin the key by
    // hand beforehand (REG-024), which is what a non-interactive run does.
    let library_json: serde_json::Value =
        serde_json::from_str(&read(library.join("library.json"))).expect("library.json");
    let public_key = library_json["publicKey"].as_str().expect("publicKey");
    let library_address = std::fs::canonicalize(&library).expect("library root");
    std::fs::write(
        app.join("suprnova.toml"),
        format!(
            "# Pinned by hand before the first install.\n[live.libraries.\"{}\"]\nkey = \"{public_key}\"\n",
            library_address.display()
        ),
    )
    .expect("suprnova.toml");

    let mut add = vec!["live:add", "../acme/components/counter", "--yes"];
    for capability in &allowed {
        add.push("--allow");
        add.push(capability);
    }
    author.succeed(&app, &add);

    let live = read(app.join("src/live/mod.rs"));
    assert_eq!(live.matches(REGISTRATION).count(), 1, "{live}");
    assert_eq!(live.matches("pub mod acme;").count(), 1, "{live}");
    let namespace = read(app.join("src/live/acme/mod.rs"));
    assert_eq!(
        namespace.matches("pub mod counter;").count(),
        1,
        "{namespace}"
    );
    assert!(app.join("src/live/acme/counter.rs").is_file());
    for file in ["counter.html", "counter.css", "counter.js"] {
        assert!(
            app.join("templates/acme-ui/counter").join(file).is_file(),
            "{file}"
        );
    }
    assert!(
        read(app.join("suprnova.toml")).contains("crate::live::acme::counter::Counter"),
        "the record names the registration it wrote"
    );

    add_counter_page(&app);
    cargo_succeeds(&app, &["build", "--bin", "shop"]);
    let server = Server::start(&shared_target().join("debug/shop"), &app, tmp.path());
    let page = server.get("/counter");
    assert!(page.starts_with("HTTP/1.1 200"), "{page}\n{}", server.log());
    assert!(
        page.contains("data-suprnova-live-document-key=\"acme-counter\""),
        "{page}"
    );
    assert!(
        page.contains("class=\"acme-counter-value\">0</output>"),
        "{page}"
    );
    assert!(page.contains(">Add one</button>"), "{page}");
    let stylesheet = server.get("/acme-ui/counter/counter.css");
    assert!(stylesheet.starts_with("HTTP/1.1 200"), "{stylesheet}");
    drop(server);

    author.succeed(&app, &add);
    let live = read(app.join("src/live/mod.rs"));
    assert_eq!(live.matches(REGISTRATION).count(), 1, "{live}");
    assert_eq!(live.matches("pub mod acme;").count(), 1, "{live}");
    assert_eq!(
        read(app.join("src/live/acme/mod.rs"))
            .matches("pub mod counter;")
            .count(),
        1
    );
}
