//! What every kit shares: the signing key `suprnova new` writes into
//! `.env`, `src/agents.rs` and the entry that serves it, both plugins'
//! migrations and the case-insensitive email index, the return address a
//! sign-in link carries, and the email rules at sign-in, registration and
//! password reset.
//!
//! These read what `suprnova new` writes; none compiles a scaffold. One
//! builds the framework's agent auth from a written `.env`, through the
//! framework this crate's tests link.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

/// The CLI binary under test.
const CLI: &str = env!("CARGO_BIN_EXE_suprnova");

/// The kits that share the backend `suprnova new` writes: the three Inertia
/// kits and Live. The API kit writes its own.
pub(crate) const SHARED_BACKEND_KITS: [&str; 4] = ["svelte", "react", "vue", "live"];

/// The three Inertia kits.
pub(crate) const INERTIA_KITS: [&str; 3] = ["svelte", "react", "vue"];

/// A project `suprnova new` scaffolded into its own temporary directory,
/// with everything the command printed.
pub(crate) struct Project {
    pub(crate) kit: &'static str,
    pub(crate) root: PathBuf,
    pub(crate) printed: String,
    _parent: TempDir,
}

impl Project {
    /// Scaffold `kit`. `api` scaffolds the API kit with `--api`.
    pub(crate) fn new(kit: &'static str) -> Self {
        let parent = TempDir::new().expect("a temporary directory");
        let mut command = Command::new(CLI);
        command.args(["new", "agents-app", "--no-interaction", "--no-git"]);
        if kit == "api" {
            command.arg("--api");
        } else {
            command.args(["--frontend", kit]);
        }
        let output = command
            .current_dir(parent.path())
            .output()
            .expect("run `suprnova new`");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "`suprnova new` for {kit} failed:\n{printed}"
        );
        Self {
            kit,
            root: parent.path().join("agents-app"),
            printed,
            _parent: parent,
        }
    }

    /// The scaffolded file at `relative`.
    pub(crate) fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative))
            .unwrap_or_else(|error| panic!("{}: read {relative}: {error}", self.kit))
    }
}

/// The text of the item `name` (`fn name(`, `struct name`) in `source`:
/// from its first line to the next line that closes an item at column zero.
pub(crate) fn item<'a>(source: &'a str, name: &str) -> &'a str {
    let start = [format!("fn {name}("), format!("struct {name} ")]
        .iter()
        .find_map(|needle| source.find(needle.as_str()))
        .unwrap_or_else(|| panic!("no `{name}` in:\n{source}"));
    let rest = &source[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |end| end + 3);
    &rest[..end]
}

/// Whether `haystack` holds `needle`, whatever whitespace either has, so a
/// line rustfmt wraps still matches.
pub(crate) fn contains(haystack: &str, needle: &str) -> bool {
    let squash = |text: &str| text.split_whitespace().collect::<String>();
    squash(haystack).contains(&squash(needle))
}

/// The routes between the guest group's end and the authenticated group's
/// end in a scaffolded `src/routes.rs`: the authenticated group.
fn authenticated_group(routes: &str) -> &str {
    let guest_end = routes
        .find(".middleware(middleware::authenticate::guest())")
        .expect("routes.rs has a guest group");
    let auth_end = routes
        .find(".middleware(middleware::authenticate::auth())")
        .expect("routes.rs has an authenticated group");
    &routes[guest_end..auth_end]
}

/// The signing key a scaffolded `.env` names: the key identifier and the
/// quoted one-line PEM as written. A failed assertion names the line, never
/// the key.
struct WrittenKey {
    kid: String,
    quoted_pem: String,
}

impl WrittenKey {
    fn of(project: &Project) -> Self {
        let env = project.read(".env");
        let values = |name: &str| -> Vec<String> {
            env.lines()
                .filter_map(|line| line.strip_prefix(&format!("{name}=")))
                .map(str::to_owned)
                .collect()
        };
        let kids = values("AGENT_AUTH_KEY_ID");
        let pems = values("AGENT_AUTH_PRIVATE_KEY");
        assert_eq!(
            kids.len(),
            1,
            "{}: .env must set AGENT_AUTH_KEY_ID exactly once",
            project.kit
        );
        assert_eq!(
            pems.len(),
            1,
            "{}: .env must set AGENT_AUTH_PRIVATE_KEY exactly once",
            project.kit
        );
        Self {
            kid: kids[0].clone(),
            quoted_pem: pems[0].clone(),
        }
    }

    /// The PEM's base64 lines, each a run of key material.
    fn body_lines(&self) -> Vec<&str> {
        self.quoted_pem
            .trim_matches('"')
            .split("\\n")
            .filter(|line| !line.is_empty() && !line.starts_with("-----"))
            .collect()
    }
}

/// Build the framework's agent auth with the key `kid` and `pem`, on the
/// OAuth server it needs, as `Router::try_agents` builds it at boot. A
/// key the framework does not accept fails the build.
async fn build_agent_auth(kid: &str, pem: &str) -> Result<(), String> {
    use suprnova::agents::{AgentAuth, AgentsConfig, OAuthServer};

    // The OAuth server's issuer is the application's URL, which an
    // application outside the local and testing environments must name, on
    // HTTPS or a loopback host.
    if !suprnova::Config::has::<suprnova::AppConfig>() {
        suprnova::Config::register(
            suprnova::AppConfig::builder()
                .url("http://localhost")
                .build(),
        );
    }
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .map_err(|error| error.to_string())?;
    OAuthServer::migrate(&db)
        .await
        .map_err(|error| error.to_string())?;
    AgentAuth::migrate(&db)
        .await
        .map_err(|error| error.to_string())?;
    suprnova::Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db))
                .agent_auth(AgentAuth::new().signing_key(kid, pem)),
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// AGENT-017: `suprnova new` gives every kit's `.env` a signing key of its
/// own, which agent auth needs to start (AGENT-007), in the two lines
/// `agents:key` prints.
#[test]
fn agent_017_new_project_env_carries_a_fresh_signing_key() {
    let kid_shape = regex::Regex::new(r"^agent-(\d{8})-[0-9a-f]{8}$").expect("a pattern");
    for kit in ["svelte", "live", "api"] {
        let project = Project::new(kit);
        let key = WrittenKey::of(&project);
        let date = kid_shape
            .captures(&key.kid)
            .unwrap_or_else(|| {
                panic!(
                    "{kit}: AGENT_AUTH_KEY_ID must be agent-YYYYMMDD-<8 hex>, got {}",
                    key.kid
                )
            })
            .get(1)
            .map(|day| day.as_str().to_owned())
            .expect("the date");
        let minted = chrono::NaiveDate::parse_from_str(&date, "%Y%m%d")
            .unwrap_or_else(|error| panic!("{kit}: the key identifier's date {date}: {error}"));
        let today = chrono::Utc::now().date_naive();
        assert!(
            (today - minted).num_days().abs() <= 1,
            "{kit}: the key identifier carries the day it was made, got {date}"
        );
        assert!(
            key.quoted_pem
                .starts_with("\"-----BEGIN PRIVATE KEY-----\\n")
                && key.quoted_pem.ends_with("\\n-----END PRIVATE KEY-----\""),
            "{kit}: AGENT_AUTH_PRIVATE_KEY must be a quoted PKCS#8 PEM on one line, \
             its line breaks written as \\n"
        );
        assert!(
            !key.body_lines().is_empty()
                && key
                    .body_lines()
                    .iter()
                    .all(|line| line.bytes().all(|byte| byte.is_ascii_alphanumeric()
                        || byte == b'+'
                        || byte == b'/'
                        || byte == b'=')),
            "{kit}: the PEM's body must be base64 lines"
        );
    }
}

/// AGENT-017: the key `suprnova new` writes is one the framework's agent
/// auth accepts. Read from `.env` as the application reads it, it builds
/// agent auth; a key that is not P-256 PEM does not.
#[tokio::test]
async fn agent_017_the_written_signing_key_builds_agent_auth() {
    for kit in ["svelte", "api"] {
        let project = Project::new(kit);
        let mut kid = None;
        let mut pem = None;
        for entry in dotenvy::from_path_iter(project.root.join(".env"))
            .unwrap_or_else(|error| panic!("{kit}: .env does not parse: {error}"))
        {
            let (name, value) =
                entry.unwrap_or_else(|error| panic!("{kit}: a .env line does not parse: {error}"));
            match name.as_str() {
                "AGENT_AUTH_KEY_ID" => kid = Some(value),
                "AGENT_AUTH_PRIVATE_KEY" => pem = Some(value),
                _ => {}
            }
        }
        let kid = kid.unwrap_or_else(|| panic!("{kit}: .env names no AGENT_AUTH_KEY_ID"));
        let pem = pem.unwrap_or_else(|| panic!("{kit}: .env holds no AGENT_AUTH_PRIVATE_KEY"));
        build_agent_auth(&kid, &pem)
            .await
            .unwrap_or_else(|error| panic!("{kit}: the written key fails agent auth: {error}"));
        // A key that does not read fails the same build, so the build above
        // checked the key.
        let refused = build_agent_auth(&kid, "not a key")
            .await
            .expect_err("a key that does not read fails the build");
        assert!(refused.contains("P-256"), "{kit}: {refused}");
    }
}

/// The variable that names the project the child test below reads.
const CHILD_PROJECT: &str = "KIT_AGENTS_PROJECT";

/// The child test's name, as the test harness filters it.
const CHILD_TEST: &str = "shared::agent_017_environment_child_builds_agent_auth";

/// Run [`agent_017_environment_child_builds_agent_auth`] in a process of its
/// own on the project at `root`, with no signing key in its inherited
/// environment. Loading `.env` writes the process environment, which no
/// other test of this binary may share. Answers whether it passed, and what
/// it printed.
fn run_environment_child(root: &std::path::Path) -> (bool, String) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--ignored", "--exact", CHILD_TEST, "--test-threads=1"])
        .env(CHILD_PROJECT, root)
        .env_remove("AGENT_AUTH_KEY_ID")
        .env_remove("AGENT_AUTH_PRIVATE_KEY")
        .output()
        .expect("run the child test");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let passed = output.status.success() && printed.contains("test result: ok. 1 passed");
    (passed, printed)
}

/// AGENT-017: the application reads the key `suprnova new` wrote the way it
/// reads it at boot. `Config::init` loads `.env` into the environment, and
/// agent auth, configured as `src/agents.rs` configures it, takes its key
/// from there. A `.env` without the key fails the same build.
#[test]
fn agent_017_the_written_signing_key_builds_agent_auth_from_the_environment() {
    for kit in ["svelte", "api"] {
        let project = Project::new(kit);
        let (passed, printed) = run_environment_child(&project.root);
        assert!(
            passed,
            "{kit}: the key in .env does not build agent auth:\n{printed}"
        );

        let keyless = TempDir::new().expect("a temporary directory");
        let env = project.read(".env");
        let without_key: String = env
            .lines()
            .filter(|line| !line.starts_with("AGENT_AUTH_"))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(keyless.path().join(".env"), without_key).expect("write a keyless .env");
        let (passed, printed) = run_environment_child(keyless.path());
        assert!(
            !passed && printed.contains("AGENT_AUTH_KEY_ID"),
            "{kit}: a .env without the key must fail the build:\n{printed}"
        );
    }
}

/// The child of the test above: load the project's `.env` with the
/// framework's own loader, then build agent auth as `src/agents.rs` builds
/// it, with no key named in code.
#[test]
#[ignore = "runs in its own process, started by \
            agent_017_the_written_signing_key_builds_agent_auth_from_the_environment"]
fn agent_017_environment_child_builds_agent_auth() {
    use suprnova::agents::{AgentAuth, AgentsConfig, OAuthServer};

    let root = std::env::var_os(CHILD_PROJECT).expect("the parent test names the project");
    // Before any runtime starts: the loader refuses to write the
    // environment from inside one.
    suprnova::Config::init(std::path::Path::new(&root)).expect("the project's .env loads");
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let built = runtime.block_on(async {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .map_err(|error| error.to_string())?;
        OAuthServer::migrate(&db)
            .await
            .map_err(|error| error.to_string())?;
        AgentAuth::migrate(&db)
            .await
            .map_err(|error| error.to_string())?;
        suprnova::Router::new()
            .try_agents(
                AgentsConfig::new()
                    .oauth_server(
                        OAuthServer::new()
                            .custom_approval_page("/oauth/approve")
                            .custom_connected_agents_page()
                            .connection(db),
                    )
                    .agent_auth(AgentAuth::new().custom_claim_page("/agent/claim")),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    if let Err(error) = built {
        panic!("agent auth did not build from the environment: {error}");
    }
}

/// AGENT-017: every project gets its own key, never a shared one.
#[test]
fn agent_017_two_new_projects_get_different_signing_keys() {
    let first = WrittenKey::of(&Project::new("svelte"));
    let second = WrittenKey::of(&Project::new("svelte"));
    assert_ne!(
        first.kid, second.kid,
        "two projects must not share a key identifier"
    );
    assert!(
        first.quoted_pem != second.quoted_pem,
        "two projects must not share a signing key"
    );
}

/// AGENT-017: the key stays in `.env`, which only its owner reads, and
/// `.env.example` names both variables with no value.
#[test]
fn agent_017_env_is_private_and_the_example_documents_the_key_without_a_value() {
    for kit in ["svelte", "live", "api"] {
        let project = Project::new(kit);
        WrittenKey::of(&project);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(project.root.join(".env"))
                .expect(".env exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{kit}: .env must be 0600");
        }
        if kit == "api" {
            // The API kit writes no `.env.example`.
            continue;
        }
        let example = project.read(".env.example");
        for name in ["AGENT_AUTH_KEY_ID", "AGENT_AUTH_PRIVATE_KEY"] {
            let lines: Vec<&str> = example.lines().filter(|line| line.contains(name)).collect();
            assert_eq!(
                lines,
                vec![format!("# {name}=").as_str()],
                "{kit}: .env.example must name {name} once, with no value"
            );
        }
        assert!(
            !example.contains("PRIVATE KEY-----"),
            "{kit}: .env.example must hold no key"
        );
    }
}

/// AGENT-017: `suprnova new` prints no key material.
#[test]
fn agent_017_new_prints_no_key_material() {
    for kit in ["svelte", "live", "api"] {
        let project = Project::new(kit);
        let key = WrittenKey::of(&project);
        assert!(
            !project.printed.contains("PRIVATE KEY")
                && !project.printed.contains("AGENT_AUTH_PRIVATE_KEY"),
            "{kit}: `suprnova new` must not print the signing key"
        );
        for line in key.body_lines() {
            assert!(
                !project.printed.contains(line),
                "{kit}: `suprnova new` printed part of the signing key"
            );
        }
    }
}

/// AGENT-017: no template carries a private key, and a new project holds
/// its key in `.env` alone, which only its owner reads. Every key is minted
/// when a project is made, and a key in a template would be every project's
/// key.
#[test]
fn agent_017_no_template_holds_a_private_key() {
    let files = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/templates/files");
    let mut read = 0;
    for entry in walkdir::WalkDir::new(&files) {
        let entry = entry.expect("walk the templates");
        if !entry.file_type().is_file() {
            continue;
        }
        let text =
            String::from_utf8_lossy(&fs::read(entry.path()).expect("read a template")).into_owned();
        read += 1;
        let path = entry.path().display();
        assert!(
            !text.contains("PRIVATE KEY-----"),
            "{path} holds a private key"
        );
        for line in text.lines() {
            let line = line.trim_start_matches(['#', ' ']);
            if let Some(value) = line.strip_prefix("AGENT_AUTH_PRIVATE_KEY=") {
                assert!(value.is_empty(), "{path} assigns AGENT_AUTH_PRIVATE_KEY");
            }
        }
    }
    assert!(read > 100, "walked only {read} templates");

    for kit in ["svelte", "live", "api"] {
        let project = Project::new(kit);
        let mut written = 0;
        for entry in walkdir::WalkDir::new(&project.root) {
            let entry = entry.expect("walk the project");
            if !entry.file_type().is_file() {
                continue;
            }
            written += 1;
            let text =
                String::from_utf8_lossy(&fs::read(entry.path()).expect("read a file")).into_owned();
            let is_env = entry.path() == project.root.join(".env");
            assert_eq!(
                text.contains("PRIVATE KEY"),
                is_env,
                "{kit}: {} {} the private key",
                entry.path().display(),
                if is_env { "lacks" } else { "holds" }
            );
        }
        assert!(written > 10, "{kit}: walked only {written} files");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(project.root.join(".env"))
                .expect(".env exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{kit}: .env must be 0600");
        }
    }
}

/// AGENT-017: the Inertia and Live kits serve the MCP endpoint, the OAuth
/// server and agent auth from `src/agents.rs`, with the kit's own approval,
/// claim and connected-agents pages, and the entry serves its routes.
#[test]
fn agent_017_inertia_and_live_kits_wire_the_endpoint_and_both_plugins() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let agents = project.read("src/agents.rs");
        for part in [
            "AgentsConfig::new()",
            ".oauth_server(",
            "OAuthServer::new()",
            ".custom_approval_page(\"/oauth/approve\")",
            ".custom_connected_agents_page()",
            ".agent_auth(AgentAuth::new().custom_claim_page(\"/agent/claim\"))",
            "pub fn serve(router: Router) -> Result<Router, FrameworkError>",
            "router.try_agents(config())",
        ] {
            assert!(
                agents.contains(part),
                "{kit}: src/agents.rs lacks `{part}`:\n{agents}"
            );
        }
        // A shared tool would also appear on every page's browser tool set.
        assert!(
            !agents.contains("share_tools"),
            "{kit}: src/agents.rs must not share tools:\n{agents}"
        );
        let lib = project.read("src/lib.rs");
        assert!(
            lib.lines().any(|line| line == "pub mod agents;"),
            "{kit}: src/lib.rs must declare `pub mod agents;`:\n{lib}"
        );
        let main = project.read("cmd/main.rs");
        assert!(
            main.contains("agents::serve(routes::register())?"),
            "{kit}: cmd/main.rs must serve the routes through agents::serve:\n{main}"
        );
        assert!(
            main.contains("agents, bootstrap, config, live, migrations, routes, schedule"),
            "{kit}: cmd/main.rs must import the agents module:\n{main}"
        );
    }
}

/// AGENT-017: the kits' migrator creates both plugins' tables and, after
/// the users table, the index that keeps an address unique whatever its
/// case.
#[test]
fn agent_017_kit_migrators_create_the_plugins_tables_and_the_email_index() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let migrations = project.read("src/migrations/mod.rs");
        let squashed = migrations.split_whitespace().collect::<String>();
        let users = squashed
            .find("Box::new(m20240101_000001_create_users_table::Migration)")
            .unwrap_or_else(|| panic!("{kit}: no users migration:\n{migrations}"));
        let index = squashed
            .find(
                "Box::new(suprnova::auth::migrations::UniqueEmailIgnoringCase::on(\"users\",\"email\",))",
            )
            .unwrap_or_else(|| panic!("{kit}: no case-insensitive email index:\n{migrations}"));
        assert!(
            users < index,
            "{kit}: the email index must follow the users table:\n{migrations}"
        );
        assert!(
            migrations.contains("migrations.extend(suprnova::agents::migrations());"),
            "{kit}: the migrator must create both plugins' tables:\n{migrations}"
        );
    }
}

/// AGENT-017: a signed-in user who opens a sign-in link that carries
/// `return_to` goes there when it is a path on this application, and to
/// the dashboard otherwise. The guest routes keep their guard.
#[test]
fn agent_017_signed_in_user_follows_a_same_origin_return_to() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let middleware = project.read("src/middleware/authenticate.rs");
        assert!(
            middleware.contains("pub struct RedirectIfAuthenticated;"),
            "{kit}:\n{middleware}"
        );
        assert!(
            middleware.contains("pub fn guest() -> RedirectIfAuthenticated {"),
            "{kit}: guest() must return RedirectIfAuthenticated:\n{middleware}"
        );
        // The framework's guest middleware decides who is a guest and how a
        // signed-in user is sent on: a 409 with `X-Inertia-Location` to an
        // Inertia visit, a 302 otherwise.
        let handle = item(&middleware, "handle");
        for part in [
            "request.query_param(RETURN_TO)",
            "url::same_origin_path(",
            "unwrap_or_else(|| HOME.to_owned())",
            "GuestMiddleware::redirect_to(destination)",
            ".handle(request, next)",
        ] {
            assert!(
                contains(handle, part),
                "{kit}: RedirectIfAuthenticated lacks `{part}`:\n{handle}"
            );
        }
        let checked = handle.find("url::same_origin_path(").expect("checked");
        let sent = handle.find("GuestMiddleware::redirect_to(").expect("sent");
        assert!(
            checked < sent,
            "{kit}: return_to must be checked before it is sent:\n{handle}"
        );
        assert!(
            middleware.contains("const RETURN_TO: &str = \"return_to\";")
                && middleware.contains("const HOME: &str = \"/dashboard\";"),
            "{kit}:\n{middleware}"
        );
        let routes = project.read("src/routes.rs");
        assert!(
            routes.contains(".middleware(middleware::authenticate::guest())"),
            "{kit}: the guest routes keep their guard:\n{routes}"
        );
    }
}

/// AGENT-017: the sign-in page keeps a same-origin `return_to` as the
/// session's intended address, checked before it is stored, so it cannot
/// send anyone to another site.
#[test]
fn agent_017_login_remembers_return_to() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let middleware = project.read("src/middleware/authenticate.rs");
        assert!(
            middleware.contains("pub fn remember_return_to(request: &Request) {"),
            "{kit}:\n{middleware}"
        );
        let remember = item(&middleware, "remember_return_to");
        let checked = remember
            .find("url::same_origin_path(")
            .unwrap_or_else(|| panic!("{kit}: return_to must be checked:\n{remember}"));
        let stored = remember
            .find("Redirect::set_intended_url(")
            .unwrap_or_else(|| panic!("{kit}: return_to must be stored:\n{remember}"));
        assert!(
            checked < stored,
            "{kit}: return_to must be checked before it is stored:\n{remember}"
        );
        let auth = project.read("src/controllers/auth.rs");
        let page = if kit == "live" {
            item(&auth, "login")
        } else {
            item(&auth, "show_login")
        };
        assert!(
            page.contains("remember_return_to(&req)"),
            "{kit}: the sign-in page must remember return_to:\n{page}"
        );
    }
}

/// AGENT-017: signing in, or registering in the same session, continues
/// to the intended address, such as the claim page that sent a guest to
/// sign in. Email verification does not carry it.
#[test]
fn agent_017_sign_in_and_registration_continue_to_the_intended_address() {
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        let auth = project.read("src/controllers/auth.rs");
        for (handler, fallback) in [("login", "/dashboard"), ("register", "/verify-email")] {
            let body = item(&auth, handler);
            for part in [
                "Redirect::intended_url().is_some()".to_owned(),
                format!("Inertia::location(Redirect::intended(\"{fallback}\"))"),
                format!("redirect!(\"{fallback}\")"),
            ] {
                assert!(
                    body.contains(&part),
                    "{kit}: `{handler}` lacks `{part}`:\n{body}"
                );
            }
        }
        let verification = project.read("src/controllers/email_verification.rs");
        assert!(
            !verification.contains("intended"),
            "{kit}: email verification must not carry the intended address:\n{verification}"
        );
    }

    let project = Project::new("live");
    for (file, fallback) in [
        ("src/live/login_form.rs", "\"dashboard\""),
        ("src/live/register_form.rs", "\"verification.notice\""),
    ] {
        let source = project.read(file);
        for part in [
            "Redirect::intended_url().is_some()",
            "\"login.continue\"",
            fallback,
        ] {
            assert!(
                source.contains(part),
                "live: {file} lacks `{part}`:\n{source}"
            );
        }
    }
    let routes = project.read("src/routes.rs");
    assert!(
        authenticated_group(&routes).contains(
            r#"get!("/login/continue", controllers::auth::continue_after_sign_in).name("login.continue")"#
        ),
        "live: the continue route belongs to the signed-in user:\n{routes}"
    );
    let auth = project.read("src/controllers/auth.rs");
    let continue_after_sign_in = item(&auth, "continue_after_sign_in");
    assert!(
        continue_after_sign_in.contains("Redirect::intended(\"/dashboard\")"),
        "live:\n{continue_after_sign_in}"
    );
    let verification = project.read("src/controllers/email_verification.rs");
    assert!(
        !verification.contains("intended"),
        "live: email verification must not carry the intended address:\n{verification}"
    );
}

/// The kits' address rule, compiled from the template every kit writes as
/// `src/email_address.rs`.
mod email_address {
    include!("../../src/templates/files/backend/email_address.rs.tpl");
}

/// The `impl` block whose header is `header`, from its first line to the
/// next line that closes an item at column zero.
fn impl_block<'a>(source: &'a str, header: &str) -> &'a str {
    let start = source
        .find(&format!("impl {header} {{"))
        .unwrap_or_else(|| panic!("no `impl {header}` in:\n{source}"));
    let rest = &source[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |end| end + 3);
    &rest[..end]
}

/// AGENT-017: the kits fold an address by changing ASCII letters only. A
/// database's `LOWER` folds ASCII letters on every engine, so a stored
/// address stays reachable wherever it was written, and the length the
/// rules checked is the length stored.
#[test]
fn agent_017_email_address_folds_ascii_letters_only() {
    assert_eq!(
        email_address::normalize("  Ada@MÜNCHEN.de \n"),
        "ada@mÜnchen.de"
    );
    assert_eq!(
        email_address::normalize("ADA@EXAMPLE.COM"),
        "ada@example.com"
    );
    // `İ` lowercases to two characters under Unicode rules, which made a
    // 254-character address 264 characters long.
    let long = format!("{}@x.io", "İ".repeat(249));
    assert_eq!(long.chars().count(), 254);
    let folded = email_address::normalize(&long);
    assert_eq!(folded.chars().count(), 254, "the fold changed the length");
    assert_eq!(folded, long, "only ASCII letters may change");
    for address in [
        "ẞ@X.IO",
        "Σ@ΣΑΣ.GR",
        "Ǆ@x.io",
        "ﬀ@x.io",
        "KELVIN\u{212A}@x.io",
    ] {
        let folded = email_address::normalize(address);
        assert_eq!(folded.chars().count(), address.chars().count(), "{address}");
        assert!(
            folded
                .chars()
                .zip(address.chars())
                .all(|(after, before)| after == before.to_ascii_lowercase()),
            "{address} became {folded}"
        );
    }
}

/// AGENT-017: sign-in, registration and the reset form store and look up an
/// address in one form, folded before the rules check it, so the rules
/// check what is stored. Every kit takes the form from
/// `src/email_address.rs`, and registration's duplicate check compares
/// without case through the database's `LOWER`.
#[test]
fn agent_017_kits_fold_the_email_to_lowercase() {
    let template = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/templates/files/backend/email_address.rs.tpl"),
    )
    .expect("the address rule's template");
    let prepared = "req.transform_input(\"email\", |email| email_address::normalize(&email));";

    for kit in ["svelte", "react", "vue", "live", "api"] {
        let project = Project::new(kit);
        assert_eq!(
            project.read("src/email_address.rs"),
            template,
            "{kit}: src/email_address.rs is the template"
        );
        let lib = project.read("src/lib.rs");
        assert!(
            lib.lines().any(|line| line == "pub mod email_address;"),
            "{kit}: src/lib.rs must declare `pub mod email_address;`:\n{lib}"
        );
        // No kit folds an address with Unicode rules.
        for entry in walkdir::WalkDir::new(project.root.join("src")) {
            let entry = entry.expect("walk the scaffold");
            if entry.file_type().is_file() {
                let source = fs::read_to_string(entry.path()).expect("read a source file");
                assert!(
                    !source.contains("to_lowercase()"),
                    "{kit}: {} folds with Unicode rules",
                    entry.path().display()
                );
            }
        }
    }

    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        let auth = project.read("src/controllers/auth.rs");
        let reset = project.read("src/controllers/password_reset.rs");
        for (source, request) in [
            (&auth, "LoginRequest"),
            (&auth, "RegisterRequest"),
            (&reset, "ForgotPasswordRequest"),
        ] {
            let block = impl_block(source, &format!("FormRequest for {request}"));
            assert!(
                contains(block, "fn prepare_for_validation(req: &mut Request)")
                    && contains(block, prepared),
                "{kit}: `{request}` must fold the address before the rules run:\n{block}"
            );
        }
        assert!(
            item(&auth, "login").contains("Credentials::password(&form.email, "),
            "{kit}: login must sign in with the folded address"
        );
        let register = item(&auth, "register");
        assert!(
            register.contains("User::find_by_email(&form.email)")
                && register.contains("User::create(&form.name, &form.email, "),
            "{kit}: registration must check and store the folded address:\n{register}"
        );
        assert!(
            item(&reset, "send_link").contains("PasswordReset::send_link(&form.email, "),
            "{kit}: the reset form must look up the folded address"
        );
    }

    let project = Project::new("live");
    for (file, action) in [
        ("src/live/login_form.rs", "login"),
        ("src/live/register_form.rs", "register"),
        ("src/live/forgot_password_form.rs", "send"),
    ] {
        let source = project.read(file);
        let body = item(&source, action);
        let folded = body
            .find("email: email_address::normalize(&self.email),")
            .unwrap_or_else(|| panic!("live: {file} must fold the address:\n{body}"));
        let checked = body.find("input.validate()").expect("the rules run");
        assert!(
            folded < checked,
            "live: {file} must fold the address before the rules run:\n{body}"
        );
    }

    let project = Project::new("api");
    let users = project.read("src/controllers/users.rs");
    for request in ["RegisterRequest", "LoginRequest"] {
        let block = impl_block(&users, &format!("FormRequest for {request}"));
        assert!(
            contains(block, "fn prepare_for_validation(req: &mut Request)")
                && contains(block, prepared),
            "api: `{request}` must fold the address before the rules run:\n{block}"
        );
    }

    for kit in SHARED_BACKEND_KITS {
        let model = Project::new(kit).read("src/models/user.rs");
        let find = item(&model, "find_by_email");
        assert!(
            find.contains(".where_raw(\"LOWER(email) = LOWER(?)\", vec![email.into()])"),
            "{kit}: find_by_email must ignore case:\n{find}"
        );
    }
}

/// AGENT-017: a name is at most 255 characters and an address at most 254
/// wherever a kit takes one: the columns are `varchar(255)`, and RFC 5321
/// bounds a deliverable address at 254.
#[test]
fn agent_017_kit_forms_bound_the_name_to_255_and_the_email_to_254() {
    const NAME: &str = "max = 255";
    const EMAIL: &str = "length(max = 254";
    let email_and_name = |source: &str, input: &str, kit: &str, file: &str| {
        let fields = item(source, input);
        // A field's attributes: the text after the field before it, or after
        // the struct's brace, up to the field.
        let field = |name: &str| {
            let at = fields
                .find(&format!("{name}: String"))
                .unwrap_or_else(|| panic!("{kit}: {file} `{input}` has no `{name}`:\n{fields}"));
            let start = fields[..at]
                .rfind("String,\n")
                .or_else(|| fields[..at].find('{'))
                .unwrap_or(0);
            fields[start..at].to_owned()
        };
        let email = field("email");
        assert!(
            email.contains("email(") && email.contains(EMAIL),
            "{kit}: {file} `{input}` must bound the email to 254:\n{fields}"
        );
        if fields.contains("name: String") {
            assert!(
                field("name").contains(NAME),
                "{kit}: {file} `{input}` must bound the name to 255:\n{fields}"
            );
        }
    };

    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        let auth = project.read("src/controllers/auth.rs");
        email_and_name(&auth, "LoginRequest", kit, "auth.rs");
        email_and_name(&auth, "RegisterRequest", kit, "auth.rs");
        assert!(
            item(&auth, "RegisterRequest").contains("name: String"),
            "{kit}: registration takes a name"
        );
        let reset = project.read("src/controllers/password_reset.rs");
        email_and_name(&reset, "ForgotPasswordRequest", kit, "password_reset.rs");
        let profile = project.read("src/controllers/profile.rs");
        assert!(profile.contains("max = 255"), "{kit}: the profile name");

        // The inputs say the bound too, so a browser stops the typing.
        let extension = match kit {
            "react" => "tsx",
            "vue" => "vue",
            _ => "svelte",
        };
        let attribute = |bound: u32| match kit {
            "react" => format!("maxLength={{{bound}}}"),
            _ => format!("maxlength=\"{bound}\""),
        };
        for (page, bounds) in [
            ("Register", &[255, 254][..]),
            ("Login", &[254][..]),
            ("ForgotPassword", &[254][..]),
        ] {
            let source = project.read(&format!("frontend/src/pages/auth/{page}.{extension}"));
            for bound in bounds {
                assert!(
                    source.contains(&attribute(*bound)),
                    "{kit}: {page} must hint the bound {bound}:\n{source}"
                );
            }
        }
    }

    let project = Project::new("live");
    for (file, input) in [
        ("src/live/login_form.rs", "LoginInput"),
        ("src/live/register_form.rs", "RegisterInput"),
        ("src/live/forgot_password_form.rs", "ForgotPasswordInput"),
    ] {
        email_and_name(&project.read(file), input, "live", file);
    }
    assert!(
        item(&project.read("src/live/register_form.rs"), "RegisterInput").contains("name: String"),
        "live: registration takes a name"
    );
    assert!(
        project
            .read("src/live/profile_name.rs")
            .contains("max = 255"),
        "live: the profile name"
    );

    let project = Project::new("api");
    let users = project.read("src/controllers/users.rs");
    for request in ["RegisterRequest", "LoginRequest"] {
        email_and_name(&users, request, "api", "users.rs");
    }
    let model = project.read("src/models/user.rs");
    let create = item(&model, "create");
    assert!(
        create.contains("MAX_EMAIL_LENGTH"),
        "api: User::create must refuse an address over 254:\n{create}"
    );
}
