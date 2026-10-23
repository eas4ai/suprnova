//! PUB-005 static transport behavior shared by the three Inertia kits.
use std::process::Command;

use suprnova_cli::templates::{Frontend, scaffold_frontend};

#[test]
fn pub_005_all_kits_ship_one_shared_core_and_install_their_adapter() {
    let core = include_str!("../src/templates/files/frontend/static/core.ts.tpl");
    for (kit, main, adapter) in [
        (Frontend::React, "main.tsx", "React"),
        (Frontend::Vue, "main.ts", "Vue"),
        (Frontend::Svelte, "main.ts", "Svelte"),
    ] {
        let dir = tempfile::tempdir().expect("temporary scaffold");
        scaffold_frontend(dir.path(), "static-app", kit).expect("scaffold frontend");
        let src = dir.path().join("frontend/src");
        assert_eq!(
            std::fs::read_to_string(src.join("lib/static/core.ts")).unwrap(),
            core
        );
        let entry = std::fs::read_to_string(src.join(main)).unwrap();
        assert!(entry.contains("import { staticHttpClient } from './lib/static'"));
        assert!(entry.contains("http: staticHttpClient(),"));
        let transport = std::fs::read_to_string(src.join("lib/static/index.ts")).unwrap();
        assert!(transport.contains(&format!("The {adapter} entry")));
        assert!(
            transport
                .contains("createStaticClient(xhrHttpClient, readStaticPublication(document))")
        );
        assert!(!src.join("lib/static/runtime.test.ts").exists());
    }
}

#[test]
fn pub_005_shared_core_executes_public_private_version_and_csrf_paths() {
    let dir = tempfile::tempdir().expect("temporary runtime");
    std::fs::write(
        dir.path().join("core.ts"),
        include_str!("../src/templates/files/frontend/static/core.ts.tpl"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("runtime.test.ts"),
        include_str!("../src/templates/files/frontend/static/runtime.test.ts"),
    )
    .unwrap();
    let run = Command::new("bun")
        .args(["--no-env-file", "test", "runtime.test.ts"])
        .current_dir(dir.path())
        .output()
        .expect("Bun must be installed to execute the static transport contract");
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
