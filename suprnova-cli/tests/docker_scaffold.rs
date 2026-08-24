//! CI-02 — build the image a scaffolded project actually ships with.
//!
//! `scaffold_snapshot` scaffolds a project, repoints its `suprnova`
//! dependency at the in-tree framework, and runs `cargo check`. That
//! catches API drift, and it is the only thing that does. What it cannot
//! catch is everything between "the source type-checks" and "the artifact
//! a user deploys exists":
//!
//! - `cargo check` never resolves a default binary, so a manifest with two
//!   `[[bin]]` entries and no `default-run` type-checks perfectly while
//!   every `cargo run` wrapper fails.
//! - Nothing ever ran the generated `Dockerfile`, so its `COPY` of an
//!   absent `Cargo.lock`, its `npm ci` without a lockfile, and its cache
//!   stage stubbing only one of two declared binaries all shipped.
//!
//! This test closes that gap: scaffold, build the image with the real
//! Dockerfile and the real pinned git tag, then run the migrator inside
//! the resulting container. Nothing is rewritten — the point is to
//! exercise what a user gets.
//!
//! **`#[ignore]`d by default.** It needs Docker, network access, and a
//! full release build of the framework, which is minutes and gigabytes.
//! The release gate runs it; the fast gate does not.
//!
//! ```bash
//! cargo test -p suprnova-cli --test docker_scaffold -- --ignored --nocapture
//! ```
//!
//! ## Why the pinned tag rather than the local tree
//!
//! Docker cannot `COPY` from outside the build context, so a local-path
//! dependency would mean vendoring the framework into the project and
//! editing the generated Dockerfile to copy it — at which point the thing
//! under test is no longer the Dockerfile we ship.
//!
//! Resolving the real `tag = "v<version>"` is also the more honest test:
//! REL-01's finding was that *generated release artifacts do not match the
//! released framework*, and this is the check that compares them. The tag
//! is always the last released one at gate time, because `release.sh` runs
//! the full gate before it bumps.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_suprnova"))
}

/// Fail loudly rather than skipping. The caller opted in by asking for
/// `--ignored`; a gate step that quietly passes when its tooling is
/// missing is worse than no gate step, because the green tick still
/// appears.
fn require_docker() {
    let output = Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .expect("`docker` must be on PATH to run the image gate");
    assert!(
        output.status.success(),
        "the Docker daemon must be reachable to run the image gate; \
         `docker info` said:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scaffold(tmp: &TempDir, name: &str) -> PathBuf {
    scaffold_with(tmp, name, &[])
}

/// `suprnova new <name>` with extra flags — `--api` selects the
/// frontend-free scaffold, whose Docker story is completely different.
fn scaffold_with(tmp: &TempDir, name: &str, extra: &[&str]) -> PathBuf {
    let output = Command::new(cli_binary())
        .arg("new")
        .arg(name)
        .arg("--no-interaction")
        .arg("--no-git")
        .args(extra)
        .current_dir(tmp.path())
        .output()
        .expect("`suprnova new` should run");
    assert!(
        output.status.success(),
        "`suprnova new {name} {extra:?}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    tmp.path().join(name)
}

/// `suprnova docker:init` — the Dockerfile is not part of `new`'s output,
/// it is generated on demand into an existing project.
fn docker_init(project: &Path) {
    let output = Command::new(cli_binary())
        .arg("docker:init")
        .current_dir(project)
        .output()
        .expect("`suprnova docker:init` should run");
    assert!(
        output.status.success(),
        "`suprnova docker:init` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        project.join("Dockerfile").exists(),
        "docker:init reported success but wrote no Dockerfile"
    );
}

/// `docker build`, returning combined output so a failure names the stage
/// that broke rather than just an exit code.
fn docker_build(project: &Path, tag: &str) -> (bool, String) {
    let output = Command::new("docker")
        .args(["build", "--progress", "plain", "-t", tag, "."])
        .current_dir(project)
        .output()
        .expect("`docker build` should run");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn docker_rmi(tag: &str) {
    let _ = Command::new("docker").args(["rmi", "-f", tag]).output();
}

/// The headline gate: a freshly scaffolded project builds into an image,
/// and the binary in that image runs.
#[test]
#[ignore = "needs Docker, network, and a full release build — run with --ignored"]
fn a_fresh_scaffold_builds_and_runs_its_image() {
    require_docker();

    let tmp = TempDir::new().unwrap();
    let project = scaffold(&tmp, "dockergate");
    docker_init(&project);
    let tag = "suprnova-docker-gate:test";
    docker_rmi(tag);

    let (built, log) = docker_build(&project, tag);
    assert!(
        built,
        "the generated Dockerfile must build a fresh scaffold. Build log:\n{log}"
    );

    // Building is not enough — REL-01b's `default-run` defect produced an
    // image whose binary could not be selected. Run the migrator against
    // the default SQLite URL and require a clean exit.
    let run = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-e",
            "APP_ENV=production",
            "-e",
            "APP_KEY=WOxvJ4rQJ0Ck8mF2nT7pL9sV1yB3dG6hK8jN0qR4uW0",
            "-e",
            "DATABASE_URL=sqlite://./database.db?mode=rwc",
            tag,
            "./app",
            "migrate",
        ])
        .output()
        .expect("`docker run` should run");

    let run_log = format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    docker_rmi(tag);

    assert!(
        run.status.success(),
        "`./app migrate` must succeed inside the built image. {run_log}"
    );
}

/// The cheap half of the `default-run` guarantee, and the one worth
/// running often: cargo's own parser, against the *generated* manifest,
/// with no build and no network.
///
/// `cargo metadata --no-deps --offline` reports `default_run` without
/// resolving the dependency graph, so this catches the regression in
/// under a second. It is stronger than the template assertion in
/// `template_drift` because it runs after `{package_name}` substitution
/// and after cargo has actually parsed the TOML — a template that emitted
/// the key into a comment, or produced invalid TOML, passes there and
/// fails here.
///
/// Not `#[ignore]`d: it is fast and hermetic, so the normal gate runs it.
#[test]
fn a_fresh_scaffold_resolves_a_default_binary() {
    let tmp = TempDir::new().unwrap();
    let project = scaffold(&tmp, "runresolve");

    // Cargo treats a package under an ancestor workspace as a member unless
    // the scratch manifest declares its own workspace. Project-local TMPDIR
    // keeps test artifacts auditable but places this copy beneath the Suprnova
    // worktree, so isolate only the parser fixture from the parent workspace.
    // The scaffold templates remain unchanged and are covered byte-for-byte by
    // the snapshot tests.
    let manifest_path = project.join("Cargo.toml");
    let mut manifest = std::fs::read_to_string(&manifest_path).unwrap();
    manifest.push_str("\n[workspace]\n");
    std::fs::write(&manifest_path, manifest).unwrap();

    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(&project)
        .output()
        .expect("`cargo metadata` should run");
    assert!(
        output.status.success(),
        "cargo could not parse the generated manifest:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON");
    let package = &meta["packages"][0];

    let bins: Vec<&str> = package["targets"]
        .as_array()
        .expect("targets array")
        .iter()
        .filter(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|v| v == "bin"))
        })
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        bins.len() >= 2,
        "expected the multi-binary shape this guards; found {bins:?}"
    );

    let default_run = package["default_run"].as_str();
    assert_eq!(
        default_run,
        Some("runresolve"),
        "a scaffolded project declares {bins:?} and must name a `default-run`, \
         substituted to the package name. Without it `cargo run` refuses to \
         pick, and every `suprnova migrate` / `schedule:work` / `web:run` \
         wrapper — which shells out to `cargo run` inside the project — fails \
         before doing any work."
    );
}

/// The API scaffold's image must build too.
///
/// `docker:init` emitted one Dockerfile for every project shape through
/// v0.7.2, and it was the full-stack one. An API project has no
/// `frontend/`, so the image's very first instruction —
/// `COPY frontend/package.json` — failed and `suprnova new --api` +
/// `docker:init` + `docker build` could not succeed at all.
///
/// The static assertions in `template_drift` check the template does not
/// *name* a path the API scaffold lacks. This checks the stronger and
/// more expensive thing, because CI-02 is the reason this file exists:
/// static checks on the full-stack Dockerfile passed while two defects
/// made every scaffolded image unbuildable, and only a real build found
/// them.
#[test]
#[ignore = "needs Docker, network, and a full release build — run with --ignored"]
fn a_fresh_api_scaffold_builds_its_image() {
    require_docker();

    let tmp = TempDir::new().expect("temp dir");
    let project = scaffold_with(&tmp, "apidock", &["--api"]);

    // The premise: this scaffold really has no frontend. If that ever
    // changes, the API Dockerfile is the wrong shape and this test would
    // otherwise keep passing for the wrong reason.
    assert!(
        !project.join("frontend").exists(),
        "an --api scaffold must have no frontend/ — if it grew one, the \
         API Dockerfile needs a frontend stage after all"
    );
    assert!(
        !project.join("cmd").exists(),
        "an --api scaffold must have no cmd/ — its server bin is src/main.rs"
    );

    docker_init(&project);

    let dockerfile = std::fs::read_to_string(project.join("Dockerfile"))
        .expect("docker:init wrote a Dockerfile");
    // Instructions only. The API template's own header comment explains
    // that an API project has no `frontend/`, so a raw substring check
    // matches the explanation and fails on a correct file — which is
    // exactly what it did on the first run of this test.
    let instructions: String = dockerfile
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !instructions.contains("frontend/"),
        "docker:init emitted the full-stack Dockerfile for an --api \
         project; its frontend COPY cannot succeed here. Got:\n{dockerfile}"
    );

    let tag = "suprnova-api-scaffold-test:latest";
    let (ok, output) = docker_build(&project, tag);
    docker_rmi(tag);

    assert!(
        ok,
        "an --api scaffold's image must build. Output:\n{output}"
    );
}
