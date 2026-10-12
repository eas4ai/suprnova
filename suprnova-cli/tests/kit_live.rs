//! The Live starter kit (PAR-192), checked on the files a fresh
//! `suprnova new --frontend live` writes. Every test here reads text or
//! runs the CLI on the scaffold: none compiles it. The ignored
//! `live_kit_compiles_and_renders` in `scaffold_snapshot.rs` builds and
//! serves one. Each part of the kit keeps its tests in its own module.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// The package every scaffold here is named, which prefixes its Live
/// component names.
const NAME: &str = "kit_live_app";

/// A project `suprnova new --frontend live` wrote into a temporary
/// directory under this package's target directory.
struct Scaffold {
    root: PathBuf,
    _dir: tempfile::TempDir,
}

impl Scaffold {
    fn new() -> Self {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
        let output = Command::new(BIN)
            .args([
                "new",
                NAME,
                "--no-interaction",
                "--no-git",
                "--frontend",
                "live",
            ])
            .current_dir(dir.path())
            .output()
            .expect("spawn suprnova");
        assert!(
            output.status.success(),
            "`suprnova new {NAME} --frontend live` failed:\n{}",
            combined(&output)
        );
        let root = dir.path().join(NAME);
        Self { root, _dir: dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn read(&self, rel: &str) -> String {
        let path = self.path(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("the Live scaffold must write {}: {e}", path.display()))
    }

    /// Runs the CLI in the project directory.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .current_dir(&self.root)
            .env_remove("SUPRNOVA_FRONTEND")
            .output()
            .expect("spawn suprnova")
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Whether `haystack` contains `needle` once both lose their whitespace,
/// so a call rustfmt wraps over several lines still matches the one-line
/// form a test names.
fn contains(haystack: &str, needle: &str) -> bool {
    let squash = |text: &str| text.split_whitespace().collect::<String>();
    squash(haystack).contains(&squash(needle))
}

/// Every file under `dir`, recursively, by path.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap_or_else(|e| panic!("read {next:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The pages of the kit: route, template and the layout it extends.
const PAGES: [(&str, &str, &str); 10] = [
    ("/", "home", "app"),
    ("/login", "login", "guest"),
    ("/register", "register", "guest"),
    ("/forgot-password", "forgot-password", "guest"),
    ("/reset-password", "reset-password", "guest"),
    ("/verify-email", "verify-email", "guest"),
    ("/dashboard", "dashboard", "app"),
    ("/notes", "notes-index", "app"),
    ("/notes/{id}", "notes-show", "app"),
    ("(error)", "error", "app"),
];

/// The kit's Live components: module, view, and the actions its view
/// invokes.
const COMPONENTS: [(&str, &[&str]); 9] = [
    ("login_form", &["login"]),
    ("register_form", &["register"]),
    ("forgot_password_form", &["send"]),
    ("reset_password_form", &["reset"]),
    ("verify_email", &["resend"]),
    ("profile_name", &["save"]),
    ("note_stats", &["refresh"]),
    ("note_form", &["save"]),
    ("note_list", &["search", "clear_search", "load_more"]),
];

mod no_node {
    //! The kit has no Node toolchain: no `frontend/`, no `package.json`,
    //! no Vite config, and a Dockerfile with no Node stage.

    use super::*;

    #[test]
    fn kit_live_writes_no_frontend_and_no_node_project() {
        let app = Scaffold::new();
        assert!(
            !app.path("frontend").exists(),
            "the Live kit has no frontend/"
        );
        for file in files_under(&app.root) {
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
            assert!(
                !matches!(name, "package.json" | "package-lock.json")
                    && !name.starts_with("vite.config")
                    && !name.ends_with(".svelte")
                    && !name.ends_with(".tsx")
                    && !name.ends_with(".vue"),
                "the Live kit wrote {}",
                file.display()
            );
        }
        let gitignore = app.read(".gitignore");
        assert!(!gitignore.contains("node_modules"), "{gitignore}");
        assert!(!gitignore.contains("frontend/"), "{gitignore}");
    }

    #[test]
    fn kit_live_env_names_the_kit_and_runs_no_vite() {
        let app = Scaffold::new();
        let env = app.read(".env");
        assert!(env.contains("SUPRNOVA_FRONTEND=live"), "{env}");
        assert!(!env.contains("VITE_PORT"), "{env}");
        assert!(!env.contains("InertiaConfig"), "{env}");
        assert!(env.contains("SERVER_PORT=8765"), "{env}");
    }

    #[test]
    fn kit_live_bootstrap_installs_no_inertia_layer() {
        let app = Scaffold::new();
        let bootstrap = app.read("src/bootstrap.rs");
        for absent in [
            "Inertia::install",
            "InertiaConfig",
            "inertia_share",
            "templates/app.html",
        ] {
            assert!(!bootstrap.contains(absent), "{absent} in:\n{bootstrap}");
        }
        assert!(
            contains(&bootstrap, "crate::live::registry()"),
            "{bootstrap}"
        );
        assert!(
            contains(&bootstrap, "crate::live::authorize();"),
            "{bootstrap}"
        );
        for middleware in [
            "global_middleware!(SessionMiddleware::new(session_config.clone()));",
            "global_middleware!(ErrorPage);",
            "global_middleware!(CsrfMiddleware::new().with_session_config(&session_config));",
        ] {
            assert!(
                contains(&bootstrap, middleware),
                "{middleware} in:\n{bootstrap}"
            );
        }
        assert!(!app.path("templates/app.html").exists());
        assert!(!app.path("src/props").exists());
    }

    #[test]
    fn kit_live_dockerfile_has_no_node_stage() {
        let app = Scaffold::new();
        let output = app.run(&["docker:init"]);
        assert!(output.status.success(), "{}", combined(&output));
        assert!(
            combined(&output).contains("Live kit"),
            "{}",
            combined(&output)
        );
        let dockerfile = app.read("Dockerfile");
        let instructions: Vec<&str> = dockerfile
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .collect();
        assert!(instructions.len() > 10, "{dockerfile}");
        let body = instructions.join("\n");
        for absent in ["FROM node", "npm", "frontend/", "public/assets"] {
            assert!(!body.contains(absent), "`{absent}` in:\n{dockerfile}");
        }
        for present in [
            "COPY templates/ ./templates/",
            "COPY templates/suprnova-ui/ ./templates/suprnova-ui/",
            "COPY public/ ./public/",
            "COPY lang/ ./lang/",
            &format!("cargo new --bin {NAME}"),
        ] {
            assert!(
                body.contains(present),
                "`{present}` missing in:\n{dockerfile}"
            );
        }
    }

    #[test]
    fn kit_live_make_inertia_refuses_and_names_the_kit() {
        let app = Scaffold::new();
        let output = app.run(&["make:inertia", "Reports"]);
        assert!(!output.status.success(), "{}", combined(&output));
        assert!(
            combined(&output).contains("Live kit"),
            "{}",
            combined(&output)
        );
        assert!(!app.path("frontend").exists());
    }
}

mod documents {
    //! Every page is a Live document rendered through `LiveDocument`, with
    //! the framework's bootstrap tags, in one of the two layouts.

    use super::*;

    #[test]
    fn kit_live_every_page_view_names_its_layout() {
        let app = Scaffold::new();
        let views = app.read("src/views/mod.rs");
        for (_, page, layout) in PAGES {
            let template = app.read(&format!("templates/live/pages/{page}.html"));
            assert!(
                template.starts_with(&format!("{{% extends \"live/layouts/{layout}.html\" %}}")),
                "{page} must extend the {layout} layout:\n{template}"
            );
            assert!(
                views.contains(&format!(
                    "#[suprnova::view(path = \"live/pages/{page}.html\")]"
                )),
                "src/views/mod.rs declares no view for {page}"
            );
        }
    }

    #[test]
    fn kit_live_layouts_carry_the_bootstrap_navigation_user_and_toast() {
        let app = Scaffold::new();
        let head = app.read("templates/live/layouts/head.html");
        assert!(
            head.contains("{% if let Some(bootstrap) = frame.bootstrap %}{{ bootstrap|trusted_html }}{% endif %}"),
            "{head}"
        );
        let toast = app.read("templates/live/layouts/toast.html");
        assert!(toast.contains("toast::toast_region(\"toasts\")"), "{toast}");
        assert!(toast.contains("frame.toast"), "{toast}");
        for layout in ["guest", "app"] {
            let template = app.read(&format!("templates/live/layouts/{layout}.html"));
            for part in [
                "{% include \"live/layouts/head.html\" %}",
                "{% include \"live/layouts/toast.html\" %}",
                "{% block content %}{% endblock %}",
                &format!("data-layout=\"{layout}\""),
            ] {
                assert!(
                    template.contains(part),
                    "the {layout} layout lacks {part}:\n{template}"
                );
            }
        }
        let app_layout = app.read("templates/live/layouts/app.html");
        for part in [
            "header::header_bar(",
            "header::header_link(frame.href(\"/dashboard\"), \"Dashboard\"",
            "header::header_link(frame.href(\"/notes\"), \"Notes\"",
            "menu::account_menu(\"account-menu\", user.name, user.initials)",
            "menu::account_sign_out(frame.href(\"/logout\"), frame.csrf)",
        ] {
            assert!(
                app_layout.contains(part),
                "the app layout lacks {part}:\n{app_layout}"
            );
        }
    }

    #[test]
    fn kit_live_controllers_render_live_documents_with_the_bootstrap() {
        let app = Scaffold::new();
        let views = app.read("src/views/mod.rs");
        assert!(
            contains(
                &views,
                "document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?"
            ),
            "{views}"
        );
        assert!(
            contains(
                &views,
                "document.render(name, page, intent, AssetSet::empty())?"
            ),
            "{views}"
        );
        for (controller, pages) in [
            ("home", &["home"][..]),
            ("auth", &["login", "register"][..]),
            ("password_reset", &["forgot-password", "reset-password"][..]),
            ("email_verification", &["verify-email"][..]),
            ("dashboard", &["dashboard"][..]),
            ("notes", &["notes-index", "notes-show"][..]),
        ] {
            let source = app.read(&format!("src/controllers/{controller}.rs"));
            assert!(
                source.contains("LiveDocument::from_request(req)?"),
                "{controller}:\n{source}"
            );
            assert!(
                source.contains("Frame::live(&mut document,"),
                "{controller}:\n{source}"
            );
            for page in pages {
                assert!(
                    source.contains(&format!("\"live/pages/{page}.html\"")),
                    "{controller} renders no {page}:\n{source}"
                );
            }
        }
    }

    #[test]
    fn kit_live_error_page_renders_framework_errors() {
        let app = Scaffold::new();
        let errors = app.read("src/controllers/errors.rs");
        for part in [
            "impl Middleware for ErrorPage",
            "!request.expects_json() && !request.path().starts_with(\"/__live/\")",
            "!(status >= 500 && Config::is_debug())",
            "HttpResponse::html(html).status(status)",
        ] {
            assert!(contains(&errors, part), "{part} in:\n{errors}");
        }
    }
}

mod components {
    //! The Live components where a page acts: each is registered, mounted,
    //! and its view invokes the actions the component declares.

    use super::*;

    #[test]
    fn kit_live_components_exist_register_and_name_their_actions() {
        let app = Scaffold::new();
        let module = app.read("src/live/mod.rs");
        for (component, actions) in COMPONENTS {
            let source = app.read(&format!("src/live/{component}.rs"));
            let kebab = component.replace('_', "-");
            assert!(
                source.contains(&format!("name = \"{NAME}.{kebab}\"")),
                "{component} is not named {NAME}.{kebab}:\n{source}"
            );
            assert!(
                source.contains(&format!("view = \"live/{component}.html\"")),
                "{component}:\n{source}"
            );
            assert!(
                module.contains(&format!("pub mod {component};")),
                "{module}"
            );
            assert!(
                module.contains(&format!(".register::<{component}::")),
                "{component} is not registered:\n{module}"
            );
            let view = app.read(&format!("templates/live/{component}.html"));
            for action in actions {
                assert!(
                    contains(&source, &format!("#[action] pub fn {action}("))
                        || contains(&source, &format!("#[action] pub async fn {action}(")),
                    "{component} declares no action {action}:\n{source}"
                );
                assert!(
                    view.contains(&format!("live:submit.prevent=\"{action}\""))
                        || view.contains(&format!("action=\"{action}\""))
                        || view.contains(&format!("::load_more(\"{action}\"")),
                    "the {component} view never invokes {action}:\n{view}"
                );
            }
        }
    }

    #[test]
    fn kit_live_auth_forms_are_public_seeds_and_the_rest_identity_bound() {
        let app = Scaffold::new();
        let module = app.read("src/live/mod.rs");
        for (field, route) in [
            ("login", "/login"),
            ("register", "/register"),
            ("forgot_password", "/forgot-password"),
            ("reset_password", "/reset-password"),
        ] {
            assert!(
                contains(
                    &module,
                    &format!("{field}: LiveMount::public_seed(\"{route}\",")
                ),
                "{field}:\n{module}"
            );
        }
        for (field, route) in [
            ("verify_email", "/verify-email"),
            ("profile", "/dashboard"),
            ("stats", "/dashboard"),
            ("note_form", "/notes"),
            ("note_list", "/notes"),
        ] {
            assert!(
                contains(
                    &module,
                    &format!("{field}: LiveMount::identity_bound(\"{route}\",")
                ),
                "{field}:\n{module}"
            );
            assert!(
                contains(&module, &format!(".try_live_mount(&mounts.{field})")),
                "{field}:\n{module}"
            );
        }
    }

    #[test]
    fn kit_live_forms_show_validation_errors_inline() {
        let app = Scaffold::new();
        for (component, fields) in [
            ("login_form", &["email", "password"][..]),
            (
                "register_form",
                &["name", "email", "password", "password_confirmation"][..],
            ),
            ("forgot_password_form", &["email"][..]),
            (
                "reset_password_form",
                &["password", "password_confirmation"][..],
            ),
            ("profile_name", &["name"][..]),
            ("note_form", &["title", "body"][..]),
        ] {
            let view = app.read(&format!("templates/live/{component}.html"));
            for field in fields {
                assert!(
                    view.contains(&format!("hint=errors.get(\"{field}\")")),
                    "the {component} view shows no message for {field}:\n{view}"
                );
            }
            let source = app.read(&format!("src/live/{component}.rs"));
            assert!(
                source.contains("FieldErrors::from_rules(&errors)"),
                "{component}:\n{source}"
            );
        }
        let register = app.read("src/live/register_form.rs");
        assert!(
            register.contains("\"Passwords do not match.\""),
            "{register}"
        );
        let login = app.read("src/live/login_form.rs");
        assert!(
            login.contains("\"These credentials do not match our records.\""),
            "{login}"
        );
    }

    #[test]
    fn kit_live_dashboard_stats_refresh_without_a_page_load() {
        let app = Scaffold::new();
        let stats = app.read("src/live/note_stats.rs");
        for part in [
            "streams(stream(name = \"notes\", topics(\"notes/:principal\"), events(NotesChanged)))",
            "#[rendering] pub async fn load(&mut self)",
            "streams.refresh(&topic(user_id))",
            "Note::owned_by(user.id).count()",
            ".limit(RECENT_NOTES)",
        ] {
            assert!(contains(&stats, part), "{part} in:\n{stats}");
        }
        let form = app.read("src/live/note_form.rs");
        assert!(
            contains(&form, "note_stats::publish_refresh(user.id).await;"),
            "{form}"
        );
        let module = app.read("src/live/mod.rs");
        assert!(
            module.contains(&format!("\"live:{NAME}.note-stats.stream.notes\"")),
            "{module}"
        );
    }

    #[test]
    fn kit_live_notes_list_searches_by_url_and_loads_more() {
        let app = Scaffold::new();
        let list = app.read("src/live/note_list.rs");
        for part in [
            "#[url(key = \"search\")] search: String",
            "#[model(debounce = 250)] query: String",
            "url_intent(CanonicalValue::Object(query))",
            "Note::owned_by(user.id)",
            ".latest_by(\"id\")",
            "minimum_protocol_version = 2",
        ] {
            assert!(contains(&list, part), "{part} in:\n{list}");
        }
        let view = app.read("templates/live/note_list.html");
        for part in [
            "search_input::search_input(\"query\"",
            "more::load_more_list(\"notes\"",
            "more::load_more(\"load_more\", exhausted=!more)",
            "live:key=\"{{ note.key|live_key }}\"",
        ] {
            assert!(view.contains(part), "{part} in:\n{view}");
        }
        let controller = app.read("src/controllers/notes.rs");
        assert!(
            controller.contains("req.query_param(\"search\")"),
            "{controller}"
        );
    }

    #[test]
    fn kit_live_flows_toast_once_through_the_session() {
        let app = Scaffold::new();
        let flash = app.read("src/flash.rs");
        assert!(contains(&flash, "session.put(TOAST_KEY, &self)"), "{flash}");
        assert!(
            contains(&flash, "session.pull::<Toast>(TOAST_KEY)"),
            "{flash}"
        );
        let views = app.read("src/views/mod.rs");
        assert!(views.contains("toast: Toast::take(),"), "{views}");
        for (file, toast) in [
            ("src/live/login_form.rs", "Toast::success(\"Signed in.\")"),
            ("src/live/note_form.rs", "Toast::success(\"Note saved.\")"),
            (
                "src/live/profile_name.rs",
                "Toast::success(\"Your name was saved.\")",
            ),
            (
                "src/live/reset_password_form.rs",
                "Toast::success(\"Your password was reset.\")",
            ),
            (
                "src/controllers/auth.rs",
                "Toast::info(\"Signed out.\").flash()?",
            ),
        ] {
            assert!(app.read(file).contains(toast), "{file} lacks {toast}");
        }
    }
}

mod routes {
    //! The routes mount the documents under Laravel's names, and the Live
    //! module installs the reserved routes and the library assets.

    use super::*;

    #[test]
    fn kit_live_routes_mount_the_documents_and_register_live() {
        let app = Scaffold::new();
        let routes = app.read("src/routes.rs");
        for route in [
            r#"get!("/", controllers::home::index).name("home")"#,
            r#"get!("/login", controllers::auth::login).name("login")"#,
            r#"get!("/register", controllers::auth::register).name("register")"#,
            r#"get!("/forgot-password", controllers::password_reset::forgot).name("password.request")"#,
            r#"get!("/reset-password", controllers::password_reset::reset).name("password.reset")"#,
            r#"get!("/dashboard", controllers::dashboard::index).name("dashboard")"#,
            r#"get!("/notes", controllers::notes::index).name("notes.index")"#,
            r#"get!("/notes/{id}", controllers::notes::show).name("notes.show")"#,
            r#"post!("/logout", controllers::auth::logout).name("logout")"#,
            r#"get!("/verify-email", controllers::email_verification::notice).name("verification.notice")"#,
            r#"get!("/verify-email/verify", controllers::email_verification::verify).name("verification.verify")"#,
        ] {
            assert!(contains(&routes, route), "`{route}` missing:\n{routes}");
        }
        assert!(
            contains(&routes, ".middleware(middleware::authenticate::guest())"),
            "{routes}"
        );
        assert!(
            contains(
                &routes,
                ".middleware(middleware::authenticate::auth()).middleware(live::tenant())"
            ),
            "{routes}"
        );

        let module = app.read("src/live/mod.rs");
        for part in [
            "router.try_live_with(",
            ".middleware(AuthMiddleware::optional())",
            ".try_live_ui_assets()?",
            ".try_live_document(\"/\")?",
            ".try_live_document(\"/notes/{id}\")?",
            "RenderCache::install(routes(router)?, RenderCacheConfig::from_env()?)",
        ] {
            assert!(contains(&module, part), "{part} in:\n{module}");
        }
        let main = app.read("cmd/main.rs");
        assert!(
            contains(&main, "live::routes_with_render_cache(routes::register())"),
            "{main}"
        );
    }

    #[test]
    fn kit_live_installs_the_library_components_its_views_import() {
        let app = Scaffold::new();
        let project = app.read("suprnova.toml");
        let mut imported = std::collections::BTreeSet::new();
        for file in files_under(&app.path("templates/live")) {
            let text = std::fs::read_to_string(&file).expect("read view");
            for part in text.split("{% import \"suprnova-ui/").skip(1) {
                if let Some(name) = part.split('/').next() {
                    imported.insert(name.to_owned());
                }
            }
        }
        assert!(imported.len() >= 10, "{imported:?}");
        for name in &imported {
            let dir = app.path(&format!("templates/suprnova-ui/{name}"));
            assert!(
                dir.join(format!("{name}.html")).is_file(),
                "{name} is imported but not installed"
            );
            assert!(
                project.contains(&format!("[live.components.\"suprnova/{name}\"]")),
                "suprnova.toml does not record {name}:\n{project}"
            );
        }
        assert!(app.path("public/css/app.css").is_file());
    }

    #[test]
    fn kit_live_shares_the_kits_backend() {
        let app = Scaffold::new();
        let note = app.read("src/models/note.rs");
        assert!(
            note.contains("pub fn owned_by(user_id: u64) -> Builder<Self>"),
            "{note}"
        );
        let migrations = app.read("src/migrations/mod.rs");
        assert!(
            migrations.contains("m20240101_000005_create_notes_table"),
            "{migrations}"
        );
        assert!(
            migrations.contains("Box::new(suprnova::render_cache::migration::Migration),"),
            "{migrations}"
        );
        let catalog = app.read("lang/en/app.ftl");
        for id in ["welcome = ", "error-reference = ", "error-go-home = "] {
            assert!(catalog.contains(id), "{catalog}");
        }
    }
}

mod cli {
    //! `suprnova new` accepts `live` and names it among the frontends.

    use super::*;

    #[test]
    fn kit_live_new_help_and_refusal_name_live() {
        let help = Command::new(BIN)
            .args(["new", "--help"])
            .output()
            .expect("spawn suprnova");
        assert!(combined(&help).contains("live"), "{}", combined(&help));

        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
        let unknown = Command::new(BIN)
            .args([
                "new",
                "odd_kit",
                "--no-interaction",
                "--no-git",
                "--frontend",
                "angular",
            ])
            .current_dir(dir.path())
            .output()
            .expect("spawn suprnova");
        assert!(
            combined(&unknown).contains("Supported: react, svelte, vue, live"),
            "{}",
            combined(&unknown)
        );
    }
}
