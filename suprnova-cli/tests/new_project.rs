//! `suprnova new`: it refuses a project path that already names anything on
//! disk, and ships the root template a first visit renders through.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

#[cfg(unix)]
#[test]
fn a_dangling_symlink_at_the_project_path_is_refused() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let target = tmp.path().join("elsewhere");
    std::os::unix::fs::symlink(&target, tmp.path().join("myapp")).expect("dangling symlink");
    let output = Command::new(BIN)
        .args([
            "new",
            "myapp",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path())
        .output()
        .expect("spawn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("already exists"), "{text}");
    assert!(
        !target.exists(),
        "nothing was created at the link target: {}",
        target.display()
    );
}

/// Scaffolds `name` with `frontend` under `dir` and returns its path.
fn scaffold(dir: &std::path::Path, name: &str, frontend: &str) -> std::path::PathBuf {
    let output = Command::new(BIN)
        .args([
            "new",
            name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            frontend,
        ])
        .current_dir(dir)
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "`suprnova new {name} --frontend {frontend}` failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    dir.join(name)
}

/// RDOC-006: `suprnova new` ships a root template under `templates/` in
/// place of the `frontend/index.html` the server never served, renders
/// every first visit through it, and titles its pages with the project's
/// name rather than `Suprnova`, for every frontend.
#[test]
fn rdoc_006_a_new_project_ships_a_root_template_and_a_default_title() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for frontend in ["svelte", "react", "vue"] {
        let name = format!("my-shop-{frontend}");
        let project = scaffold(tmp.path(), &name, frontend);

        assert!(
            !project.join("frontend/index.html").exists(),
            "{frontend}: frontend/index.html is still written"
        );
        let template = std::fs::read_to_string(project.join("templates/app.html"))
            .unwrap_or_else(|e| panic!("{frontend}: templates/app.html: {e}"));
        for part in ["{{ lang }}", "{{ title }}", "{{ head }}", "{{ body }}"] {
            assert!(
                template.contains(part),
                "{frontend}: the root template does not place {part}:\n{template}"
            );
        }

        let bootstrap = std::fs::read_to_string(project.join("src/bootstrap.rs"))
            .unwrap_or_else(|e| panic!("{frontend}: src/bootstrap.rs: {e}"));
        let title = format!("My Shop {}", {
            let mut chars = frontend.chars();
            let first = chars.next().expect("a frontend name").to_uppercase();
            format!("{first}{}", chars.as_str())
        });
        for expected in [
            "#[suprnova::inertia_root(path = \"app.html\")]".to_string(),
            ".root_template(InertiaRootTemplate::of::<AppDocument>())".to_string(),
            format!(".default_title(\"{title}\")"),
        ] {
            assert!(
                bootstrap.contains(&expected),
                "{frontend}: src/bootstrap.rs lacks {expected}:\n{bootstrap}"
            );
        }
    }
}

/// RDOC-006: the Dockerfile `suprnova docker:init` writes for a new project
/// copies `templates/` into the stage that compiles the application, before
/// it compiles it: Askama reads the root template at compile time, and the
/// build context's `.dockerignore` keeps it.
#[test]
fn rdoc_006_the_dockerfile_of_a_new_project_copies_its_templates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(tmp.path(), "docked", "svelte");
    let output = Command::new(BIN)
        .arg("docker:init")
        .current_dir(&project)
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "`suprnova docker:init` failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let dockerfile = std::fs::read_to_string(project.join("Dockerfile")).expect("the Dockerfile");
    let stage = dockerfile
        .split_once("AS backend-builder")
        .and_then(|(_, rest)| rest.split_once("AS runtime"))
        .expect("a backend-builder stage before the runtime stage")
        .0;
    let lines: Vec<&str> = stage
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect();
    let copies = lines
        .iter()
        .position(|line| {
            line.starts_with("COPY ") && !line.contains("--from=") && line.contains("template")
        })
        .unwrap_or_else(|| panic!("the backend stage copies no templates:\n{stage}"));
    let builds = lines
        .iter()
        .rposition(|line| line.contains("cargo build --release"))
        .expect("the backend stage builds the application");
    assert!(
        copies < builds,
        "templates are copied after the build:\n{stage}"
    );
    assert!(
        lines[copies..builds]
            .iter()
            .any(|line| line.contains("templates/")),
        "the copied templates never reach templates/ before the build:\n{stage}"
    );

    let ignore = std::fs::read_to_string(project.join(".dockerignore")).expect(".dockerignore");
    for line in ignore.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        assert!(
            !"templates/app.html".starts_with(line.trim_end_matches('/')),
            ".dockerignore excludes `{line}`, which takes templates/ out of the build context"
        );
    }
}
