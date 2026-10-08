//! Each starter kit installs, type-checks and builds on a fresh scaffold
//! (PAR-076): `npm install`, `npm run check`, `npm run build` and
//! `npm run build:ssr`, with the SSR bundle landing at
//! `frontend/bootstrap/ssr/ssr.js`, where `suprnova ssr:start` reads it.
//!
//! `#[ignore]`d: each test downloads the kit's packages and runs Vite.
//! Run them explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test kit_typecheck -- --ignored --test-threads=1
//! ```

use std::path::Path;
use std::process::Command;

/// Run `program args` in `dir` and fail the test with its output when it
/// exits unsuccessfully.
fn run(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("spawn `{program} {}`: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`{program} {}` failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "installs the kit's npm packages and builds it; slow and needs the network"]
fn kit_vue_type_checks_and_builds() {
    // Under the target directory, not a tmpfs `/tmp`: `node_modules` is
    // tens of thousands of files.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    run(
        tmp.path(),
        env!("CARGO_BIN_EXE_suprnova"),
        &[
            "new",
            "kit_vue",
            "--no-interaction",
            "--no-git",
            "--frontend",
            "vue",
        ],
    );
    let frontend = tmp.path().join("kit_vue/frontend");
    run(&frontend, "npm", &["install", "--no-audit", "--no-fund"]);
    run(&frontend, "npm", &["run", "check"]);
    run(&frontend, "npm", &["run", "build"]);
    run(&frontend, "npm", &["run", "build:ssr"]);

    let bundle = frontend.join("bootstrap/ssr/ssr.js");
    assert!(
        bundle.is_file(),
        "`npm run build:ssr` must write {}",
        bundle.display()
    );
    let manifest = tmp.path().join("kit_vue/public/assets/.vite/manifest.json");
    assert!(
        manifest.is_file(),
        "`npm run build` must write the client manifest at {}",
        manifest.display()
    );
}
