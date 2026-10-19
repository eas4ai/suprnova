//! A generated application's account flows, validation-error display and
//! static-file serving, proven by running the generated application.
//!
//! Scaffolds a Vue project, points it at the in-tree framework crate,
//! writes `tests/fixtures/scaffold/account_flows.rs` into it, and runs
//! `cargo test` there. That test drives the generated router in-process
//! over a real Hyper connection with mail captured in memory (see the
//! fixture for what it covers: GitHub issues #6, #7 and #8).
//!
//! `#[ignore]`d for the same reason as the compile checks in
//! `scaffold_snapshot.rs`: it builds the framework and the generated
//! application. Run it explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test scaffold_account_flows -- --ignored --nocapture
//! ```

#[path = "scaffold_account_flows/harness.rs"]
mod harness;

use std::fs;

/// The test that runs inside the generated project. `__PACKAGE__` is the
/// generated crate's name.
const ACCOUNT_FLOWS_TEST: &str = include_str!("fixtures/scaffold/account_flows.rs");

const PROJECT: &str = "account_flows";

#[test]
#[ignore = "runs the generated application's own tests; builds the framework and the app; slow"]
fn a_generated_application_verifies_email_resets_passwords_and_serves_its_assets() {
    let tmp = harness::scratch_dir();
    let project = harness::scaffold(tmp.path(), PROJECT);
    harness::patch_manifest(&project);

    // What `npm run build` would leave behind: a hashed bundle and Vite's
    // manifest, the one file under public/ that must not be served.
    let assets = project.join("public/assets");
    fs::create_dir_all(assets.join(".vite")).expect("create public/assets/.vite");
    fs::write(assets.join("app-1234.js"), "console.log('built')\n").expect("write asset");
    fs::write(assets.join(".vite/manifest.json"), "{}\n").expect("write manifest");

    // The fixture reads the mailed links under this address.
    let output = harness::run_project_test(
        tmp.path(),
        &project,
        "account_flows",
        ACCOUNT_FLOWS_TEST,
        "http://app.test",
    );
    assert!(
        output.status.success(),
        "the generated application's account-flow test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test account_flows_validation_errors_and_static_files ... ok"),
        "the in-project test must have run and passed:\n{stdout}"
    );
}
