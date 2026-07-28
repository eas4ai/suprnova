//! Scaffold templates must stay consistent with the framework they target.
//!
//! Template drift is silent by construction: `scaffold_snapshot` rewrites
//! the `suprnova` dependency to a local path before compiling, so a stale
//! git tag never reaches a compiler, and no test reads the `.env` template
//! at all. Both defects shipped through v0.7.2. These assertions are the
//! mechanical guard that was missing.

use std::fs;
use std::path::{Path, PathBuf};

fn cli_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = cli_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The tag both scaffolds must pin, derived the same way the scaffolder
/// derives it. `suprnova-cli` inherits `version.workspace = true` and
/// `release.sh` tags `v<workspace version>`.
fn expected_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

#[test]
fn backend_cargo_template_has_no_hardcoded_tag() {
    let tpl = read("src/templates/files/backend/Cargo.toml.tpl");
    assert!(
        tpl.contains(r#"tag = "{framework_tag}""#),
        "backend Cargo.toml.tpl must template its tag, not hardcode one; got:\n{tpl}"
    );
}

#[test]
fn api_cargo_template_has_no_hardcoded_tag() {
    let tpl = read("src/templates/files/api/Cargo.toml.tpl");
    assert!(
        tpl.contains(r#"tag = "{framework_tag}""#),
        "api Cargo.toml.tpl must template its tag, not hardcode one; got:\n{tpl}"
    );
}

#[test]
fn no_scaffold_template_pins_a_literal_version_tag() {
    // Catches the general case: any template that hardcodes `tag = "vX.Y.Z"`
    // will be stale the moment the next release ships.
    let templates = cli_root().join("src/templates/files");
    let mut offenders = Vec::new();
    visit(&templates, &mut |path, body| {
        for line in body.lines() {
            if line.contains("tag = \"v") && !line.contains("{framework_tag}") {
                offenders.push(format!("{}: {}", path.display(), line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "templates must derive the framework tag, not hardcode it:\n{}",
        offenders.join("\n")
    );
}

/// Walk every file under `dir`, handing each readable one to `f`.
fn visit(dir: &Path, f: &mut impl FnMut(&Path, &str)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit(&path, f);
        } else if let Ok(body) = fs::read_to_string(&path) {
            f(&path, &body);
        }
    }
}

#[test]
fn rendered_backend_cargo_toml_pins_the_running_version() {
    let rendered = suprnova_cli::templates::cargo_toml("my_app", "A test app", "");
    let expected = format!("tag = \"{}\"", expected_tag());
    assert!(
        rendered.contains(&expected),
        "backend Cargo.toml must pin {expected}; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("{framework_tag}"),
        "placeholder left unsubstituted — check the format! argument name"
    );
}

#[test]
fn rendered_api_cargo_toml_pins_the_running_version() {
    let rendered = suprnova_cli::templates::api::cargo_toml("my_api", "my-api");
    let expected = format!("tag = \"{}\"", expected_tag());
    assert!(
        rendered.contains(&expected),
        "api Cargo.toml must pin {expected}; rendered:\n{rendered}"
    );
    assert!(
        !rendered.contains("{framework_tag}"),
        "placeholder left unsubstituted — check the .replace() call"
    );
}

/// The API starter must not create a table Torii already owns.
///
/// Torii's own migration creates `users` with `.if_not_exists()`, a
/// string primary key, and name/password_hash/email_verified_at columns.
/// The starter shipped its own `users` migration with an autoincrement
/// bigint id and three columns, against the SAME connection. Whichever
/// ran first won and the other silently skipped, so Torii's columns never
/// existed and `POST /api/auth/register` died on `no such column:
/// users.name`. The two schemas can never be one table.
#[test]
fn api_starter_does_not_claim_the_torii_users_table() {
    let migration = read("src/templates/files/api/src/migrations/create_users_table.rs.tpl");
    // Check the enum *declaration*, not a bare `Users::Table` substring:
    // the fix renames the enum to `AppUsers`, and `AppUsers::Table`
    // itself contains the substring `Users::Table`, which would make a
    // naive `.contains("Users::Table")` check false-fail forever after a
    // correct fix.
    assert!(
        !migration.contains("\"users\"") && !migration.contains("enum Users {"),
        "the api starter must not create a table named `users` — Torii owns \
         that name and creates it with an incompatible schema; migration was:\n{migration}"
    );

    let model = read("src/templates/files/api/src/models/user.rs.tpl");
    assert!(
        !model.contains(r#"table_name = "users""#),
        "the api starter's User model must not map to `users`; model was:\n{model}"
    );
    assert!(
        model.contains(r#"table_name = "app_users""#),
        "the api starter's User model must map to `app_users`; model was:\n{model}"
    );
}
