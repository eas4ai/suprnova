//! The starter kits type-check and build on a fresh scaffold (PAR-076).
//!
//! Each test scaffolds a project with the CLI binary, installs the kit's
//! packages with npm and runs the kit's `check`, `build` and `build:ssr`
//! scripts. They are `#[ignore]`d: they need Node, npm and the network,
//! which the default test run cannot assume. Run them explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test kit_typecheck -- --ignored --test-threads=1
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

/// Scaffold a project of `kit` named `name` into `tmp` and return its
/// `frontend/` directory.
fn scaffold(tmp: &tempfile::TempDir, name: &str, kit: &str) -> PathBuf {
    let out = Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .args([
            "new",
            name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            kit,
        ])
        .current_dir(tmp.path())
        .output()
        .expect("run `suprnova new`");
    assert!(
        out.status.success(),
        "`suprnova new --frontend {kit}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    tmp.path().join(name).join("frontend")
}

/// Run `npm <args>` in `frontend` and fail with its whole output when it
/// does not succeed.
fn npm(frontend: &Path, args: &[&str]) {
    let out = Command::new("npm")
        .args(args)
        .current_dir(frontend)
        .output()
        .unwrap_or_else(|e| panic!("run `npm {}`: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`npm {}` failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        frontend.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
#[ignore = "needs node, npm and the network: runs npm install, check, build and build:ssr"]
fn kit_svelte_type_checks_and_builds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let frontend = scaffold(&tmp, "kitcheck", "svelte");

    npm(&frontend, &["install", "--no-audit", "--no-fund"]);
    assert!(
        frontend
            .join("node_modules/@inertiajs/vite/package.json")
            .is_file(),
        "the kit must install the Inertia Vite plugin"
    );

    npm(&frontend, &["run", "check"]);
    npm(&frontend, &["run", "build"]);
    assert!(
        frontend
            .join("../public/assets/.vite/manifest.json")
            .is_file(),
        "`npm run build` must write the manifest the server reads"
    );

    npm(&frontend, &["run", "build:ssr"]);
    assert!(
        frontend.join("bootstrap/ssr/ssr.js").is_file(),
        "`npm run build:ssr` must land the bundle at frontend/bootstrap/ssr/ssr.js, \
         where `suprnova ssr:start` reads it"
    );

/// The React kit.
mod react {
    use std::path::Path;
    use std::process::Command;

    const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

    /// Run `program args` in `dir` and fail with its whole output unless it
    /// succeeds.
    fn run(dir: &Path, program: &str, args: &[&str]) {
        let output = Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("spawn `{program} {}`: {e}", args.join(" ")));
        assert!(
            output.status.success(),
            "`{program} {}` failed in {}:\n{}{}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "installs npm packages and runs Vite; needs the network"]
    fn kit_react_type_checks_and_builds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        run(
            tmp.path(),
            BIN,
            &[
                "new",
                "kit-react",
                "--no-interaction",
                "--no-git",
                "--frontend",
                "react",
            ],
        );
        let frontend = tmp.path().join("kit-react/frontend");

        run(&frontend, "npm", &["install", "--no-audit", "--no-fund"]);
        run(&frontend, "npm", &["run", "check"]);
        run(&frontend, "npm", &["run", "build"]);
        assert!(
            tmp.path()
                .join("kit-react/public/assets/.vite/manifest.json")
                .is_file(),
            "`npm run build` writes the client bundle and its manifest to public/assets"
        );
        run(&frontend, "npm", &["run", "build:ssr"]);
        assert!(
            frontend.join("bootstrap/ssr/ssr.js").is_file(),
            "`npm run build:ssr` must land the SSR bundle at frontend/bootstrap/ssr/ssr.js, \
             the path `suprnova ssr:start` reads"
        );
    }
}
