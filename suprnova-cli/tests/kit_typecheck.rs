//! The starter kits type-check and build on a fresh scaffold (PAR-076):
//! `npm install`, `npm run check`, `npm run build` and `npm run build:ssr`
//! in `frontend/`, with the SSR bundle landing where `suprnova ssr:start`
//! reads it. Ignored: each test installs the kit's packages from the npm
//! registry and runs Vite, so it needs the network and takes minutes. Run
//! one with `cargo test -p suprnova-cli --test kit_typecheck <kit> --
//! --ignored`.

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
