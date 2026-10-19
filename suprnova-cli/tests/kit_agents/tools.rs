//! The two tools every Inertia and Live kit ships on the signed-in user's
//! own notes: `notes.search` and `notes.create`, so a new project's MCP
//! endpoint lists something from the first `cargo run`, and its approval
//! page has a real scope to show.
//!
//! These read what `suprnova new` writes; none compiles a scaffold. The
//! compiled proof is the `agent_001_`, `agent_002_` and `agent_014_` cases of
//! the agent pages fixtures in `scaffold_snapshot`, and the browser proof is
//! the notes cases of the `agents_browser` binary.

use std::path::Path;

use crate::shared::{INERTIA_KITS, Project, SHARED_BACKEND_KITS, contains};

/// The template a scaffolded file must equal byte for byte.
fn template(relative: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/templates/files")
            .join(relative),
    )
    .unwrap_or_else(|error| panic!("read the template {relative}: {error}"))
}

/// The authenticated group of a scaffolded `src/routes.rs`: from the end
/// of the guest group to the first `auth()` middleware, which closes it in
/// both kits.
fn authenticated_group(routes: &str) -> &str {
    let guest_end = routes
        .find(".middleware(middleware::authenticate::guest())")
        .expect("routes.rs has a guest group");
    let auth_end = routes
        .find(".middleware(middleware::authenticate::auth())")
        .expect("routes.rs has an authenticated group");
    &routes[guest_end..auth_end]
}

/// The text of the `#[tool(...)]` marking whose name is `name`.
fn marking<'a>(controller: &'a str, name: &str) -> &'a str {
    let needle = format!("name = \"{name}\"");
    let at = controller
        .find(&needle)
        .unwrap_or_else(|| panic!("no tool is named {name}:\n{controller}"));
    let start = controller[..at]
        .rfind("#[tool(")
        .unwrap_or_else(|| panic!("{name} is not in a #[tool(...)] marking"));
    let end = controller[at..]
        .find(")]\n#[handler]")
        .map(|end| end + at)
        .unwrap_or_else(|| panic!("the marking of {name} does not end on its handler"));
    &controller[start..end]
}

/// AGENT-017: `src/agents.rs` declares the application's two scopes, each
/// with the description the approval page shows, and no other.
#[test]
fn agent_017_kits_declare_the_notes_scopes() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let agents = project.read("src/agents.rs");
        assert_eq!(
            agents.matches("suprnova::tool_scopes! {").count(),
            1,
            "{kit}: src/agents.rs declares its scopes once:\n{agents}"
        );
        let start = agents
            .find("suprnova::tool_scopes! {")
            .expect("the declaration");
        let declared = &agents[start..];
        let declared = &declared[..declared.find("\n}\n").expect("the declaration closes")];
        assert!(
            declared.contains("    pub mod scope {"),
            "{kit}: the scopes live in `agents::scope`:\n{declared}"
        );
        let lines: Vec<&str> = declared
            .lines()
            .map(str::trim)
            .filter(|line| line.contains(" => "))
            .collect();
        assert_eq!(
            lines,
            [
                r#"NOTES_READ = "notes:read" => "Read your notes";"#,
                r#"NOTES_WRITE = "notes:write" => "Write notes for you";"#,
            ],
            "{kit}: src/agents.rs declares exactly the two notes scopes"
        );
    }
}

/// AGENT-017: both kit families compile the one shared controller of the
/// notes tools, and each tool carries its markings: who may call it, the
/// scope a token needs, its input and output, and its hints.
#[test]
fn agent_017_kits_ship_the_notes_tools() {
    let shared = template("backend/controllers/note_tools.rs.tpl");
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let controller = project.read("src/controllers/note_tools.rs");
        assert_eq!(
            controller, shared,
            "{kit}: src/controllers/note_tools.rs is the shared template"
        );
        let modules = project.read("src/controllers/mod.rs");
        assert!(
            modules.lines().any(|line| line == "pub mod note_tools;"),
            "{kit}: src/controllers/mod.rs must declare `pub mod note_tools;`:\n{modules}"
        );
    }

    let search = marking(&shared, "notes.search");
    for part in [
        "callers = [user, agent],",
        "scopes = [scope::NOTES_READ],",
        "input = SearchNotesInput,",
        "output = NoteSearchResults,",
        "annotations(read_only, untrusted_content),",
    ] {
        assert!(
            contains(search, part),
            "notes.search lacks `{part}`:\n{search}"
        );
    }
    let create = marking(&shared, "notes.create");
    for part in [
        "callers = [user, agent],",
        "scopes = [scope::NOTES_WRITE],",
        "input = CreateNoteInput,",
        "output = NoteItem,",
        "annotations(read_only = false, destructive = false),",
    ] {
        assert!(
            contains(create, part),
            "notes.create lacks `{part}`:\n{create}"
        );
    }
    for part in [
        "#[tool_input]\npub struct SearchNotesInput {",
        "#[validate(length(max = 200,",
        "pub query: String,",
        "#[validate(range(min = 1, max = 50,",
        "pub limit: Option<u32>,",
        "#[tool_output]\npub struct NoteSearchResults {",
        "pub notes: Vec<NoteItem>,",
        "#[tool_output]\npub struct NoteItem {",
        "#[tool_input]\n#[form_request(custom_hooks)]\npub struct CreateNoteInput {",
        "pub async fn search(caller: ToolCaller, form: SearchNotesInput) -> Response {",
        "pub async fn create(caller: ToolCaller, form: CreateNoteInput) -> Response {",
        // The caller's own notes only, through the notes page's own
        // queries, and a write through the model the notes form writes
        // with, so search indexes the note.
        "Note::owned_by(user_id)",
        ".filter(format!(\"user_id = {user_id}\"))",
        "<Note as Model>::create(attrs! {",
    ] {
        assert!(
            contains(&shared, part),
            "the notes tools controller lacks `{part}`"
        );
    }
    for absent in ["share_tools", "Auth::user", "unwrap()", "expect("] {
        assert!(
            !shared.contains(absent),
            "the notes tools controller must not use `{absent}`"
        );
    }
}

/// AGENT-017: `notes.create` takes what the notes form takes, with the
/// same bounds and the same messages as the Inertia kits' `StoreNoteRequest`
/// and the Live kit's note form.
#[test]
fn agent_017_notes_create_keeps_the_notes_form_bounds() {
    let shared = template("backend/controllers/note_tools.rs.tpl");
    for form in [
        template("backend/controllers/notes.rs.tpl"),
        template("live/src/live/note_form.rs.tpl"),
    ] {
        for rule in [
            "length(\n        min = 1,\n        max = 255,\n        message = \"Give the note a title of 1 to 255 characters.\"\n    )",
            "length(max = 10000, message = \"Keep the note to 10000 characters.\")",
        ] {
            assert!(
                contains(&form, rule),
                "the notes form no longer holds `{rule}`"
            );
            assert!(
                contains(&shared, rule),
                "notes.create does not hold the notes form's `{rule}`"
            );
        }
    }
    assert!(
        contains(
            &shared,
            "errs.add(\"title\", \"Give the note a title of 1 to 255 characters.\");"
        ),
        "notes.create refuses a title of spaces as the notes form does"
    );
}

/// AGENT-014 and AGENT-017: the tools' routes sit behind sign-in, the
/// search's static segment beside `/notes/{id}`, and the notes page is the
/// one page that offers them. Nothing shares a tool with every page.
#[test]
fn agent_017_the_notes_page_offers_the_notes_tools() {
    for kit in SHARED_BACKEND_KITS {
        let project = Project::new(kit);
        let routes = project.read("src/routes.rs");
        let authenticated = authenticated_group(&routes);
        for route in [
            r#"get!("/notes", controllers::notes::index).name("notes.index").with_tools(["notes.search", "notes.create"])"#,
            r#"get!("/notes/search", controllers::note_tools::search).name("notes.search")"#,
            r#"post!("/notes/tools", controllers::note_tools::create).name("notes.create")"#,
        ] {
            assert!(
                contains(authenticated, route),
                "{kit}: `{route}` must sit in the authenticated group:\n{routes}"
            );
        }
        let search = routes.find("\"/notes/search\"").expect("the search route");
        let show = routes.find("\"/notes/{id}\"").expect("the note route");
        assert!(
            search < show,
            "{kit}: the search route reads first, beside the note it outranks"
        );
        assert_eq!(
            routes.matches("with_tools").count(),
            1,
            "{kit}: the notes page is the one page that offers tools:\n{routes}"
        );
        for file in ["src/routes.rs", "src/agents.rs", "cmd/main.rs"] {
            assert!(
                !project.read(file).contains("share_tools"),
                "{kit}: {file} must share no tool with every page"
            );
        }
    }
}

/// AGENT-015 and AGENT-017: the tools' input schemas are in the
/// `tool-schemas.ts` every Inertia kit ships, as `generate-types` writes
/// them, bounds included.
#[test]
fn agent_017_inertia_tool_schemas_carry_the_notes_tools() {
    let schemas = template("frontend/types/tool-schemas.ts.tpl");
    for (input, parts) in [
        (
            "SearchNotesInput",
            &[
                "\"maxLength\": 200",
                "\"minimum\": 1",
                "\"maximum\": 50",
                "\"default\": \"\"",
            ][..],
        ),
        (
            "CreateNoteInput",
            &[
                "\"minLength\": 1",
                "\"maxLength\": 255",
                "\"maxLength\": 10000",
            ][..],
        ),
    ] {
        let start = schemas
            .find(&format!("  {input}: {{"))
            .unwrap_or_else(|| panic!("tool-schemas.ts has no {input}"));
        let rest = &schemas[start..];
        let schema = &rest[..rest.find("\n  },").expect("the schema closes")];
        for part in parts {
            assert!(schema.contains(part), "{input} lacks {part}:\n{schema}");
        }
    }
    for kit in INERTIA_KITS {
        let project = Project::new(kit);
        assert_eq!(
            project.read("frontend/src/types/tool-schemas.ts"),
            schemas,
            "{kit}: tool-schemas.ts"
        );
    }
}

/// AGENT-017: the API kit has no notes, so it ships no notes tools.
#[test]
fn agent_017_api_kit_ships_no_notes_tools() {
    let project = Project::new("api");
    assert!(
        !project.root.join("src/controllers/note_tools.rs").exists(),
        "api: the API kit has no notes to offer tools on"
    );
    let routes = project.read("src/routes.rs");
    assert!(
        !routes.contains("note_tools") && !routes.contains("with_tools"),
        "api: {routes}"
    );
}
