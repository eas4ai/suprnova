//! `live:make` scaffolds a component, its view, and registration without ever
//! overwriting user work.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    fs::create_dir_all(tmp.path().join("src")).expect("src");
    fs::write(
        tmp.path().join("src/lib.rs"),
        "pub mod bootstrap;\npub mod controllers;\npub mod routes;\n",
    )
    .expect("lib");
    tmp
}

fn make(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("live:make")
        .args(args)
        .current_dir(root)
        .output()
        .expect("spawn")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("read {}: {e}", path.as_ref().display()))
}

#[test]
fn make_scaffolds_component_view_and_registration() {
    let tmp = project();
    let output = make(tmp.path(), &["Counter"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));

    let component = read(tmp.path().join("src/live/counter.rs"));
    assert!(component.contains("pub struct Counter"), "{component}");
    assert!(component.contains("#[derive(LiveComponent)]"));
    assert!(
        component.contains("name = \"demo-app.counter\""),
        "{component}"
    );
    assert!(
        component.contains("view = \"live/counter.html\""),
        "{component}"
    );
    assert!(component.contains("use suprnova::live::"), "{component}");
    assert!(
        !component.contains("suprnova_live"),
        "only the public facade is named"
    );

    let view = read(tmp.path().join("templates/live/counter.html"));
    assert!(view.contains("live:click=\"increment\""), "{view}");
    assert!(view.contains("{{ count }}"), "{view}");

    let module = read(tmp.path().join("src/live/mod.rs"));
    assert!(module.contains("pub mod counter;"), "{module}");
    assert!(
        module.contains(".register::<counter::Counter>()?"),
        "{module}"
    );
    assert!(module.contains("pub fn registry()"), "{module}");

    let lib = read(tmp.path().join("src/lib.rs"));
    assert_eq!(lib.matches("pub mod live;").count(), 1, "{lib}");
    assert!(
        lib.contains("pub mod routes;"),
        "existing declarations survive"
    );

    for text in [&component, &view, &module] {
        for key in [
            "{snake}",
            "{pascal}",
            "{component_name}",
            "{view}",
            "{package_name}",
        ] {
            assert!(!text.contains(key), "{key} substituted");
        }
    }
    assert!(!tmp.path().join("src/live/.counter.rs.tmp").exists());
}

#[test]
fn a_second_component_appends_its_registration_once() {
    let tmp = project();
    assert!(make(tmp.path(), &["Counter"]).status.success());
    let output = make(tmp.path(), &["todo-list"]);
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(tmp.path().join("src/live/todo_list.rs").exists());
    assert!(tmp.path().join("templates/live/todo_list.html").exists());
    let module = read(tmp.path().join("src/live/mod.rs"));
    assert!(module.contains("pub mod counter;"));
    assert!(module.contains("pub mod todo_list;"));
    let counter = module
        .find(".register::<counter::Counter>()?")
        .expect("counter");
    let todo = module
        .find(".register::<todo_list::TodoList>()?")
        .expect("todo");
    assert!(
        counter < todo,
        "registrations keep creation order:\n{module}"
    );
    assert_eq!(module.matches(".build()").count(), 1);
    assert_eq!(
        read(tmp.path().join("src/lib.rs"))
            .matches("pub mod live;")
            .count(),
        1
    );
    let component = read(tmp.path().join("src/live/todo_list.rs"));
    assert!(component.contains("pub struct TodoList"));
    assert!(
        component.contains("name = \"demo-app.todo-list\""),
        "{component}"
    );
}

#[test]
fn dry_run_reports_the_plan_and_writes_nothing() {
    let tmp = project();
    let output = make(tmp.path(), &["Counter", "--dry-run"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("src/live/counter.rs"), "{text}");
    assert!(text.contains("templates/live/counter.html"), "{text}");
    assert!(text.contains("src/live/mod.rs"), "{text}");
    assert!(!tmp.path().join("src/live").exists());
    assert!(!tmp.path().join("templates").exists());
    assert!(!read(tmp.path().join("src/lib.rs")).contains("live"));
}

#[test]
fn existing_files_are_never_overwritten_and_nothing_partial_is_written() {
    let tmp = project();
    fs::create_dir_all(tmp.path().join("src/live")).expect("dir");
    fs::write(tmp.path().join("src/live/counter.rs"), "// mine\n").expect("user file");
    let output = make(tmp.path(), &["Counter"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("already exists"), "{text}");
    assert_eq!(read(tmp.path().join("src/live/counter.rs")), "// mine\n");
    assert!(
        !tmp.path().join("templates/live/counter.html").exists(),
        "all-or-nothing"
    );
    assert!(!tmp.path().join("src/live/mod.rs").exists());
    assert!(!read(tmp.path().join("src/lib.rs")).contains("live"));

    let view_only = project();
    fs::create_dir_all(view_only.path().join("templates/live")).expect("dir");
    fs::write(
        view_only.path().join("templates/live/counter.html"),
        "<p>mine</p>\n",
    )
    .expect("view");
    let output = make(view_only.path(), &["Counter"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(combined(&output).contains("already exists"));
    assert!(!view_only.path().join("src/live").exists());
    assert_eq!(
        read(view_only.path().join("templates/live/counter.html")),
        "<p>mine</p>\n"
    );
}

#[test]
fn repeated_runs_are_idempotent() {
    let tmp = project();
    assert!(make(tmp.path(), &["Counter"]).status.success());
    let before = read(tmp.path().join("src/live/mod.rs"));
    let output = make(tmp.path(), &["Counter"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(combined(&output).contains("already exists"));
    assert_eq!(read(tmp.path().join("src/live/mod.rs")), before);
}

#[test]
fn invalid_names_are_rejected_before_any_write() {
    let tmp = project();
    for name in [
        "../evil",
        "9lives",
        "my component",
        "mod",
        "self",
        "a/b",
        "",
        "counter.rs",
    ] {
        let output = make(tmp.path(), &[name]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{name:?}: {}",
            combined(&output)
        );
        assert!(
            combined(&output).contains("not a valid"),
            "{name:?}: {}",
            combined(&output)
        );
    }
    assert!(!tmp.path().join("src/live").exists());
    assert!(!tmp.path().join("templates").exists());
}

#[test]
fn make_requires_a_project_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = make(tmp.path(), &["Counter"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(combined(&output).contains("project root"));
}

#[cfg(unix)]
#[test]
fn symlinked_targets_are_refused() {
    let tmp = project();
    let outside = tempfile::tempdir().expect("outside");
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("src/live")).expect("symlink");
    let output = make(tmp.path(), &["Counter"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("symlink"), "{text}");
    assert!(
        fs::read_dir(outside.path())
            .expect("outside")
            .next()
            .is_none(),
        "nothing written through the link"
    );
    assert!(!tmp.path().join("templates").exists());
}

#[cfg(unix)]
#[test]
fn a_failed_write_rolls_back_everything_the_run_wrote() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = project();
    let views = tmp.path().join("templates/live");
    fs::create_dir_all(&views).expect("views dir");
    fs::set_permissions(&views, fs::Permissions::from_mode(0o500)).expect("read-only views");
    if fs::write(views.join(".probe"), b"x").is_ok() {
        // Permissions do not bind this user (root); the scenario cannot be staged.
        let _ = fs::remove_file(views.join(".probe"));
        return;
    }
    let output = make(tmp.path(), &["Counter"]);
    fs::set_permissions(&views, fs::Permissions::from_mode(0o700)).expect("restore views");
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("rolled back"), "{text}");
    assert!(
        !tmp.path().join("src/live/counter.rs").exists(),
        "component rolled back"
    );
    assert!(
        !tmp.path().join("src/live/mod.rs").exists(),
        "registration rolled back"
    );
    assert!(!read(tmp.path().join("src/lib.rs")).contains("live"));
    assert!(
        fs::read_dir(&views).expect("views").next().is_none(),
        "no temp file left behind"
    );
    let entries: Vec<_> = fs::read_dir(tmp.path().join("src/live"))
        .map(|dir| dir.map(|e| e.expect("entry").file_name()).collect())
        .unwrap_or_default();
    assert!(
        entries.is_empty(),
        "src/live is empty after rollback: {entries:?}"
    );
}
