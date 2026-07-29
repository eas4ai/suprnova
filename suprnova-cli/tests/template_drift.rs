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
    let mut seen = 0usize;
    visit(&templates, &mut |path, body| {
        seen += 1;
        for line in body.lines() {
            if line.contains("tag = \"v") && !line.contains("{framework_tag}") {
                offenders.push(format!("{}: {}", path.display(), line.trim()));
            }
        }
    });
    assert!(
        seen > 0,
        "walked zero template files — did src/templates/files move? \
         This test passes vacuously when the tree is not found."
    );
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

/// `SessionToken`'s `Display`/`to_string()` deliberately prints the
/// literal string `[REDACTED]` (torii-core `session/mod.rs`) so a secret
/// never leaks into logs. The login handler must call `expose_secret()`
/// instead — the accessor Torii documents for "transmission to client" —
/// or the API starter's login endpoint hands every client the literal
/// string `[REDACTED]` as its bearer token, which can never authenticate
/// anything. Caught by curling a running scaffold: `POST /api/auth/login`
/// returned `{"token":"[REDACTED]"}` verbatim.
#[test]
fn api_starter_login_exposes_the_real_token() {
    let tpl = read("src/templates/files/api/src/controllers/users.rs.tpl");
    assert!(
        !tpl.contains("token.to_string()"),
        "login must not call SessionToken::to_string() — Display redacts \
         the value to \"[REDACTED]\" by design; template was:\n{tpl}"
    );
    assert!(
        tpl.contains("token.expose_secret()"),
        "login must call SessionToken::expose_secret() to hand the real \
         token to the client; template was:\n{tpl}"
    );
}

#[test]
fn api_user_routes_are_behind_an_auth_gate() {
    // `BearerTokenMiddleware` populates the authenticated user when a valid
    // token is present and *never* rejects — it documents this at
    // torii_integration/middleware.rs:18-19. So a route that carries no
    // explicit gate is anonymous, and `UserResource` serializes `email`.
    // Through v0.7.2 a stock `suprnova new x --api` served every user's
    // email address to unauthenticated callers.
    let tpl = read("src/templates/files/api/src/routes.rs.tpl");

    assert!(
        tpl.contains("AuthMiddleware::new()"),
        "api routes template must gate the user routes with \
         AuthMiddleware::new(); got:\n{tpl}"
    );

    // Slice the group's BALANCED body, not "everything after `group!`".
    // A to-end-of-file slice passes even when the routes sit outside the
    // group entirely — a false pass on the test guarding a security fix.
    let after = tpl
        .split_once("group!")
        .expect("template must contain a group!")
        .1;
    let open = after.find('{').expect("group! must have a body");
    let mut depth = 0usize;
    let mut end = None;
    for (i, ch) in after[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &after[open..=end.expect("group! body must be balanced")];

    assert!(
        body.contains("list_users") && body.contains("show_user"),
        "both list_users and show_user must sit INSIDE the gated group body; \
         body was:\n{body}"
    );
}

#[test]
fn api_auth_routes_stay_public() {
    // Register and login must NOT be gated — gating them would make the
    // starter impossible to bootstrap.
    let tpl = read("src/templates/files/api/src/routes.rs.tpl");
    let public = tpl
        .split("group!")
        .next()
        .expect("template has content before the first group!");
    assert!(
        public.contains("controllers::users::register")
            && public.contains("controllers::users::login"),
        "register and login must remain outside the gated group; \
         ungated section was:\n{public}"
    );
}

/// Every `MAIL_*` key the scaffold's `.env` advertises must be one the
/// framework actually reads.
///
/// Through v0.7.2, five of the seven were dead: the scaffold shipped
/// Laravel-style names (`MAIL_HOST`, `MAIL_PORT`, `MAIL_USERNAME`,
/// `MAIL_PASSWORD`, `MAIL_FROM_ADDRESS`) while the transport reads
/// `MAIL_SMTP_*` and the auth flows read `MAIL_FROM`. An operator who
/// filled the file in exactly as instructed got unauthenticated cleartext
/// SMTP to 127.0.0.1 with `MAIL_HOST` ignored, and every password-reset
/// send failed outright because `require_mail_from` hard-errors on an
/// unset `MAIL_FROM`. Nothing caught it because no test read this file.
#[test]
fn every_scaffold_mail_key_is_read_by_the_framework() {
    let env_tpl = read("src/templates/files/root/env.tpl");

    let framework_src = cli_root().join("../framework/src");
    let mut framework_body = String::new();
    visit(&framework_src, &mut |_, body| framework_body.push_str(body));
    assert!(
        !framework_body.is_empty(),
        "could not read framework/src — check the relative path"
    );

    let mut dead = Vec::new();
    for line in env_tpl.lines() {
        let line = line.trim();
        if line.starts_with('#') || !line.starts_with("MAIL_") {
            continue;
        }
        if let Some(key) = line.split('=').next() {
            let key = key.trim();
            if !framework_body.contains(&format!("var(\"{key}\")")) {
                dead.push(key.to_string());
            }
        }
    }

    assert!(
        dead.is_empty(),
        "these MAIL_* keys are advertised in the scaffold .env but never read \
         by the framework, so setting them does nothing: {dead:?}"
    );
}

/// `auth_flows::require_mail_from` returns `Err` when `MAIL_FROM` is unset,
/// so a scaffold that omits it ships broken password reset on day one.
#[test]
fn scaffold_env_ships_the_required_mail_from_key() {
    let env_tpl = read("src/templates/files/root/env.tpl");
    assert!(
        env_tpl.lines().any(|l| l.trim().starts_with("MAIL_FROM=")),
        "env.tpl must ship MAIL_FROM — auth_flows/mod.rs:83-90 returns Err when \
         it is unset, breaking password reset and email verification"
    );
}

// ============================================================================
// Generated Docker Compose — exposure and credentials
// ============================================================================

/// Collect every `ports:` publish line across the compose templates.
///
/// Reads the raw templates rather than the rendered output so a service
/// that is off by default (mailpit, minio) is still covered — the point
/// is that no template can reintroduce a wide bind, including one nobody
/// enables in the default scaffold.
fn compose_publish_lines() -> Vec<(String, String)> {
    let mut lines = Vec::new();
    for name in [
        "docker-compose.yml.tpl",
        "mailpit.service.tpl",
        "minio.service.tpl",
    ] {
        let body = read(&format!("src/templates/files/docker/{name}"));
        for line in body.lines() {
            let trimmed = line.trim();
            // A publish entry is a list item whose value contains a colon
            // inside quotes: `- "127.0.0.1:5432:5432"`.
            if trimmed.starts_with("- \"") && trimmed.contains(':') {
                lines.push((name.to_string(), trimmed.to_string()));
            }
        }
    }
    assert!(
        !lines.is_empty(),
        "found no publish lines at all — did the compose templates move? \
         This test passes vacuously when it cannot find them."
    );
    lines
}

/// `ports: - "5432:5432"` binds 0.0.0.0 on the Docker host. On a laptop
/// on a shared network, or any cloud VM without a firewall, `suprnova new`
/// followed by `docker compose up` then publishes a development database,
/// an unauthenticated Redis, an open SMTP relay (Mailpit accepts any
/// credentials), and MinIO — to the internet.
#[test]
fn compose_publishes_every_port_on_loopback() {
    for (file, line) in compose_publish_lines() {
        assert!(
            line.contains("127.0.0.1") || line.contains("HOST_BIND"),
            "{file}: publish line binds every interface — prefix it with a \
             loopback bind: {line}"
        );
        assert!(
            !line.contains("0.0.0.0"),
            "{file}: publish line binds 0.0.0.0 explicitly: {line}"
        );
    }
}

/// The compose templates shipped `suprnova_secret` and `minioadmin/minioadmin`
/// as literal defaults. A known password is only a development convenience
/// while the port is closed; combined with a wide bind it is a public
/// database. Passwords are now minted per project by
/// `generate_service_password`, so no literal may come back.
#[test]
fn compose_templates_carry_no_literal_credentials() {
    for name in ["docker-compose.yml.tpl", "minio.service.tpl"] {
        let body = read(&format!("src/templates/files/docker/{name}"));
        for literal in ["suprnova_secret", "minioadmin"] {
            assert!(
                !body.contains(literal),
                "{name} still ships the literal credential `{literal}`; \
                 generate it per project instead"
            );
        }
    }
    // And the placeholder the generator substitutes must still be there —
    // otherwise the previous assertion passes simply because the field was
    // deleted.
    let compose = read("src/templates/files/docker/docker-compose.yml.tpl");
    assert!(
        compose.contains("{db_password}"),
        "docker-compose.yml.tpl must carry the {{db_password}} placeholder"
    );
    let minio = read("src/templates/files/docker/minio.service.tpl");
    assert!(
        minio.contains("{minio_password}"),
        "minio.service.tpl must carry the {{minio_password}} placeholder"
    );
}
