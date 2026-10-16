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

/// Run a kit command and include its output if it fails.
fn run(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("run `{program} {}`: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`{program} {}` failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Render the home page through the built worker with no localization props.
/// Issue #153: the stylesheet is its own entry, so the server writes it as a `<link>` on the
/// first paint instead of leaving it to the entry's JavaScript.
fn assert_stylesheet_entry(manifest: &Path) {
    let text = std::fs::read_to_string(manifest).expect("read the client manifest");
    assert!(
        text.contains("\"src/app.css\":"),
        "the client manifest must list src/app.css as its own entry:\n{text}"
    );
}

fn assert_home_heading(frontend: &Path) {
    let script = r#"
import assert from 'node:assert/strict'
import { Server } from 'node:http'
import { pathToFileURL } from 'node:url'

// An ephemeral port lets other worktrees run their own SSR checks.
const listen = Server.prototype.listen
let worker
const listening = new Promise((resolve) => {
  Server.prototype.listen = function (options, ...args) {
    worker = this
    this.once('listening', resolve)
    return listen.call(this, { ...options, port: 0, host: '127.0.0.1' }, ...args)
  }
})
const deadline = setTimeout(() => {
  console.error('SSR worker did not finish the home heading check within 30 seconds')
  process.exit(1)
}, 30_000)
try {
  await import(pathToFileURL(`${process.cwd()}/bootstrap/ssr/ssr.js`))
  await listening
  const page = {
    component: 'Home', url: '/', version: null, clearHistory: false, encryptHistory: false, flash: {},
    props: {
      title: 'SSR Fixture', message: 'First visit', root: '',
      auth: { user: null }, errors: {},
    },
  }
  const response = await fetch(`http://127.0.0.1:${worker.address().port}/render`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(page), signal: AbortSignal.timeout(10_000),
  })
  assert.equal(response.status, 200, await response.clone().text())
  const rendered = await response.json()
  const body = rendered.body.replace(/<!--[\s\S]*?-->/g, '')
  assert.match(body, /<h1[^>]*>Welcome to SSR Fixture<\/h1>/)
  assert.doesNotMatch(body, /<h1[^>]*>welcome<\/h1>/)
} finally {
  clearTimeout(deadline)
  if (worker) await new Promise((resolve) => worker.close(resolve))
}
"#;
    run(frontend, "node", &["--input-type=module", "--eval", script]);
}

#[test]
#[ignore = "needs node, npm and the network: runs npm install, check, build and build:ssr"]
fn kit_svelte_type_checks_and_builds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let frontend = scaffold(&tmp, "kitcheck", "svelte");

    run(&frontend, "npm", &["install", "--no-audit", "--no-fund"]);
    assert!(
        frontend
            .join("node_modules/@inertiajs/vite/package.json")
            .is_file(),
        "the kit must install the Inertia Vite plugin"
    );

    run(&frontend, "npm", &["run", "check"]);
    run(&frontend, "npm", &["run", "build"]);
    assert!(
        frontend
            .join("../public/assets/.vite/manifest.json")
            .is_file(),
        "`npm run build` must write the manifest the server reads"
    );
    assert_stylesheet_entry(&frontend.join("../public/assets/.vite/manifest.json"));

    run(&frontend, "npm", &["run", "build:ssr"]);
    assert!(
        frontend.join("bootstrap/ssr/ssr.js").is_file(),
        "`npm run build:ssr` must land the bundle at frontend/bootstrap/ssr/ssr.js, \
         where `suprnova ssr:start` reads it"
    );
    assert_home_heading(&frontend);
}

/// The React kit.
mod react {
    use super::run;

    const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

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
        super::assert_stylesheet_entry(
            &tmp.path()
                .join("kit-react/public/assets/.vite/manifest.json"),
        );
        run(&frontend, "npm", &["run", "build:ssr"]);
        assert!(
            frontend.join("bootstrap/ssr/ssr.js").is_file(),
            "`npm run build:ssr` must land the SSR bundle at frontend/bootstrap/ssr/ssr.js, \
             the path `suprnova ssr:start` reads"
        );
        super::assert_home_heading(&frontend);
    }
}

/// The Vue kit.
mod vue {
    use super::run;

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
        super::assert_home_heading(&frontend);
        let manifest = tmp.path().join("kit_vue/public/assets/.vite/manifest.json");
        assert!(
            manifest.is_file(),
            "`npm run build` must write the client manifest at {}",
            manifest.display()
        );
        super::assert_stylesheet_entry(&manifest);
    }
}
