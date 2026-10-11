//! Compile-time rules of route binding (docs/spec/route-binding.md).
//!
//! Only a real `cargo check` of a crate proves a build failure. Each test
//! copies the fixture workspace under this package's target directory and
//! checks one member; the copies share one target directory, so the
//! framework is checked once. The tutorial member is written from
//! `manual/tutorial-inertia-crud.md` itself (BIND-014). The TypeScript
//! route generator is driven through the CLI's library by a fixture binary
//! (BIND-015), so this package does not depend on the CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn macros_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_root() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("route-binding")
}

const MEMBERS: &[&str] = &[
    "auto-names",
    "removed-macro",
    "removed-trait-fn",
    "route-key-unknown",
    "route-key-injected",
    "enum-fields",
    "authorize-form",
    "authorize-primitive",
    "resource-missing",
    "resource-selected",
    "tutorial",
    "ts-routes",
    "url-strings",
    "impl-handlers",
];

/// A fresh copy of the fixture workspace for one test, never the system
/// temp dir: the shared target holds a check of `suprnova`.
fn workspace(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the previous fixture copy");
    }
    copy_tree(&macros_dir().join("tests/fixtures/route-binding"), &dir);

    let framework = macros_dir()
        .join("../framework")
        .canonicalize()
        .expect("locate the framework crate");
    let framework = framework.to_str().expect("a UTF-8 framework path");
    let cli = macros_dir()
        .join("../suprnova-cli")
        .canonicalize()
        .expect("locate the CLI crate");
    let cli = cli.to_str().expect("a UTF-8 CLI path");
    let members = MEMBERS
        .iter()
        .map(|member| format!("{member:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\n\
             resolver = \"3\"\n\
             members = [{members}]\n\
             \n\
             [workspace.dependencies]\n\
             suprnova = {{ path = {framework:?}, default-features = false }}\n\
             suprnova-cli = {{ path = {cli:?} }}\n\
             sea-orm = {{ version = \"2.0\", default-features = false }}\n\
             serde = {{ version = \"1\", features = [\"derive\"] }}\n\
             tokio = {{ version = \"1\", features = [\"sync\"] }}\n\
             chrono = {{ version = \"0.4\", features = [\"serde\"] }}\n\
             validator = {{ version = \"0.20\", features = [\"derive\"] }}\n"
        ),
    )
    .expect("write the fixture workspace manifest");
    // Starting from the repository's lockfile keeps the fixture on the
    // dependency versions the framework is tested with.
    fs::copy(macros_dir().join("../Cargo.lock"), dir.join("Cargo.lock"))
        .expect("seed the fixture lockfile");
    dir
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create the fixture copy");
    for entry in fs::read_dir(source).expect("read the fixture") {
        let entry = entry.expect("read a fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("inspect a fixture entry").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy a fixture file");
        }
    }
}

fn cargo_check(workspace: &Path, member: &str) -> Output {
    Command::new(env!("CARGO"))
        .args(["check", "--quiet", "-p", &format!("route-binding-{member}")])
        .env("CARGO_TARGET_DIR", scratch_root().join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(workspace)
        .output()
        .expect("run cargo check on the fixture workspace")
}

fn assert_compiles(output: &Output, why: &str) {
    assert!(
        output.status.success(),
        "{why}, yet the check failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(output: &Output, expected: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the check passed, expected it to fail naming {expected:?}:\n{stderr}"
    );
    for fragment in expected {
        assert!(
            stderr.contains(fragment),
            "expected `{fragment}` in the compile error:\n{stderr}"
        );
    }
}

/// The first ```rust block after the line `heading` in `markdown`.
fn rust_block_after(markdown: &str, heading: &str) -> String {
    block_after(markdown, heading, "rust")
}

/// The first code block of `language` after `heading` in `markdown`.
fn block_after(markdown: &str, heading: &str, language: &str) -> String {
    let start = markdown
        .find(heading)
        .unwrap_or_else(|| panic!("the tutorial has a `{heading}` section"));
    let rest = &markdown[start..];
    let fence = format!("```{language}\n");
    let open = rest
        .find(&fence)
        .unwrap_or_else(|| panic!("`{heading}` has a {language} block"));
    let body = &rest[open + fence.len()..];
    let close = body.find("\n```").expect("the block is closed");
    body[..close].to_owned()
}

#[test]
fn bind_001_auto_route_binding_is_reachable_through_both_paths() {
    let dir = workspace("auto-names");
    assert_compiles(
        &cargo_check(&dir, "auto-names"),
        "`AutoRouteBinding` names `RouteBinding` through the root and `database`",
    );
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read a source directory") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The source files of every workspace member, read from the root manifest.
fn workspace_sources() -> Vec<PathBuf> {
    let root = macros_dir().join("..");
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let start = manifest
        .find("members = [")
        .expect("the workspace lists its members");
    let list = &manifest[start..];
    let list = &list[..list.find(']').expect("the member list is closed")];
    let mut sources = Vec::new();
    for member in list.split('"').skip(1).step_by(2) {
        let src = root.join(member).join("src");
        if src.is_dir() {
            rust_files(&src, &mut sources);
        }
    }
    sources
}

/// Whether `rest` starts with the identifier `name`, not a longer one.
fn starts_with_ident(rest: &str, name: &str) -> bool {
    rest.strip_prefix(name).is_some_and(|after| {
        !after
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

#[test]
fn bind_001_the_route_binding_macro_is_gone() {
    // Not exported: the path does not resolve at all. A macro that still
    // existed but failed to expand would fail with another error.
    let dir = workspace("removed-macro");
    assert_rejected(
        &cargo_check(&dir, "removed-macro"),
        &["could not find `route_binding` in `suprnova`"],
    );

    // Nor defined anywhere in the workspace, exported or not: no
    // `macro_rules! route_binding` and no proc macro of that name.
    let mut definitions = Vec::new();
    let sources = workspace_sources();
    assert!(
        sources
            .iter()
            .any(|path| path.ends_with("suprnova-macros/src/lib.rs")),
        "the scan must read the macros crate"
    );
    for path in sources {
        let text = fs::read_to_string(&path).expect("read a source file");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if let Some(at) = line.find("macro_rules!")
                && starts_with_ident(
                    line[at + "macro_rules!".len()..].trim_start(),
                    "route_binding",
                )
            {
                definitions.push(format!("{}:{}: {line}", path.display(), index + 1));
            }
            if line.trim_start().starts_with("#[proc_macro")
                && let Some(signature) = lines[index + 1..].iter().find(|next| next.contains("fn "))
                && let Some(at) = signature.find("fn ")
                && starts_with_ident(signature[at + 3..].trim_start(), "route_binding")
            {
                definitions.push(format!("{}:{}: {signature}", path.display(), index + 1));
            }
        }
    }
    assert!(
        definitions.is_empty(),
        "the removed `route_binding!` macro is defined:\n{}",
        definitions.join("\n")
    );
}

#[test]
fn bind_001_the_param_name_trait_is_gone() {
    let dir = workspace("removed-trait-fn");
    assert_rejected(&cargo_check(&dir, "removed-trait-fn"), &["param_name"]);
}

#[test]
fn bind_003_associated_handlers_take_every_binding_form() {
    let dir = workspace("impl-handlers");
    assert_compiles(
        &cargo_check(&dir, "impl-handlers"),
        "`#[handler(Self = Posts)]` inside `impl Posts` must compile with every \
         binding form and register as `Posts::show`",
    );
}

#[test]
fn bind_003_a_handler_inside_an_impl_block_without_its_type_fails_the_build() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/route-binding/*.rs");
}

#[test]
fn bind_005_a_route_key_that_is_no_column_fails_the_build() {
    let dir = workspace("route-key-unknown");
    assert_rejected(
        &cargo_check(&dir, "route-key-unknown"),
        &["route_key = \"slug\"", "names no column of `Post`"],
    );
    let dir = workspace("route-key-injected");
    assert_rejected(
        &cargo_check(&dir, "route-key-injected"),
        &["route_key = \"__eager\"", "names no column"],
    );
}

#[test]
fn bind_010_an_enum_with_a_field_cannot_derive_route_binding() {
    let dir = workspace("enum-fields");
    assert_rejected(&cargo_check(&dir, "enum-fields"), &["unit variants"]);
}

#[test]
fn bind_015_an_authorize_target_that_neither_binds_nor_is_a_path_value_fails_the_build() {
    let dir = workspace("authorize-form");
    assert_rejected(
        &cargo_check(&dir, "authorize-form"),
        &["`UpdatePost` cannot be bound from a route parameter"],
    );
}

#[test]
fn bind_015_a_primitive_authorize_target_still_compiles() {
    let dir = workspace("authorize-primitive");
    assert_compiles(
        &cargo_check(&dir, "authorize-primitive"),
        "`#[authorize(\"show\", id)]` over `id: i64` keeps compiling",
    );
}

#[test]
fn bind_011_a_selected_action_without_its_function_fails_the_build() {
    let dir = workspace("resource-missing");
    assert_rejected(&cargo_check(&dir, "resource-missing"), &["destroy"]);
    let dir = workspace("resource-selected");
    assert_compiles(
        &cargo_check(&dir, "resource-selected"),
        "an action `only` leaves out needs no function",
    );
}

#[test]
fn bind_012_every_string_pair_form_written_before_route_binding_compiles() {
    let dir = workspace("url-strings");
    assert_compiles(
        &cargo_check(&dir, "url-strings"),
        "a `route()` or `try_route()` call written with string pairs must keep compiling",
    );
}

#[test]
fn bind_014_the_tutorial_model_controller_and_routes_compile() {
    let tutorial = fs::read_to_string(macros_dir().join("../manual/tutorial-inertia-crud.md"))
        .expect("read the tutorial");
    let dir = workspace("tutorial");
    let member = dir.join("tutorial/src");
    fs::write(
        member.join("models/todo.rs"),
        rust_block_after(&tutorial, "## 3. Model"),
    )
    .expect("write the model");
    fs::write(
        member.join("controllers/todo.rs"),
        rust_block_after(&tutorial, "## 4. Controller"),
    )
    .expect("write the controller");
    fs::write(
        member.join("routes.rs"),
        rust_block_after(&tutorial, "## 5. Routes"),
    )
    .expect("write the routes");
    // `inertia_response!` checks at compile time that each page exists, so
    // the tutorial's three pages are written where it looks.
    let pages = dir.join("tutorial/frontend/src/pages/Todos");
    fs::create_dir_all(&pages).expect("create the pages directory");
    for page in ["Index", "Create", "Edit"] {
        fs::write(
            pages.join(format!("{page}.svelte")),
            block_after(&tutorial, &format!("### {page}\n"), "svelte"),
        )
        .expect("write a page");
    }
    assert_compiles(
        &cargo_check(&dir, "tutorial"),
        "the tutorial's model, controller and routes must compile as written",
    );
}

#[test]
fn bind_015_the_typescript_route_generator_reads_binding_fields_and_bound_arguments() {
    let dir = workspace("ts-routes");
    let project = dir.join("ts-project");
    fs::create_dir_all(project.join("src/controllers")).expect("create the project");
    fs::write(
        project.join("src/routes.rs"),
        r#"use suprnova::{put, routes};

routes! {
    put!("/posts/{post:slug}", controllers::posts::update).name("posts.update"),
}
"#,
    )
    .expect("write routes.rs");
    fs::write(project.join("src/controllers/mod.rs"), "pub mod posts;\n")
        .expect("write controllers/mod.rs");
    fs::write(
        project.join("src/controllers/posts.rs"),
        r#"use suprnova::{handler, request, Response};
use crate::models::Post;

#[request]
pub struct UpdatePost {
    pub title: String,
}

#[handler]
pub async fn update(post: Post, form: UpdatePost) -> Response {
    suprnova::http::text(form.title)
}
"#,
    )
    .expect("write the controller");

    let output = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "-p", "route-binding-ts-routes", "--"])
        .arg(&project)
        .env("CARGO_TARGET_DIR", scratch_root().join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(&dir)
        .output()
        .expect("run the generator fixture");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the generator fixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("route posts.update params post request UpdatePost"),
        "`{{post:slug}}` must be the parameter `post`, and the leading bound \
         `post: Post` no form request:\n{stdout}"
    );
    assert!(
        stdout.contains("typescript-holds-a-field false"),
        "the TypeScript must not hold `post:slug`:\n{stdout}"
    );
}
