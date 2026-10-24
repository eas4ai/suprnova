//! Issue #171: refusal diagnostics can change while schema bytes stay the same.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use suprnova_cli::commands::tool_schemas::generate_tool_schemas_to_file;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");
const OUTPUT: &str = "frontend/src/types/tool-schemas.ts";
const REFUSED: &str =
    "#[request] pub struct Save { pub values: std::collections::HashMap<String, String> }\n";

fn project(source: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create fixture project");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("write manifest");
    fs::create_dir(dir.path().join("src")).expect("create src");
    fs::write(dir.path().join("src/requests.rs"), source).expect("write requests");
    let first = generate_tool_schemas_to_file(dir.path(), &dir.path().join(OUTPUT))
        .expect("seed the generated module");
    assert!(first.wrote);
    dir
}

struct Watcher(Child);

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn watch(project: &Path) -> Watcher {
    let log = fs::File::create(project.join("watch.log")).expect("create watcher log");
    let child = Command::new(BIN)
        .args(["generate-types", "--watch"])
        .current_dir(project)
        .env("NO_COLOR", "1")
        .stdout(Stdio::from(log.try_clone().expect("clone log")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("start watcher");
    let mut watcher = Watcher(child);
    wait_for(project, &mut watcher, |text| {
        text.contains("Watching ./src for changes")
    });
    watcher
}

fn wait_for(project: &Path, watcher: &mut Watcher, ready: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = fs::read_to_string(project.join("watch.log")).expect("read watcher log");
        if ready(&text) {
            return text;
        }
        assert!(
            watcher.0.try_wait().expect("poll watcher").is_none(),
            "watcher exited before the expected output: {text}"
        );
        assert!(Instant::now() < deadline, "watcher timed out: {text}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn rescan(project: &Path, watcher: &mut Watcher, source: &str) -> String {
    let before = fs::read_to_string(project.join("watch.log")).expect("read watcher log");
    let scans = before.matches("tool-schemas.ts is up to date").count();
    fs::write(project.join("src/requests.rs"), source).expect("edit requests");
    wait_for(project, watcher, |text| {
        text.matches("tool-schemas.ts is up to date").count() > scans
    })
}

#[test]
fn a_new_refusal_is_reported_when_the_module_bytes_do_not_change() {
    let dir = project("");
    let module = fs::read(dir.path().join(OUTPUT)).expect("read seeded module");
    let mut watcher = watch(dir.path());

    let text = rescan(dir.path(), &mut watcher, REFUSED);

    assert_eq!(
        fs::read(dir.path().join(OUTPUT)).expect("read module"),
        module
    );
    assert_eq!(
        text.matches("No tool schema for `Save`").count(),
        1,
        "{text}"
    );
    assert!(text.contains("field `values` has the type"), "{text}");
    assert!(text.contains("HashMap"), "{text}");
}

#[test]
fn an_unchanged_second_scan_does_not_repeat_the_first_refusal() {
    let dir = project(REFUSED);
    let mut watcher = watch(dir.path());

    let text = rescan(dir.path(), &mut watcher, REFUSED);

    assert_eq!(
        text.matches("No tool schema for `Save`").count(),
        1,
        "{text}"
    );
}

#[test]
fn a_schema_write_does_not_repeat_unchanged_refusals() {
    let dir = project(REFUSED);
    let mut watcher = watch(dir.path());
    fs::write(
        dir.path().join("src/requests.rs"),
        format!("{REFUSED}#[request] pub struct Other {{ pub title: String }}\n"),
    )
    .expect("add a supported request");

    let text = wait_for(dir.path(), &mut watcher, |text| {
        text.contains("Regenerated 1 form request schema(s)")
    });

    assert_eq!(
        text.matches("No tool schema for `Save`").count(),
        1,
        "{text}"
    );
}

#[test]
fn one_shot_runs_report_current_refusals_even_when_nothing_is_written() {
    let dir = project(REFUSED);
    for _ in 0..2 {
        let output = Command::new(BIN)
            .arg("generate-types")
            .current_dir(dir.path())
            .output()
            .expect("run generate-types");
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.matches("No tool schema for `Save`").count(),
            1,
            "{stderr}"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("tool-schemas.ts is up to date"));
    }
}
