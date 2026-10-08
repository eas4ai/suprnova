//! The starter kits on Inertia 3.8 (PAR-076 to PAR-080), checked on the
//! files a fresh `suprnova new` writes. Every test here reads text: none
//! runs npm or compiles the scaffold. Each part of the kits keeps its tests
//! in its own module.

mod backend {
    //! The backend every kit shares: the routes and their names, the notes
    //! table and model, the dashboard, notes and profile handlers, the flash
    //! struct and the toasts the account flows flash (PAR-078, PAR-080), and
    //! the Inertia client pin the scaffold tests hold the kits to (PAR-076).
    //!
    //! The backend templates are the same for every frontend, and each test
    //! still scaffolds all three: a kit that wrote a backend file of its own
    //! would show up here.

    use std::path::{Path, PathBuf};
    use std::process::Command;

    const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

    const KITS: [&str; 3] = ["svelte", "react", "vue"];

    /// A project `suprnova new` wrote into a temporary directory.
    struct Scaffold {
        kit: &'static str,
        root: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Scaffold {
        fn new(kit: &'static str) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let output = Command::new(BIN)
                .args([
                    "new",
                    "kit_app",
                    "--no-interaction",
                    "--no-git",
                    "--frontend",
                    kit,
                ])
                .current_dir(dir.path())
                .output()
                .expect("spawn suprnova");
            assert!(
                output.status.success(),
                "`suprnova new kit_app --frontend {kit}` failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let root = dir.path().join("kit_app");
            Self {
                kit,
                root,
                _dir: dir,
            }
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn read(&self, rel: &str) -> String {
            let path = self.path(rel);
            std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "the {} scaffold must write {}: {e}",
                    self.kit,
                    path.display()
                )
            })
        }

        /// Every `.rs` file under `src/controllers`, by file name.
        fn controllers(&self) -> Vec<(String, String)> {
            let dir = self.path("src/controllers");
            let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
                .map(|entry| entry.expect("dir entry").path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
                .map(|path| {
                    let name = path
                        .file_name()
                        .expect("file name")
                        .to_string_lossy()
                        .into_owned();
                    let body = std::fs::read_to_string(&path)
                        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                    (name, body)
                })
                .collect();
            files.sort();
            files
        }
    }

    /// Whether `haystack` contains `needle` once both lose their
    /// whitespace, so a call rustfmt wraps over several lines still
    /// matches the one-line form a test names.
    fn contains(haystack: &str, needle: &str) -> bool {
        let squash = |text: &str| text.split_whitespace().collect::<String>();
        squash(haystack).contains(&squash(needle))
    }

    /// Run `check` on a fresh scaffold of every kit.
    fn each_kit(check: impl Fn(&Scaffold)) {
        for kit in KITS {
            check(&Scaffold::new(kit));
        }
    }

    /// The body of the function `name` in `source`: from its `fn` line to
    /// the next line that closes an item at column zero.
    fn function<'a>(source: &'a str, name: &str) -> &'a str {
        let start = source
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no `fn {name}(` in:\n{source}"));
        let rest = &source[start..];
        let end = rest.find("\n}\n").map_or(rest.len(), |end| end + 3);
        &rest[..end]
    }

    /// The routes of the guest group and of the authenticated group in a
    /// scaffolded `src/routes.rs`.
    fn route_groups(routes: &str) -> (&str, &str) {
        let guest_end = routes
            .find(".middleware(middleware::authenticate::guest())")
            .expect("routes.rs has a guest group");
        let auth_end = routes
            .find(".middleware(middleware::authenticate::auth())")
            .expect("routes.rs has an authenticated group");
        assert!(guest_end < auth_end, "the guest group comes first");
        (&routes[..guest_end], &routes[guest_end..auth_end])
    }

    /// PAR-080: the kit pages and the JSON handler sit behind the same
    /// authentication as the dashboard, each under Laravel's route name,
    /// and the account routes carry Laravel's names too.
    #[test]
    fn kit_routes_name_every_page_and_put_the_kit_pages_behind_authentication() {
        each_kit(|app| {
            let routes = app.read("src/routes.rs");
            let (guest, auth) = route_groups(&routes);
            for route in [
                r#"get!("/login", controllers::auth::show_login).name("login")"#,
                r#"get!("/register", controllers::auth::show_register).name("register")"#,
                r#"get!("/forgot-password", controllers::password_reset::forgot).name("password.request")"#,
                r#"post!("/forgot-password", controllers::password_reset::send_link).name("password.email")"#,
                r#"get!("/reset-password", controllers::password_reset::reset_form).name("password.reset")"#,
                r#"post!("/reset-password", controllers::password_reset::reset).name("password.update")"#,
            ] {
                assert!(
                    contains(guest, route),
                    "{}: `{route}` must sit in the guest group; got:\n{routes}",
                    app.kit
                );
            }
            for route in [
                r#"get!("/dashboard", controllers::dashboard::index).name("dashboard")"#,
                r#"get!("/notes", controllers::notes::index).name("notes.index")"#,
                r#"get!("/notes/{id}", controllers::notes::show).name("notes.show")"#,
                r#"post!("/notes", controllers::notes::store).name("notes.store")"#,
                r#"post!("/profile/name", controllers::profile::update_name).name("profile.name")"#,
                r#"post!("/logout", controllers::auth::logout).name("logout")"#,
                r#"get!("/verify-email", controllers::email_verification::notice).name("verification.notice")"#,
                r#"post!("/email/verification-notification", controllers::email_verification::resend).name("verification.send")"#,
                r#"get!("/verify-email/verify", controllers::email_verification::verify).name("verification.verify")"#,
            ] {
                assert!(
                    contains(auth, route),
                    "{}: `{route}` must sit in the authenticated group; got:\n{routes}",
                    app.kit
                );
            }

            let controllers = app.read("src/controllers/mod.rs");
            for module in ["pub mod notes;", "pub mod profile;"] {
                assert!(
                    contains(&controllers, module),
                    "{}: controllers/mod.rs must declare `{module}`; got:\n{controllers}",
                    app.kit
                );
            }
        });
    }

    /// PAR-080: a `notes` table owned by a user, and the model on both
    /// sides of the relation.
    #[test]
    fn kit_notes_table_and_model_belong_to_a_user() {
        each_kit(|app| {
            let migration = app.read("src/migrations/m20240101_000005_create_notes_table.rs");
            for needle in [
                r#"Schema::create(manager, "notes""#,
                "t.unsigned_id();",
                r#"t.unsigned_foreign_id("user_id")"#,
                r#".constrained("users")"#,
                r#"t.string("title");"#,
                r#"t.text("body").nullable();"#,
                r#"t.date_time("created_at")"#,
                r#"t.date_time("updated_at")"#,
                r#"Schema::drop_if_exists(manager, "notes")"#,
            ] {
                assert!(
                    contains(&migration, needle),
                    "{}: the notes migration must contain `{needle}`; got:\n{migration}",
                    app.kit
                );
            }

            let migrator = app.read("src/migrations/mod.rs");
            for needle in [
                "mod m20240101_000005_create_notes_table;",
                "Box::new(m20240101_000005_create_notes_table::Migration),",
            ] {
                assert!(
                    contains(&migrator, needle),
                    "{}: the migrator must list the notes migration (`{needle}`); got:\n{migrator}",
                    app.kit
                );
            }
            let users = migrator
                .find("Box::new(m20240101_000001_create_users_table::Migration)")
                .expect("the migrator lists the users migration");
            let notes = migrator
                .find("Box::new(m20240101_000005_create_notes_table::Migration)")
                .expect("the migrator lists the notes migration");
            assert!(
                users < notes,
                "{}: the notes table references users, so it must be created after them",
                app.kit
            );

            let models = app.read("src/models/mod.rs");
            assert!(
                contains(&models, "pub mod note;"),
                "{}: models/mod.rs must declare the note model; got:\n{models}",
                app.kit
            );
            let note = app.read("src/models/note.rs");
            for needle in [
                "#[model(",
                r#"table = "notes""#,
                "user: BelongsTo<crate::models::user::User>",
                "pub struct Note {",
                "pub user_id: u64,",
                "pub title: String,",
                "pub body: Option<String>,",
                "pub fn owned_by(user_id: u64)",
                r#".filter("user_id", user_id)"#,
            ] {
                assert!(
                    contains(&note, needle),
                    "{}: models/note.rs must contain `{needle}`; got:\n{note}",
                    app.kit
                );
            }
            let user = app.read("src/models/user.rs");
            assert!(
                contains(&user, "notes: HasMany<crate::models::note::Note>"),
                "{}: the user model must have many notes; got:\n{user}",
                app.kit
            );
        });
    }

    /// PAR-080 and PAR-079: the dashboard sends `stats` deferred and
    /// `recent_notes` optional, both over the user's own notes. The user
    /// reaches every page through the shared `auth` prop, so the dashboard
    /// does not send it again.
    #[test]
    fn kit_dashboard_defers_stats_and_leaves_recent_notes_optional() {
        each_kit(|app| {
            let dashboard = app.read("src/controllers/dashboard.rs");
            let index = function(&dashboard, "index");
            for needle in [
                r#"InertiaResponse::new("Dashboard")"#,
                r#".defer("stats", "#,
                r#".optional("recent_notes", "#,
            ] {
                assert!(
                    contains(index, needle),
                    "{}: the dashboard handler must chain `{needle}`; got:\n{index}",
                    app.kit
                );
            }
            assert!(
                !contains(index, r#".with("user", "#),
                "{}: the dashboard must leave the user to the shared `auth` prop; got:\n{index}",
                app.kit
            );
            for needle in [
                "#[derive(InertiaProps)]\npub struct DashboardProps {",
                "pub stats: Stats,",
                "pub recent_notes: Vec<NoteSummary>,",
                "#[derive(InertiaProps)]\npub struct Stats {",
                "pub notes: i64,",
                "pub written_today: i64,",
            ] {
                assert!(
                    contains(&dashboard, needle),
                    "{}: dashboard.rs must declare `{needle}`; got:\n{dashboard}",
                    app.kit
                );
            }
            let stats = function(&dashboard, "note_stats");
            assert!(
                contains(stats, "Note::owned_by(user_id)") && contains(stats, ".count()"),
                "{}: `stats` must count the signed-in user's own notes; got:\n{stats}",
                app.kit
            );
            assert!(
                contains(stats, r#".where_date("created_at", "#),
                "{}: `written_today` must count the notes written today; got:\n{stats}",
                app.kit
            );
            let recent = function(&dashboard, "recent_notes");
            assert!(
                contains(recent, "Note::owned_by(user_id)")
                    && contains(recent, r#".latest_by("id")"#)
                    && contains(recent, ".limit(RECENT_NOTES)")
                    && contains(&dashboard, "const RECENT_NOTES: u64 = 5;"),
                "{}: `recent_notes` must be the user's five newest; got:\n{recent}",
                app.kit
            );
        });
    }

    /// PAR-080: the notes index scrolls the signed-in user's own notes by
    /// cursor, filtered by `search`, and sends `search` back.
    #[test]
    fn kit_notes_index_scrolls_the_users_own_notes_by_cursor() {
        each_kit(|app| {
            let notes = app.read("src/controllers/notes.rs");
            let index = function(&notes, "index");
            for needle in [
                "Note::owned_by(user.id)",
                r#"req.query_param("search")"#,
                ".cursor_paginate(NOTES_PER_PAGE)",
                r#"Inertia::paginate("Notes/Index", "notes", "#,
                r#".with("search", "#,
                ".map(NoteSummary::from)",
            ] {
                assert!(
                    contains(index, needle),
                    "{}: the notes index must contain `{needle}`; got:\n{index}",
                    app.kit
                );
            }
            for needle in [
                "#[derive(InertiaProps)]\npub struct NotesIndexProps {",
                "pub notes: Vec<NoteSummary>,",
                "pub search: String,",
                "#[derive(InertiaProps)]\npub struct NoteSummary {",
            ] {
                assert!(
                    contains(&notes, needle),
                    "{}: notes.rs must declare `{needle}`; got:\n{notes}",
                    app.kit
                );
            }
            assert!(
                contains(&notes, "const NOTES_PER_PAGE: u64 = 10;"),
                "{}: the notes index pages ten notes at a time; got:\n{notes}",
                app.kit
            );
            for page_numbers in [".paginate(", ".simple_paginate("] {
                assert!(
                    !index
                        .replace("Inertia::paginate(", "")
                        .contains(page_numbers),
                    "{}: the notes index must page by cursor, not by number; got:\n{index}",
                    app.kit
                );
            }
            // The filter matches title or body without regard to case, and
            // a `%` or `_` in the search is text, not a wildcard.
            for needle in ["LOWER(title) LIKE ?", "LOWER(body) LIKE ?", "ESCAPE '!'"] {
                assert!(
                    contains(&notes, needle),
                    "{}: the search filter must contain `{needle}`; got:\n{notes}",
                    app.kit
                );
            }
        });
    }

    /// PAR-080: the show handler reads the note through the signed-in
    /// user's own notes, so another user's note is the framework's 404.
    #[test]
    fn kit_notes_show_is_scoped_to_the_signed_in_user() {
        each_kit(|app| {
            let notes = app.read("src/controllers/notes.rs");
            let show = function(&notes, "show");
            for needle in [
                "id: u64",
                "Note::owned_by(user.id)",
                r#".filter("id", id)"#,
                ".first_or_fail()",
                r#"inertia_response!(&req, "Notes/Show", NotesShowProps"#,
            ] {
                assert!(
                    contains(show, needle),
                    "{}: the notes show handler must contain `{needle}`; got:\n{show}",
                    app.kit
                );
            }
            for needle in [
                "#[derive(InertiaProps)]\npub struct NotesShowProps {",
                "pub note: NoteView,",
                "#[derive(InertiaProps)]\npub struct NoteView {",
                "pub body: Option<String>,",
            ] {
                assert!(
                    contains(&notes, needle),
                    "{}: notes.rs must declare `{needle}`; got:\n{notes}",
                    app.kit
                );
            }
        });
    }

    /// PAR-080 and PAR-078: the store handler validates the title and the
    /// body, writes the note for the signed-in user, flashes the saved
    /// toast and returns to the list.
    #[test]
    fn kit_notes_store_validates_writes_for_the_user_and_flashes() {
        each_kit(|app| {
            let notes = app.read("src/controllers/notes.rs");
            for needle in [
                "pub struct StoreNoteRequest {",
                "#[validate(length(min = 1, max = 255,",
                "#[validate(length(max = 10000,",
            ] {
                assert!(
                    contains(&notes, needle),
                    "{}: notes.rs must declare `{needle}`; got:\n{notes}",
                    app.kit
                );
            }
            let store = function(&notes, "store");
            for needle in [
                "form: StoreNoteRequest",
                "user_id: user.id,",
                r#"Inertia::flash("toast", Toast::success("Note saved."))?;"#,
                r#"redirect!("/notes")"#,
            ] {
                assert!(
                    contains(store, needle),
                    "{}: the notes store handler must contain `{needle}`; got:\n{store}",
                    app.kit
                );
            }
        });
    }

    /// PAR-080: `POST /profile/name` reads JSON, validates the name with
    /// the framework's validation, saves it on the signed-in user and
    /// answers that user as JSON.
    #[test]
    fn kit_profile_name_validates_and_answers_the_user_as_json() {
        each_kit(|app| {
            let profile = app.read("src/controllers/profile.rs");
            for needle in [
                "pub struct UpdateNameRequest {",
                "#[validate(length(min = 1, max = 255,",
                "impl FormRequest for UpdateNameRequest",
            ] {
                assert!(
                    contains(&profile, needle),
                    "{}: profile.rs must contain `{needle}`; got:\n{profile}",
                    app.kit
                );
            }
            let update = function(&profile, "update_name");
            for needle in [
                "form: UpdateNameRequest",
                "Auth::user_as::<User>()",
                ".update(attrs! {",
                "HttpResponse::json(",
                r#""user": UserInfo::from(user)"#,
            ] {
                assert!(
                    contains(update, needle),
                    "{}: the name handler must contain `{needle}`; got:\n{update}",
                    app.kit
                );
            }
        });
    }

    /// PAR-078: one flash struct, marked for the generator, holding the
    /// toast, and a toast flashed after each of the eight actions the
    /// requirement names.
    #[test]
    fn kit_flash_struct_is_marked_and_every_action_flashes_a_toast() {
        each_kit(|app| {
            let lib = app.read("src/lib.rs");
            assert!(
                contains(&lib, "pub mod props;"),
                "{}: lib.rs must declare the props module; got:\n{lib}",
                app.kit
            );
            let props = app.read("src/props/mod.rs");
            assert!(
                contains(&props, "pub mod flash;"),
                "{}: props/mod.rs must declare the flash module; got:\n{props}",
                app.kit
            );
            let flash = app.read("src/props/flash.rs");
            assert_eq!(
                flash
                    .lines()
                    .filter(|line| line.trim() == "#[inertia_props(flash)]")
                    .count(),
                1,
                "{}: one struct is the flash data; got:\n{flash}",
                app.kit
            );
            for needle in [
                "#[derive(InertiaProps)]\n#[inertia_props(flash)]\npub struct Flash {",
                "pub toast: Option<Toast>,",
                "#[derive(InertiaProps)]\npub struct Toast {",
                "pub kind: String,",
                "pub message: String,",
                "pub fn success(",
                "pub fn info(",
                "pub fn error(",
            ] {
                assert!(
                    contains(&flash, needle),
                    "{}: props/flash.rs must contain `{needle}`; got:\n{flash}",
                    app.kit
                );
            }

            let controllers = app.controllers();
            let body_of = |file: &str| -> &str {
                controllers
                    .iter()
                    .find(|(name, _)| name == file)
                    .map(|(_, body)| body.as_str())
                    .unwrap_or_else(|| panic!("{}: no src/controllers/{file}", app.kit))
            };
            for (file, function_name, call) in [
                (
                    "auth.rs",
                    "login",
                    r#"Inertia::flash("toast", Toast::success("Signed in."))?;"#,
                ),
                (
                    "auth.rs",
                    "register",
                    r#"Inertia::flash("toast", Toast::success(format!("Welcome, {}.", form.name)))?;"#,
                ),
                (
                    "auth.rs",
                    "logout",
                    r#"Inertia::flash("toast", Toast::info("Signed out."))?;"#,
                ),
                (
                    "password_reset.rs",
                    "send_link",
                    r#"Toast::info("If that address is registered, a reset link is on its way.")"#,
                ),
                (
                    "password_reset.rs",
                    "reset",
                    r#"Inertia::flash("toast", Toast::success("Your password was reset."))?;"#,
                ),
                (
                    "email_verification.rs",
                    "verify",
                    r#"Inertia::flash("toast", Toast::success("Your email address is verified."))?;"#,
                ),
                (
                    "email_verification.rs",
                    "resend",
                    r#"Toast::info("A new verification link is on its way.")"#,
                ),
                (
                    "notes.rs",
                    "store",
                    r#"Inertia::flash("toast", Toast::success("Note saved."))?;"#,
                ),
            ] {
                let body = function(body_of(file), function_name);
                assert!(
                    contains(body, call),
                    "{}: `{function_name}` in {file} must flash `{call}`; got:\n{body}",
                    app.kit
                );
            }
            let flashes: usize = controllers
                .iter()
                .map(|(_, body)| {
                    let body: String = body.split_whitespace().collect();
                    body.matches(r#"Inertia::flash("toast","#).count()
                })
                .sum();
            assert_eq!(
                flashes, 8,
                "{}: the scaffold flashes exactly the eight toasts PAR-078 names",
                app.kit
            );
        });
    }

    /// PAR-077 and PAR-080: every page carries the signed-in user, or
    /// none, under the shared `auth` prop, resolved for each response, and
    /// the struct that types it is marked for the generator.
    #[test]
    fn kit_shared_auth_carries_the_signed_in_user_on_every_page() {
        each_kit(|app| {
            let props = app.read("src/props/mod.rs");
            assert!(
                contains(&props, "pub mod shared;"),
                "{}: props/mod.rs must declare the shared module; got:\n{props}",
                app.kit
            );
            let shared = app.read("src/props/shared.rs");
            for needle in [
                "#[derive(InertiaProps)]\n#[inertia_props(shared)]\npub struct SharedData {",
                "pub auth: Auth,",
                "#[derive(InertiaProps)]\npub struct Auth {",
                "pub user: Option<UserInfo>,",
                "#[derive(InertiaProps)]\npub struct UserInfo {",
                "pub async fn current() -> Result<Self, FrameworkError>",
                "suprnova::Auth::user_as::<User>()",
            ] {
                assert!(
                    contains(&shared, needle),
                    "{}: props/shared.rs must contain `{needle}`; got:\n{shared}",
                    app.kit
                );
            }
            let bootstrap = app.read("src/bootstrap.rs");
            let http_stack = function(&bootstrap, "register_http_stack");
            assert!(
                contains(
                    http_stack,
                    r#"App::inertia_share_lazy("auth", crate::props::shared::Auth::current);"#
                ),
                "{}: the HTTP stack must share `auth` per response; got:\n{http_stack}",
                app.kit
            );
        });
    }

    /// PAR-079's last sentence: no handler lists accounts, and no handler
    /// reads a note except through the signed-in user's own notes.
    #[test]
    fn kit_no_handler_lists_accounts_or_reads_another_users_notes() {
        each_kit(|app| {
            for (file, body) in app.controllers() {
                for listing in [
                    "<User as Model>::query()",
                    "<User as suprnova::eloquent::Model>::query()",
                    "User::query()",
                    "User::all(",
                ] {
                    assert!(
                        !contains(&body, listing),
                        "{}: {file} must not list accounts (`{listing}`); got:\n{body}",
                        app.kit
                    );
                }
                for unscoped in ["<Note as Model>::query()", "Note::query()", "Note::find("] {
                    assert!(
                        !contains(&body, unscoped),
                        "{}: {file} must read notes only through `Note::owned_by` \
                         (found `{unscoped}`); got:\n{body}",
                        app.kit
                    );
                }
            }
        });
    }

    /// PAR-078: `suprnova generate-types` on a fresh scaffold names the
    /// flash struct as `flashDataType`, adds `auth` to the shared props,
    /// types the note pages, and warns about no prop type.
    #[test]
    fn kit_generated_types_name_the_flash_struct_and_the_note_pages() {
        each_kit(|app| {
            let output = Command::new(BIN)
                .arg("generate-types")
                .current_dir(&app.root)
                .output()
                .expect("spawn suprnova generate-types");
            let printed = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.status.success(),
                "{}: generate-types failed:\n{printed}",
                app.kit
            );
            assert!(
                !printed.contains("isn't a struct this project defines"),
                "{}: a fresh scaffold must not warn about a prop type:\n{printed}",
                app.kit
            );
            let types = app.read("frontend/src/types/inertia-props.ts");
            for needle in [
                "export interface Flash {\n  toast: Toast | null;\n}",
                "export interface Toast {\n  kind: string;\n  message: string;\n}",
                "    flashDataType: Flash;",
                "export interface SharedProps {\n  root: string;\n  auth: Auth;\n}",
                "export interface Auth {\n  user: UserInfo | null;\n}",
                "export interface UserInfo {\n  id: number;\n  name: string;\n  email: string;\n}",
                "export interface DashboardProps {\n  stats: Stats;\n  recent_notes: Array<NoteSummary>;\n}",
                "export interface Stats {\n  notes: number;\n  written_today: number;\n}",
                "export interface NoteSummary {\n  id: number;\n  title: string;\n  created_at: string | null;\n}",
                "export interface NotesIndexProps {\n  notes: Array<NoteSummary>;\n  search: string;\n}",
                "export interface NoteView {\n  id: number;\n  title: string;\n  body: string | null;\n  created_at: string | null;\n}",
                "export interface NotesShowProps {\n  note: NoteView;\n}",
                r#"  "Notes/Show": NotesShowProps;"#,
            ] {
                assert!(
                    types.contains(needle),
                    "{}: the generated types must contain `{needle}`; got:\n{types}",
                    app.kit
                );
            }
        });
    }

    /// PAR-076: the scaffold tests hold every kit's `@inertiajs/*` pin at
    /// 3.8.0, the version the kits' manifests declare.
    #[test]
    fn kit_scaffold_snapshot_expects_the_inertia_3_8_pin() {
        let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scaffold_snapshot.rs");
        let source = std::fs::read_to_string(&snapshot)
            .unwrap_or_else(|e| panic!("read {}: {e}", snapshot.display()));
        assert!(
            source.contains(r#"const EXPECTED_INERTIA_PIN: &str = "^3.8.0";"#),
            "scaffold_snapshot.rs must expect the `^3.8.0` pin"
        );
    }

mod svelte {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Scaffold a svelte project into `tmp` and return its `frontend/`.
    fn scaffold(tmp: &tempfile::TempDir) -> PathBuf {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .args([
                "new",
                "kitsvelte",
                "--no-interaction",
                "--no-git",
                "--frontend",
                "svelte",
            ])
            .current_dir(tmp.path())
            .output()
            .expect("run `suprnova new`");
        assert!(
            out.status.success(),
            "`suprnova new --frontend svelte` failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        tmp.path().join("kitsvelte/frontend")
    }

    fn read(frontend: &Path, rel: &str) -> String {
        fs::read_to_string(frontend.join(rel))
            .unwrap_or_else(|e| panic!("read frontend/{rel}: {e}"))
    }

    /// Every `.svelte` file under `frontend/src/<dir>`, as its path relative
    /// to that directory and its text, sorted by path.
    fn svelte_files(frontend: &Path, dir: &str) -> Vec<(String, String)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
            for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else if path.extension().is_some_and(|ext| ext == "svelte") {
                    let rel = path
                        .strip_prefix(root)
                        .expect("under root")
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((rel, fs::read_to_string(&path).expect("read svelte file")));
                }
            }
        }
        let root = frontend.join("src").join(dir);
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    /// The lines of a script or a template that are not comments.
    fn code_lines(body: &str) -> Vec<&str> {
        body.lines()
            .filter(|line| {
                let line = line.trim_start();
                !["//", "/*", "*", "<!--"]
                    .iter()
                    .any(|start| line.starts_with(start))
            })
            .collect()
    }

    /// `body` with every run of whitespace turned into one space, so a check
    /// reads a tag the same whether its attributes share a line or not.
    fn flatten(body: &str) -> String {
        body.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The text of every `<a ...>` opening tag in `body`.
    fn anchor_tags(body: &str) -> Vec<&str> {
        let mut tags = Vec::new();
        let mut rest = body;
        while let Some(at) = rest.find("<a") {
            let after = &rest[at + 2..];
            if after.starts_with(|c: char| c.is_whitespace() || c == '>') {
                let end = after.find('>').map_or(after.len(), |end| end + 1);
                tags.push(&rest[at..at + 2 + end]);
            }
            rest = after;
        }
        tags
    }

    #[test]
    fn kit_svelte_anchor_reader_sees_every_anchor_and_nothing_else() {
        assert_eq!(
            anchor_tags("<a href={`${root}/x`}>x</a> <abbr>y</abbr> <a\n  href=\"https://e.x\">"),
            vec!["<a href={`${root}/x`}>", "<a\n  href=\"https://e.x\">"]
        );
        assert!(anchor_tags("<Link href={`${root}/x`}>x</Link>").is_empty());
    }

    #[test]
    fn kit_svelte_manifest_pins_inertia_3_8_and_the_vite_plugin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let manifest: serde_json::Value =
            serde_json::from_str(&read(&frontend, "package.json")).expect("package.json parses");
        for package in ["@inertiajs/svelte", "@inertiajs/core", "@inertiajs/vite"] {
            let pin = manifest["dependencies"][package]
                .as_str()
                .or_else(|| manifest["devDependencies"][package].as_str());
            assert_eq!(
                pin,
                Some("^3.8.0"),
                "package.json must declare {package} at ^3.8.0; got {manifest}"
            );
        }
        let check = manifest["scripts"]["check"].as_str().unwrap_or("");
        assert!(
            check.contains("svelte-check"),
            "package.json must have a `check` script that runs svelte-check; got {check:?}"
        );
        let build_ssr = manifest["scripts"]["build:ssr"].as_str().unwrap_or("");
        assert_eq!(build_ssr, "vite build --ssr src/ssr.ts");

        let vite = read(&frontend, "vite.config.ts");
        assert!(
            vite.contains("import inertia from '@inertiajs/vite'"),
            "vite.config.ts must import the Inertia plugin:\n{vite}"
        );
        assert!(
            vite.contains("inertia({ ssr: { entry: 'src/ssr.ts', sourcemap: false } })"),
            "vite.config.ts must register the plugin on the SSR entry `build:ssr` builds:\n{vite}"
        );
        for kept in [
            "base: './'",
            "outDir: 'bootstrap/ssr'",
            "entryFileNames: 'ssr.js'",
        ] {
            assert!(vite.contains(kept), "vite.config.ts lost `{kept}`:\n{vite}");
        }
    }

    #[test]
    fn kit_svelte_entries_resolve_pages_through_the_plugin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        for entry in ["src/main.ts", "src/ssr.ts"] {
            let body = read(&frontend, entry);
            assert!(
                body.contains("pages: './pages',"),
                "{entry} must resolve pages through the plugin's `pages` shorthand:\n{body}"
            );
            assert!(
                !body.contains("import.meta.glob"),
                "{entry} must not glob the pages itself:\n{body}"
            );
        }
        // The plugin wraps a top-level `createInertiaApp(...)` statement in
        // an SSR build and adds the server itself
        // (packages/vite/src/ssrTransform.ts), so the SSR entry starts none.
        let ssr = read(&frontend, "src/ssr.ts");
        let code = code_lines(&ssr);
        assert!(
            code.iter()
                .any(|line| line.starts_with("createInertiaApp({")),
            "src/ssr.ts must be the top-level `createInertiaApp` statement the plugin wraps:\n{ssr}"
        );
        assert!(
            !code.iter().any(|line| line.contains("createServer")),
            "src/ssr.ts must leave `createServer` to the plugin:\n{ssr}"
        );
    }

    #[test]
    fn kit_svelte_entries_pass_title_server_head_nonce_and_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let main = read(&frontend, "src/main.ts");
        for option in [
            "title: pageTitle,",
            "serverHead: true,",
            "nonce,",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),",
        ] {
            assert!(
                main.contains(option),
                "src/main.ts must pass `{option}`:\n{main}"
            );
        }
        assert!(
            main.contains(
                "document.querySelector<HTMLMetaElement>('meta[property=\"csp-nonce\"]')?.nonce"
            ),
            "src/main.ts must read the nonce from the csp-nonce meta element:\n{main}"
        );
        let ssr = read(&frontend, "src/ssr.ts");
        for option in [
            "title: pageTitle,",
            "serverHead: true,",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),",
        ] {
            assert!(
                ssr.contains(option),
                "src/ssr.ts must pass `{option}`:\n{ssr}"
            );
        }
        let title = read(&frontend, "src/lib/title.ts");
        assert!(
            title.contains("export const appName = 'Kitsvelte'"),
            "src/lib/title.ts must name the application as the server titles it:\n{title}"
        );
    }

    #[test]
    fn kit_svelte_layouts_exist_with_their_chrome() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        // Both layouts show the account links: VerifyEmail sits under
        // `auth/` and is a signed-in page, so the guest frame needs the
        // signed-in branch as much as the application frame does.
        let guest = flatten(&read(&frontend, "src/layouts/GuestLayout.svelte"));
        for part in [
            "<AccountLinks />",
            "<FlashToast />",
            "{@render children?.()}",
        ] {
            assert!(
                guest.contains(part),
                "GuestLayout.svelte lacks `{part}`:\n{guest}"
            );
        }
        let app = flatten(&read(&frontend, "src/layouts/AppLayout.svelte"));
        for part in [
            "<Link href={`${root}/dashboard`}",
            "<Link href={`${root}/notes`}",
            "<AccountLinks />",
            "<FlashToast />",
            "{#if heading}",
            "{@render children?.()}",
        ] {
            assert!(
                app.contains(part),
                "AppLayout.svelte lacks `{part}`:\n{app}"
            );
        }
        let links = flatten(&read(&frontend, "src/components/AccountLinks.svelte"));
        for part in [
            "{#if user}",
            "{user.name}",
            "<Link href={`${root}/logout`} method=\"post\" as=\"button\" preserveState={false}",
            "{:else}",
            "<Link href={`${root}/login`}",
            "<Link href={`${root}/register`}",
        ] {
            assert!(
                links.contains(part),
                "AccountLinks.svelte lacks `{part}`:\n{links}"
            );
        }
    }

    #[test]
    fn kit_svelte_no_page_or_layout_links_an_application_route_with_an_anchor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let mut offenders = Vec::new();
        let mut files = 0usize;
        for dir in ["pages", "layouts", "components"] {
            for (rel, body) in svelte_files(&frontend, dir) {
                files += 1;
                for tag in anchor_tags(&body) {
                    let external = ["href=\"https://", "href='https://", "href={'https://"]
                        .iter()
                        .any(|href| tag.contains(href));
                    if !external {
                        offenders.push(format!("{dir}/{rel}: {tag}"));
                    }
                }
            }
        }
        assert!(files >= 14, "walked only {files} svelte files");
        assert!(
            offenders.is_empty(),
            "an application route is reached through an anchor, not a `Link`:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn kit_svelte_every_page_sets_its_title_with_head() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let pages = svelte_files(&frontend, "pages");
        let names: Vec<&str> = pages.iter().map(|(rel, _)| rel.as_str()).collect();
        assert_eq!(
            names,
            [
                "Dashboard.svelte",
                "Error.svelte",
                "Home.svelte",
                "Notes/Index.svelte",
                "Notes/Show.svelte",
                "auth/ForgotPassword.svelte",
                "auth/Login.svelte",
                "auth/Register.svelte",
                "auth/ResetPassword.svelte",
                "auth/VerifyEmail.svelte",
            ]
        );
        for (rel, body) in &pages {
            assert!(
                body.contains("components/Head.svelte'") && body.contains("<Head"),
                "pages/{rel} must set its title with `Head`:\n{body}"
            );
        }
        let head = read(&frontend, "src/components/Head.svelte");
        assert!(
            head.contains("<svelte:head>") && head.contains("<title>{pageTitle(title)}</title>"),
            "components/Head.svelte must title the document through `pageTitle`:\n{head}"
        );
    }

    #[test]
    fn kit_svelte_auth_pages_submit_through_the_form_component() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        for (page, action) in [
            ("Login", "login"),
            ("Register", "register"),
            ("ForgotPassword", "forgot-password"),
            ("ResetPassword", "reset-password"),
            ("VerifyEmail", "email/verification-notification"),
        ] {
            let body = read(&frontend, &format!("src/pages/auth/{page}.svelte"));
            let flat = flatten(&body);
            assert!(
                flat.contains(&format!(
                    "<Form action={{`${{root}}/{action}`}} method=\"post\""
                )),
                "auth/{page} must submit through `<Form action={{`${{root}}/{action}`}} method=\"post\">`:\n{body}"
            );
            assert!(
                body.contains("{#snippet children({ errors, processing })}"),
                "auth/{page} must read `errors` and `processing` from the form:\n{body}"
            );
            assert!(
                body.contains("disabled={processing}"),
                "auth/{page} must disable its button while processing:\n{body}"
            );
            assert!(
                !body.contains("useForm") && !body.contains("onsubmit"),
                "auth/{page} must not submit with `useForm` or a submit handler:\n{body}"
            );
        }
        let login = read(&frontend, "src/pages/auth/Login.svelte");
        assert!(
            login.contains("name=\"remember\"") && login.contains("type=\"checkbox\""),
            "auth/Login keeps the remember checkbox:\n{login}"
        );
    }

    #[test]
    fn kit_svelte_dashboard_uses_the_client_components() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let body = flatten(&read(&frontend, "src/pages/Dashboard.svelte"));
        for part in [
            "<Deferred data=\"stats\">",
            "{#snippet fallback()}",
            "<WhenVisible data=\"recent_notes\">",
            "usePoll(10000, { only: ['stats'] })",
            "useHttp(",
            "`${root}/profile/name`",
            "page.props.auth.user",
            "router.replaceProp('auth.user.name', ",
            "router.reload({ only: ['auth'] })",
            "errors.name",
            ": DashboardProps = $props()",
            "setLayoutProps({ heading: 'Dashboard' })",
        ] {
            assert!(
                body.contains(part),
                "Dashboard.svelte lacks `{part}`:\n{body}"
            );
        }
    }

    #[test]
    fn kit_svelte_notes_pages_use_the_client_components() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let index = flatten(&read(&frontend, "src/pages/Notes/Index.svelte"));
        for part in [
            "<Form action={`${root}/notes`} method=\"post\"",
            "name=\"title\"",
            "name=\"body\"",
            "useRemember(",
            "router.get(`${root}/notes`, { search: filters.search }",
            "reset: ['notes']",
            "<InfiniteScroll data=\"notes\"",
            "href={`${root}/notes/${note.id}`}",
            "prefetch",
            "component=\"Notes/Show\"",
            "pageProps={(_props, shared) => ({ ...shared, note })}",
            ": NotesIndexProps = $props()",
        ] {
            assert!(
                index.contains(part),
                "Notes/Index.svelte lacks `{part}`:\n{index}"
            );
        }
        let show = read(&frontend, "src/pages/Notes/Show.svelte");
        assert!(
            show.contains(": NotesShowProps = $props()") && show.contains("{note.title}"),
            "Notes/Show.svelte must render the note it is given:\n{show}"
        );
    }

    #[test]
    fn kit_svelte_flash_toast_renders_the_page_flash() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let toast = read(&frontend, "src/components/FlashToast.svelte");
        assert!(
            toast.contains("page.flash.toast"),
            "FlashToast.svelte must read `page.flash.toast`:\n{toast}"
        );
        let code = code_lines(&toast).join("\n");
        assert!(
            !code.contains("$state") && !code.contains("useRemember"),
            "FlashToast.svelte must render the flash from the page, never store it:\n{toast}"
        );
        let types = read(&frontend, "src/types/inertia-props.ts");
        assert!(
            types.contains("    flashDataType: "),
            "the generated types must name the flash data type:\n{types}"
        );
    }

    /// The structs the kit's pages are typed from, as the scaffold's
    /// backend declares them for PAR-078 to PAR-080: the flash data, the
    /// shared `auth`, the dashboard and the notes pages. They go into
    /// `src/controllers/` over the scaffold's own, and `generate-types` runs
    /// on the result. Once the scaffold ships these structs itself, this
    /// list repeats or replaces them and the test fails until it is gone.
    const KIT_BACKEND: [(&str, &str); 4] = [
        (
            "dashboard.rs",
            include_str!("fixtures/kit_svelte/dashboard.rs"),
        ),
        ("notes.rs", include_str!("fixtures/kit_svelte/notes.rs")),
        ("flash.rs", include_str!("fixtures/kit_svelte/flash.rs")),
        ("shared.rs", include_str!("fixtures/kit_svelte/shared.rs")),
    ];

    #[test]
    fn kit_svelte_types_are_what_generate_types_writes_for_the_kit_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let project = frontend.parent().expect("project").to_path_buf();
        let shipped = read(&frontend, "src/types/inertia-props.ts");
        for (file, body) in KIT_BACKEND {
            fs::write(project.join("src/controllers").join(file), body)
                .unwrap_or_else(|e| panic!("write src/controllers/{file}: {e}"));
        }
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .arg("generate-types")
            .current_dir(&project)
            .output()
            .expect("run `suprnova generate-types`");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "generate-types failed: {text}");
        assert!(
            text.contains("./frontend/src/types/inertia-props.ts is up to date"),
            "the shipped types differ from what generate-types writes: {text}"
        );
        assert_eq!(read(&frontend, "src/types/inertia-props.ts"), shipped);
        for declaration in [
            "export interface SharedProps {\n  root: string;\n  auth: Auth;\n}",
            "    flashDataType: Flash;",
            "  \"Notes/Index\": NotesIndexProps;",
            "  \"Notes/Show\": NotesShowProps;",
        ] {
            assert!(
                shipped.contains(declaration),
                "the shipped types lack `{declaration}`:\n{shipped}"
            );
        }
    }
}

/// The React kit: `suprnova new <name> --frontend react`.
mod react {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

    /// The auth pages, by component name. Every one submits through `Form`.
    const AUTH_PAGES: [&str; 5] = [
        "auth/Login",
        "auth/Register",
        "auth/ForgotPassword",
        "auth/ResetPassword",
        "auth/VerifyEmail",
    ];

    /// Every page the kit ships, by component name.
    const PAGES: [&str; 10] = [
        "Home",
        "Dashboard",
        "Notes/Index",
        "Notes/Show",
        "Error",
        "auth/Login",
        "auth/Register",
        "auth/ForgotPassword",
        "auth/ResetPassword",
        "auth/VerifyEmail",
    ];

    /// A scaffolded React project: the temporary directory that owns it and
    /// its `frontend/` directory.
    struct Kit {
        _tmp: tempfile::TempDir,
        frontend: PathBuf,
    }

    impl Kit {
        /// Run `suprnova new kit-react --frontend react` in a fresh directory.
        fn scaffold() -> Kit {
            let tmp = tempfile::tempdir().expect("tempdir");
            let output = Command::new(BIN)
                .args([
                    "new",
                    "kit-react",
                    "--no-interaction",
                    "--no-git",
                    "--frontend",
                    "react",
                ])
                .current_dir(tmp.path())
                .output()
                .expect("spawn suprnova new");
            assert!(
                output.status.success(),
                "`suprnova new kit-react --frontend react` failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let frontend = tmp.path().join("kit-react/frontend");
            Kit {
                _tmp: tmp,
                frontend,
            }
        }

        /// The text of `frontend/<rel>`.
        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.frontend.join(rel))
                .unwrap_or_else(|e| panic!("read frontend/{rel}: {e}"))
        }

        /// The text of the page a handler renders as `component`.
        fn page(&self, component: &str) -> String {
            self.read(&format!("src/pages/{component}.tsx"))
        }

        /// Every `.tsx` file under `frontend/src/<dir>`, with its text.
        fn sources(&self, dir: &str) -> Vec<(PathBuf, String)> {
            let mut out = Vec::new();
            collect_tsx(&self.frontend.join("src").join(dir), &mut out);
            out
        }
    }

    fn collect_tsx(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_tsx(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "tsx") {
                let body = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                out.push((path, body));
            }
        }
    }

    /// Every `href` value of an `<a>` element in `source`, as written.
    fn anchor_hrefs(source: &str) -> Vec<String> {
        let mut hrefs = Vec::new();
        for (at, _) in source.match_indices("<a") {
            let rest = &source[at + 2..];
            // `<a` opens an anchor only when a space, a line end or `>`
            // follows; `<abbr>` and `<article>` are other elements.
            if !rest.starts_with([' ', '\n', '\r', '\t', '>']) {
                continue;
            }
            let tag = rest.split_once('>').map_or(rest, |(tag, _)| tag);
            if let Some((_, value)) = tag.split_once("href=") {
                hrefs.push(value.split_whitespace().next().unwrap_or("").to_owned());
            }
        }
        hrefs
    }

    #[test]
    fn kit_react_anchor_reader_sees_every_form() {
        assert_eq!(
            anchor_hrefs("<a href={`${root}/`}>Home</a>"),
            ["{`${root}/`}"]
        );
        assert_eq!(
            anchor_hrefs("<a\n  className=\"x\"\n  href=\"https://inertiajs.com\"\n>"),
            ["\"https://inertiajs.com\""]
        );
        assert!(anchor_hrefs("<abbr title=\"x\">x</abbr><article></article>").is_empty());
        assert!(anchor_hrefs("<Link href={`${root}/notes`}>Notes</Link>").is_empty());
    }

    /// PAR-076: the manifest declares the adapter, `@inertiajs/core` and
    /// `@inertiajs/vite` at `^3.8.0` and a `check` script, and the Vite
    /// config runs the plugin with the SSR bundle where `ssr:start` reads it.
    #[test]
    fn kit_react_pins_inertia_3_8_and_the_vite_plugin() {
        let kit = Kit::scaffold();
        let manifest: serde_json::Value =
            serde_json::from_str(&kit.read("package.json")).expect("package.json parses");
        for package in ["@inertiajs/react", "@inertiajs/core"] {
            assert_eq!(
                manifest["dependencies"][package], "^3.8.0",
                "{package} must be a dependency at ^3.8.0: {manifest}"
            );
        }
        let vite = manifest["dependencies"]["@inertiajs/vite"]
            .as_str()
            .or_else(|| manifest["devDependencies"]["@inertiajs/vite"].as_str());
        assert_eq!(
            vite,
            Some("^3.8.0"),
            "@inertiajs/vite at ^3.8.0: {manifest}"
        );
        let check = manifest["scripts"]["check"].as_str().unwrap_or("");
        assert!(
            check.contains("tsc"),
            "the kit needs a `check` script that type-checks it: {manifest}"
        );
        assert_eq!(
            manifest["scripts"]["build:ssr"], "vite build --ssr src/ssr.tsx",
            "{manifest}"
        );

        let config = kit.read("vite.config.ts");
        for needle in [
            "import inertia from '@inertiajs/vite'",
            "inertia({ ssr: 'src/ssr.tsx' })",
            "base: './',",
            "outDir: 'bootstrap/ssr'",
            "entryFileNames: 'ssr.js'",
            "outDir: '../public/assets'",
            "input: 'src/main.tsx'",
            "port: Number(process.env.VITE_PORT) || 5765",
        ] {
            assert!(
                config.contains(needle),
                "vite.config.ts lacks {needle}:\n{config}"
            );
        }
        assert!(
            !config.contains("ssr_start.rs") && config.contains("frontend/bootstrap/ssr/ssr.js"),
            "vite.config.ts cites `suprnova ssr:start` and its bundle path, not a source file:\n{config}"
        );

        for (path, body) in kit.sources("").into_iter().chain([
            (kit.frontend.join("package.json"), kit.read("package.json")),
            (kit.frontend.join("vite.config.ts"), config.clone()),
            (
                kit.frontend.join("src/lib/app.ts"),
                kit.read("src/lib/app.ts"),
            ),
        ]) {
            assert!(
                !body.contains("3.6.1"),
                "{} still cites Inertia 3.6.1",
                path.display()
            );
        }
    }

    /// PAR-076: both entries resolve pages through the plugin's `pages`
    /// shorthand and pass the title callback, the server head and the
    /// layout; the browser entry passes the document's CSP nonce.
    #[test]
    fn kit_react_entries_resolve_pages_through_the_plugin() {
        let kit = Kit::scaffold();
        let main = kit.read("src/main.tsx");
        let ssr = kit.read("src/ssr.tsx");
        for (file, body) in [("main.tsx", &main), ("ssr.tsx", &ssr)] {
            assert!(
                !body.contains("import.meta.glob"),
                "{file} resolves pages with its own glob:\n{body}"
            );
            for needle in [
                "createInertiaApp({",
                "pages: './pages',",
                "title,",
                "layout,",
                "serverHead: true,",
                "import { layout, title } from './lib/app'",
                "<LangProvider>",
            ] {
                assert!(body.contains(needle), "{file} lacks {needle}:\n{body}");
            }
        }
        assert!(
            main.contains(
                "document.querySelector<HTMLMetaElement>('meta[property=\"csp-nonce\"]')"
            ) && main.contains("nonce,"),
            "main.tsx must pass the nonce of <meta property=\"csp-nonce\">:\n{main}"
        );
        assert!(
            main.contains("initLang(props.initialPage)"),
            "main.tsx loads the catalog before the first render:\n{main}"
        );
        assert!(
            !ssr.contains("createServer(") && !ssr.contains("initLang("),
            "the plugin wraps ssr.tsx's createInertiaApp call in createServer, \
             and the SSR pass loads no catalog:\n{ssr}"
        );
        for comment in ["XSRF-TOKEN", "hydration content mismatch"] {
            assert!(
                main.contains(comment),
                "main.tsx lost the {comment} comment"
            );
        }

        let app = kit.read("src/lib/app.ts");
        for needle in [
            "export const appName = 'Kit React'",
            "title ? `${title} - ${appName}` : appName",
            "name.startsWith('auth/') ? GuestLayout : AppLayout",
        ] {
            assert!(
                app.contains(needle),
                "src/lib/app.ts lacks {needle}:\n{app}"
            );
        }
    }

    /// PAR-077 and PAR-078: the guest and application layouts, and the flash
    /// toast both show, read from the page and never kept in state. Both
    /// layouts render `AccountLinks`, which reads the signed-in user from the
    /// shared `auth` prop: the sign-in and register links when there is
    /// none, the name and the sign-out `Link` when there is one.
    #[test]
    fn kit_react_ships_both_layouts_and_the_flash_toast() {
        let kit = Kit::scaffold();
        let app = kit.read("src/layouts/AppLayout.tsx");
        let guest = kit.read("src/layouts/GuestLayout.tsx");
        for needle in [
            "href={`${root}/dashboard`}",
            "href={`${root}/notes`}",
            "heading",
        ] {
            assert!(app.contains(needle), "AppLayout lacks {needle}:\n{app}");
        }
        for (layout, body) in [("AppLayout", &app), ("GuestLayout", &guest)] {
            for needle in ["<AccountLinks", "<FlashToast />"] {
                assert!(body.contains(needle), "{layout} lacks {needle}:\n{body}");
            }
            assert!(
                !body.contains("user?:"),
                "{layout} takes the user from the shared `auth` prop, not from a page prop:\n{body}"
            );
        }
        let account = kit.read("src/components/AccountLinks.tsx");
        for needle in [
            "const { root, auth } = usePage().props",
            "auth?.user",
            "user ? (",
            "{user.name}",
            "href={`${root}/login`}",
            "href={`${root}/register`}",
            // A `Link` that posts preserves the page's state by default, and
            // a visit that preserves state keeps the layout props: signing
            // out from the dashboard would carry its heading onto the next
            // page (`packages/react/src/Link.ts` and `App.ts` in Inertia
            // 3.8.0).
            "<Link href={`${root}/logout`} method=\"post\" as=\"button\" preserveState={false}",
        ] {
            assert!(
                account.contains(needle),
                "AccountLinks lacks {needle}:\n{account}"
            );
        }

        let toast = kit.read("src/components/FlashToast.tsx");
        for needle in ["const { toast } = usePage().flash", "toast.message"] {
            assert!(
                toast.contains(needle),
                "FlashToast lacks {needle}:\n{toast}"
            );
        }
        for refused in [
            "useState",
            "useRemember",
            "useEffect",
            "'toast' in",
            "object",
        ] {
            assert!(
                !toast.contains(refused),
                "FlashToast must read the typed toast from the page, not keep or probe it \
                 ({refused}):\n{toast}"
            );
        }

        // The layouts come from `createInertiaApp`'s `layout` option, so a
        // page never renders one itself; a page that did would remount it on
        // every visit.
        for (path, body) in kit.sources("pages") {
            assert!(
                !body.contains("layouts/"),
                "{} renders a layout itself",
                path.display()
            );
        }
    }

    /// PAR-077: every in-application navigation is a `Link`; an anchor
    /// stays only for an external `https://` URL.
    #[test]
    fn kit_react_never_links_an_application_route_with_an_anchor() {
        let kit = Kit::scaffold();
        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        for dir in ["pages", "layouts", "components"] {
            for (path, body) in kit.sources(dir) {
                scanned += 1;
                for href in anchor_hrefs(&body) {
                    if !(href.starts_with("\"https://") || href.starts_with("'https://")) {
                        offenders.push(format!("{}: href={href}", path.display()));
                    }
                }
            }
        }
        assert_eq!(
            scanned, 14,
            "ten pages, two layouts, the account links and the toast"
        );
        assert!(
            offenders.is_empty(),
            "anchors to application routes:\n{}",
            offenders.join("\n")
        );
    }

    /// PAR-077: every page sets its title with `Head`.
    #[test]
    fn kit_react_titles_every_page_with_head() {
        let kit = Kit::scaffold();
        let pages = kit.sources("pages");
        assert_eq!(
            pages.len(),
            PAGES.len(),
            "the kit ships exactly these pages: {PAGES:?}"
        );
        for component in PAGES {
            let body = kit.page(component);
            assert!(
                body.contains("<Head title=")
                    && body.contains("Head")
                    && body.contains("from '@inertiajs/react'"),
                "{component} sets no title with Head:\n{body}"
            );
        }
    }

    /// PAR-079: every auth page submits through the `Form` component and
    /// reads `errors` and `processing` from it, never `useForm`.
    #[test]
    fn kit_react_auth_pages_submit_through_the_form_component() {
        let kit = Kit::scaffold();
        for (component, action) in AUTH_PAGES.into_iter().zip([
            "/login",
            "/register",
            "/forgot-password",
            "/reset-password",
            "/email/verification-notification",
        ]) {
            let body = kit.page(component);
            for needle in [
                format!("action={{`${{root}}{action}`}}"),
                "method=\"post\"".to_owned(),
                "<Form".to_owned(),
                "({ errors, processing }) =>".to_owned(),
                "disabled={processing}".to_owned(),
            ] {
                assert!(
                    body.contains(&needle),
                    "{component} lacks {needle}:\n{body}"
                );
            }
            for refused in ["useForm", "onSubmit", "preventDefault"] {
                assert!(
                    !body.contains(refused),
                    "{component} submits with {refused}, not the Form component:\n{body}"
                );
            }
        }
        let login = kit.page("auth/Login");
        assert!(
            login.contains("name=\"remember\"")
                && login.contains("remember: Boolean(data.remember)"),
            "Login keeps the remember checkbox and sends it as a boolean:\n{login}"
        );
        let reset = kit.page("auth/ResetPassword");
        assert!(
            reset.contains("<input type=\"hidden\" name=\"token\" value={token} />"),
            "ResetPassword sends the mailed token back in the form body:\n{reset}"
        );
    }

    /// PAR-077 and PAR-079: the dashboard defers `stats`, loads
    /// `recent_notes` when scrolled into view, polls `stats`, changes the
    /// display name through `useHttp` optimistically, and sets the layout's
    /// heading.
    #[test]
    fn kit_react_dashboard_uses_deferred_when_visible_poll_and_http() {
        let kit = Kit::scaffold();
        let body = kit.page("Dashboard");
        for needle in [
            "<Deferred data=\"stats\" fallback=",
            "<WhenVisible data=\"recent_notes\" fallback=",
            "usePoll(10000, { only: ['stats'] })",
            "useHttp<",
            ".optimistic(",
            ".post(`${root}/profile/name`",
            "profile.errors.name",
            "setLayoutProps({ heading: 'Dashboard' })",
            "<Head title=\"Dashboard\" />",
            // The signed-in user is the shared `auth.user`; after a save the
            // dashboard reloads it so the layout shows the new name.
            "usePage().props",
            "auth.user",
            "router.reload({ only: ['auth'] })",
            "Partial<DashboardProps>",
        ] {
            assert!(body.contains(needle), "Dashboard lacks {needle}:\n{body}");
        }
        assert!(
            body.contains(
                "export default function Dashboard({ stats, recent_notes }: Partial<DashboardProps>)"
            ),
            "the dashboard takes `stats` and `recent_notes`, both absent until loaded, \
             and no `user` prop:\n{body}"
        );
    }

    /// PAR-079: the notes page creates a note through `Form`, remembers its
    /// search, scrolls the cursor-paginated rows, and opens a note through an
    /// instant visit that prefetches on hover.
    #[test]
    fn kit_react_notes_pages_scroll_remember_and_visit_instantly() {
        let kit = Kit::scaffold();
        let index = kit.page("Notes/Index");
        for needle in [
            "<Form",
            "action={`${root}/notes`}",
            "method=\"post\"",
            "name=\"title\"",
            "name=\"body\"",
            "useRemember(",
            "router.get(`${root}/notes`, { search: value }",
            "preserveState: true",
            "<InfiniteScroll",
            "data=\"notes\"",
            "href={`${root}/notes/${note.id}`}",
            "prefetch",
            "component=\"Notes/Show\"",
            "pageProps={(_props, shared) => ({ ...shared, note })}",
        ] {
            assert!(
                index.contains(needle),
                "Notes/Index lacks {needle}:\n{index}"
            );
        }
        assert!(
            index.contains("NotesIndexProps"),
            "Notes/Index is typed by the generated NotesIndexProps:\n{index}"
        );
        let show = kit.page("Notes/Show");
        for needle in [
            "NotesShowProps",
            "<Head title={note.title} />",
            "<Link href={`${root}/notes`}",
        ] {
            assert!(show.contains(needle), "Notes/Show lacks {needle}:\n{show}");
        }
    }

    /// The kit's `inertia-props.ts` is what `suprnova generate-types` writes
    /// for the handlers the kit contract names: the scaffold's auth, reset,
    /// verification and home controllers as they are, and the dashboard,
    /// notes, shared and flash structs as the contract's amendments declare
    /// them. The backend lane writes those four; until its controllers are on
    /// this branch, the sources below stand in for them. The command runs as
    /// a user runs it, so a conflict it refuses fails here too.
    #[test]
    fn kit_react_types_are_what_generate_types_writes_for_the_kit_contract() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let project = tmp.path();
        let controllers = project.join("src/controllers");
        let props = project.join("src/props");
        for dir in [&controllers, &props, &project.join("frontend/src/types")] {
            fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
        }
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"kit-types\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        for (path, body) in [
            (
                controllers.join("home.rs"),
                suprnova_cli::templates::home_controller(),
            ),
            (
                controllers.join("auth.rs"),
                suprnova_cli::templates::auth_controller(),
            ),
            (
                controllers.join("email_verification.rs"),
                suprnova_cli::templates::email_verification_controller(),
            ),
            (
                controllers.join("password_reset.rs"),
                suprnova_cli::templates::password_reset_controller(),
            ),
            (controllers.join("dashboard.rs"), CONTRACT_DASHBOARD),
            (controllers.join("notes.rs"), CONTRACT_NOTES),
            (props.join("shared.rs"), CONTRACT_SHARED),
            (props.join("flash.rs"), CONTRACT_FLASH),
        ] {
            fs::write(&path, body).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        }

        let output = Command::new(BIN)
            .arg("generate-types")
            .current_dir(project)
            .output()
            .expect("spawn suprnova generate-types");
        assert!(
            output.status.success(),
            "`suprnova generate-types` refused the kit contract's structs:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let generated = fs::read_to_string(project.join("frontend/src/types/inertia-props.ts"))
            .expect("read the generated types");
        let shipped = suprnova_cli::templates::react::inertia_props_types();
        assert_eq!(
            shipped, generated,
            "react's inertia-props.ts.tpl is not what `suprnova generate-types` writes \
             for the kit contract.\nexpected:\n{generated}\nshipped:\n{shipped}"
        );
        for declared in [
            "\"Notes/Index\": NotesIndexProps;",
            "\"Notes/Show\": NotesShowProps;",
            "export interface SharedProps {\n  root: string;\n  auth: Auth;\n}",
            "flashDataType: Flash;",
            "toast: Toast | null;",
        ] {
            assert!(shipped.contains(declared), "the types lack {declared}");
        }
    }

    /// The dashboard's props as the kit contract declares them.
    const CONTRACT_DASHBOARD: &str = r#"
use serde::Serialize;
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(Serialize)]
pub struct Stats {
    pub notes: u64,
    pub written_today: u64,
}

#[derive(Serialize)]
pub struct NoteSummary {
    pub id: u64,
    pub title: String,
    pub created_at: String,
}

#[derive(InertiaProps)]
pub struct DashboardProps {
    pub stats: Stats,
    pub recent_notes: Vec<NoteSummary>,
}

#[handler]
pub async fn index(req: Request) -> Response {
    let props: DashboardProps = dashboard_props(&req).await?;
    inertia_response!(&req, "Dashboard", props)
}
"#;

    /// The notes pages' props as the kit contract declares them.
    const CONTRACT_NOTES: &str = r#"
use serde::Serialize;
use suprnova::{handler, inertia_response, InertiaProps, InertiaResponse, Request, Response};

use super::dashboard::NoteSummary;

#[derive(InertiaProps)]
pub struct NotesIndexProps {
    pub notes: Vec<NoteSummary>,
    pub search: String,
}

#[derive(Serialize)]
pub struct NoteView {
    pub id: u64,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
}

#[derive(InertiaProps)]
pub struct NotesShowProps {
    pub note: NoteView,
}

#[handler]
pub async fn index(req: Request) -> Response {
    let props: NotesIndexProps = notes_props(&req).await?;
    InertiaResponse::new("Notes/Index").with_data(props).into()
}

#[handler]
pub async fn show(req: Request) -> Response {
    let note = note_for(&req).await?;
    inertia_response!(&req, "Notes/Show", NotesShowProps { note })
}
"#;

    /// The shared props as the kit contract declares them, under the name
    /// `SharedData`: `suprnova generate-types` refuses a struct named
    /// `SharedProps`, the name of the declaration it writes from this one.
    const CONTRACT_SHARED: &str = r#"
use serde::Serialize;
use suprnova::InertiaProps;

#[derive(Serialize)]
pub struct UserInfo {
    pub id: u64,
    pub name: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct Auth {
    pub user: Option<UserInfo>,
}

#[derive(InertiaProps, Serialize)]
#[inertia_props(shared)]
pub struct SharedData {
    pub auth: Auth,
}
"#;

    /// The flash data as the kit contract declares it.
    const CONTRACT_FLASH: &str = r#"
use serde::Serialize;
use suprnova::InertiaProps;

#[derive(Serialize)]
pub struct Toast {
    pub kind: String,
    pub message: String,
}

#[derive(InertiaProps, Serialize)]
#[inertia_props(flash)]
pub struct Flash {
    pub toast: Option<Toast>,
}
"#;
}
