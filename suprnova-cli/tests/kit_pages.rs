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

    /// PAR-080 and PAR-079: the dashboard sends the user with the page,
    /// `stats` deferred and `recent_notes` optional, both over the user's
    /// own notes.
    #[test]
    fn kit_dashboard_defers_stats_and_leaves_recent_notes_optional() {
        each_kit(|app| {
            let dashboard = app.read("src/controllers/dashboard.rs");
            let index = function(&dashboard, "index");
            for needle in [
                r#"InertiaResponse::new("Dashboard")"#,
                r#".with("user", "#,
                r#".defer("stats", "#,
                r#".optional("recent_notes", "#,
            ] {
                assert!(
                    contains(index, needle),
                    "{}: the dashboard handler must chain `{needle}`; got:\n{index}",
                    app.kit
                );
            }
            for needle in [
                "pub struct UserInfo {",
                "pub struct NoteStats {",
                "pub notes: i64,",
                "pub written_today: i64,",
                "pub struct RecentNote {",
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
            ] {
                assert!(
                    contains(index, needle),
                    "{}: the notes index must contain `{needle}`; got:\n{index}",
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
                r#"inertia_response!(&req, "Notes/Show", NoteShowProps"#,
            ] {
                assert!(
                    contains(show, needle),
                    "{}: the notes show handler must contain `{needle}`; got:\n{show}",
                    app.kit
                );
            }
            for needle in ["pub struct NoteShowProps {", "pub note: NoteProps,"] {
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

    /// PAR-078: one flash struct, marked for the generator, and a toast
    /// flashed after each of the eight actions the requirement names.
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
            for needle in [
                "#[derive(InertiaProps)]\n#[inertia_props(flash)]\npub struct Toast {",
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
    /// flash struct as `flashDataType`, types the note pages, and warns
    /// about no prop type.
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
                "export interface Toast {\n  kind: string;\n  message: string;\n}",
                "    flashDataType: Toast;",
                r#"  "Notes/Show": NoteShowProps;"#,
                "export interface NoteProps {",
                "export interface NoteStats {",
                "export interface RecentNote {",
                "export interface UserInfo {",
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
}
