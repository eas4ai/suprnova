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

/// Read a path relative to the workspace root, for the few assertions that
/// tie a template to a sibling crate's behaviour rather than to the CLI's
/// own files.
fn read_from_repo(rel: &str) -> String {
    let path = cli_root()
        .parent()
        .expect("suprnova-cli sits inside the workspace")
        .join(rel);
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
///
/// This originally read only `env.tpl`, which is why the fix landed on
/// half the problem: `.env.example` kept all five dead keys for another
/// release. It now checks both, because the committed example is the file
/// a teammate actually copies.
#[test]
fn every_scaffold_mail_key_is_read_by_the_framework() {
    let framework_src = cli_root().join("../framework/src");
    let mut framework_body = String::new();
    visit(&framework_src, &mut |_, body| framework_body.push_str(body));
    assert!(
        !framework_body.is_empty(),
        "could not read framework/src — check the relative path"
    );

    for template in ["env.tpl", "env.example.tpl"] {
        let env_tpl = read(&format!("src/templates/files/root/{template}"));

        let mut dead = Vec::new();
        let mut checked = 0usize;
        for line in env_tpl.lines() {
            let line = line.trim();
            if line.starts_with('#') || !line.starts_with("MAIL_") {
                continue;
            }
            if let Some(key) = line.split('=').next() {
                let key = key.trim();
                checked += 1;
                if !framework_body.contains(&format!("var(\"{key}\")")) {
                    dead.push(key.to_string());
                }
            }
        }

        assert!(
            checked >= 5,
            "{template} yielded only {checked} MAIL_* keys — the scan is \
             broken and this assertion would pass vacuously"
        );
        assert!(
            dead.is_empty(),
            "these MAIL_* keys are advertised in {template} but never read \
             by the framework, so setting them does nothing: {dead:?}"
        );
    }
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

// ============================================================================
// REL-01b — a scaffolded project must be runnable and buildable
// ============================================================================

/// Cargo refuses `cargo run` on a multi-binary package with no
/// `default-run`, and does NOT fall back to the binary named after the
/// package. Verified directly against cargo before this test was written:
///
/// ```text
/// error: `cargo run` could not determine which binary to run.
/// Use the `--bin` option to specify a binary, or the `default-run` manifest key.
/// available binaries: console, twobin
/// ```
///
/// Ten CLI wrappers shell out to `cargo run` inside the user's project, so
/// without this key `suprnova migrate` — and every sibling — failed on a
/// fresh scaffold before doing any work. `scaffold_snapshot` never caught
/// it because `cargo check` does not resolve a default binary.
#[test]
fn multi_binary_templates_declare_a_default_run() {
    for rel in [
        "src/templates/files/backend/Cargo.toml.tpl",
        "src/templates/files/api/Cargo.toml.tpl",
    ] {
        let tpl = read(rel);
        let bins = tpl.matches("[[bin]]").count();
        assert!(
            bins >= 2,
            "{rel}: expected the two-binary shape this test guards, found {bins}"
        );
        assert!(
            tpl.contains(r#"default-run = "{package_name}""#),
            "{rel} declares {bins} binaries but no `default-run`, so `cargo run` \
             in a generated project refuses to pick one"
        );
    }
}

/// The Docker dependency-cache stage stubs a `main` for each binary so the
/// manifest resolves. It stubbed only `cmd/main.rs` while the manifest also
/// declared `console` at `src/bin/console.rs`, so `cargo build` failed in
/// that stage — a hard build failure, not a missed cache.
#[test]
fn docker_cache_stage_stubs_every_declared_binary() {
    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");
    let manifest = read("src/templates/files/backend/Cargo.toml.tpl");

    // Comment lines are stripped before searching. Without this the test
    // passes on prose: the comment above the stub names
    // `src/bin/console.rs` to explain why it is there, which satisfied a
    // naive `contains` even with the stub itself deleted. Caught by
    // teeth-checking this test rather than by reading it.
    let instructions: String = dockerfile
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    // Every `path = "..."` in the manifest's [[bin]] entries must be stubbed.
    let mut checked = 0usize;
    for line in manifest.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("path = \"") else {
            continue;
        };
        let Some(path) = rest.strip_suffix('"') else {
            continue;
        };
        checked += 1;
        assert!(
            instructions.contains(path),
            "the Docker cache stage never creates `{path}`, which the manifest \
             declares as a binary — `cargo build` fails there on the missing target"
        );
    }
    assert!(
        checked >= 2,
        "found {checked} [[bin]] paths in the manifest template; this test is \
         guarding the multi-binary case and passes vacuously below two"
    );
}

/// `npm ci` requires a lockfile and errors without one. The COPY globs the
/// lock as optional, so on a fresh scaffold — which ships no lock — the
/// unconditional `npm ci` failed every image build.
#[test]
fn docker_frontend_install_tolerates_a_missing_lockfile() {
    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");
    let copies_lock_optionally = dockerfile.contains("frontend/package-lock.json*");
    let uses_ci = dockerfile.contains("npm ci");
    if uses_ci && copies_lock_optionally {
        assert!(
            dockerfile.contains("if [ -f package-lock.json ]"),
            "the Dockerfile treats package-lock.json as optional in its COPY but \
             runs `npm ci`, which requires one — a fresh scaffold cannot build"
        );
    }
}

/// Same shape for the Rust lockfile: a bare `COPY … Cargo.lock` fails the
/// build outright when the file does not exist.
#[test]
fn docker_copies_the_rust_lockfile_optionally() {
    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");
    assert!(
        !dockerfile.contains("COPY Cargo.toml Cargo.lock ."),
        "a bare `COPY Cargo.toml Cargo.lock ./` fails when the project has no \
         lockfile; glob it as `Cargo.lock*`"
    );
}

/// `Cargo.lock` was in the scaffold's .gitignore. The generated project is
/// an application, and Cargo's guidance is that applications commit their
/// lockfile — otherwise CI and the production image resolve a different
/// dependency graph than the developer tested.
#[test]
fn scaffold_gitignore_does_not_exclude_the_lockfile() {
    let gitignore = read("src/templates/files/root/gitignore.tpl");
    for line in gitignore.lines() {
        let bare = line.trim();
        assert!(
            bare != "Cargo.lock" && bare != "/Cargo.lock",
            "the scaffold ignores Cargo.lock; a generated app should commit it"
        );
    }
}

/// The printed `docker run` must publish the port the image actually
/// exposes. It said 8080 while the Dockerfile set SERVER_PORT=8765 and
/// EXPOSED 8765, so following the printed command gave a container that
/// looked dead.
#[test]
fn printed_docker_run_port_matches_the_dockerfile() {
    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");
    let docker_init = read("src/commands/docker_init.rs");

    let exposed = dockerfile
        .lines()
        .find_map(|l| l.trim().strip_prefix("EXPOSE ").map(str::trim))
        .expect("the Dockerfile must EXPOSE a port");

    assert!(
        docker_init.contains(&format!("docker run -p {exposed}:{exposed}")),
        "docker_init prints a `docker run -p` that does not match the image's \
         EXPOSE {exposed}"
    );
}

/// The Dockerfile must copy the frontend build from where vite actually
/// writes it. Every scaffolded `vite.config.ts` sets
/// `build.outDir: '../public/assets'`; the Dockerfile copied from
/// `/app/frontend/dist`, which vite never creates, so the image build
/// failed at that COPY *after* `npm run build` had reported success.
///
/// Asserting the two against each other rather than pinning a literal
/// keeps them from drifting apart again in either direction.
#[test]
fn docker_copies_the_frontend_build_from_the_vite_output_dir() {
    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");

    let mut out_dirs = Vec::new();
    for frontend in ["react", "svelte", "vue"] {
        let config = read(&format!(
            "src/templates/files/frontend/{frontend}/vite.config.ts.tpl"
        ));
        let out_dir = config
            .lines()
            .find_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("outDir:")?;
                Some(
                    rest.trim()
                        .trim_matches(|c| c == '\'' || c == '"' || c == ',')
                        .to_string(),
                )
            })
            .unwrap_or_else(|| panic!("{frontend}/vite.config.ts.tpl declares no outDir"));
        out_dirs.push((frontend, out_dir));
    }

    let first = &out_dirs[0].1;
    for (frontend, dir) in &out_dirs {
        assert_eq!(
            dir, first,
            "{frontend} writes its build to `{dir}` while another frontend uses \
             `{first}` — the single Dockerfile cannot copy from both"
        );
    }

    // `../public/assets` from the frontend stage's WORKDIR (/app/frontend)
    // is /app/public/assets.
    let relative = first.strip_prefix("../").unwrap_or_else(|| {
        panic!("expected an outDir relative to the frontend dir, got `{first}`")
    });
    let expected_source = format!("/app/{relative}");

    let copy_line = dockerfile
        .lines()
        .find(|l| l.contains("--from=frontend-builder"))
        .expect("the Dockerfile must copy the frontend build out of its stage");
    assert!(
        copy_line.contains(&expected_source),
        "the Dockerfile copies the frontend build from a path vite does not \
         write. vite outDir is `{first}` (→ `{expected_source}`), but the \
         Dockerfile says:\n  {copy_line}"
    );
}

/// The Rust build stage must have the frontend page sources, because
/// `inertia_response!` resolves them at COMPILE time: it looks for
/// `frontend/src/pages/<component>.{svelte,tsx,jsx,vue}` under
/// `CARGO_MANIFEST_DIR` and fails the build when the file is absent.
///
/// The Dockerfile copied only `cmd/` and `src/` into the backend stage,
/// so through v0.7.2 every scaffolded app died there with "Inertia
/// component 'Home' not found" — the four generated controllers all
/// render a page. Building the frontend in stage 1 does not help; this is
/// a dependency of the *Rust* compile.
///
/// Anchored on the macro's own search path so moving the pages directory
/// has to move both sides together.
#[test]
fn docker_backend_stage_has_the_pages_the_inertia_macro_resolves() {
    let macro_src = read_from_repo("suprnova-macros/src/inertia.rs");

    // The macro builds the directory as `.join("frontend").join("src").join("pages")`.
    assert!(
        macro_src.contains(r#".join("frontend").join("src").join("pages")"#),
        "validate_component_exists no longer resolves frontend/src/pages the \
         way this test assumes — re-derive the expected COPY from its new path"
    );
    let pages_dir = "frontend/src/pages";

    let dockerfile = read("src/templates/files/docker/Dockerfile.tpl");
    let backend_stage = dockerfile
        .split_once("AS backend-builder")
        .expect("the Dockerfile must have a backend-builder stage")
        .1;

    let copies_pages = backend_stage
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter(|l| l.starts_with("COPY ") && !l.contains("--from="))
        .any(|l| l.contains(pages_dir));

    assert!(
        copies_pages,
        "the backend-builder stage never copies `{pages_dir}` into the build \
         context, so `inertia_response!` cannot resolve any page component and \
         the image build fails on a stock scaffold"
    );

    // `.dockerignore` must not take back what the COPY asks for.
    let dockerignore = read("src/templates/files/docker/dockerignore.tpl");
    for line in dockerignore.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let pattern = line.trim_end_matches('/');
        assert!(
            !pages_dir.starts_with(pattern),
            ".dockerignore excludes `{line}`, which removes `{pages_dir}` from \
             the build context that the backend stage must COPY"
        );
    }
}

// ---------------------------------------------------------------------------
// CI-03 — assertions against a real scaffold on disk, before any rewriting
// ---------------------------------------------------------------------------
//
// Everything above reads template files or calls a `templates::*` render
// function directly. Both stop short of the thing a user actually gets:
// `scaffold_snapshot` scaffolds for real but immediately rewrites the
// `suprnova` dependency to a local path before it compiles anything, so the
// tag that ships has never been asserted on disk — which is precisely how
// REL-01a shipped a stale pin.
//
// These scaffold a project and assert against the bytes on disk, with no
// rewriting in between.

/// Scaffold a real project into a temp dir and hand back its path.
fn scaffold_to_disk(tmp: &tempfile::TempDir, name: &str, extra: &[&str]) -> PathBuf {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"));
    cmd.arg("new")
        .arg(name)
        .arg("--no-interaction")
        .arg("--no-git")
        .args(extra)
        .current_dir(tmp.path());
    let out = cmd.output().expect("`suprnova new` should run");
    assert!(
        out.status.success(),
        "`suprnova new {name} {extra:?}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    tmp.path().join(name)
}

/// No `{placeholder}` may survive into a generated project.
///
/// The substitution is `str::replace` against a hand-maintained list of
/// keys, so adding a placeholder to a template without teaching the writer
/// about it emits the literal `{package_name}` into the user's source. That
/// is a compile error at best and a silently wrong value at worst, and
/// nothing checked for it — the render-function tests only assert the two
/// placeholders they already know about.
///
/// Scanning the whole tree catches the ones nobody thought to name.
#[test]
fn a_scaffolded_project_contains_no_unsubstituted_placeholders() {
    let tmp = tempfile::TempDir::new().unwrap();
    for (name, extra) in [("phfull", &[][..]), ("phapi", &["--api"][..])] {
        let project = scaffold_to_disk(&tmp, name, extra);

        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        visit(&project, &mut |path, body| {
            // node_modules is not ours and is not generated from templates.
            if path.components().any(|c| c.as_os_str() == "node_modules") {
                return;
            }
            scanned += 1;
            for (n, line) in body.lines().enumerate() {
                // A surviving placeholder looks like `{lower_snake_case}`.
                // Real code uses braces constantly, so match the shape the
                // templates actually use rather than any brace at all.
                for cap in line.match_indices('{') {
                    let rest = &line[cap.0 + 1..];
                    let Some(end) = rest.find('}') else { continue };
                    let inner = &rest[..end];
                    if !inner.is_empty()
                        && inner
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                        && KNOWN_TEMPLATE_KEYS.contains(&inner)
                    {
                        offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        });

        assert!(
            scanned > 0,
            "scanned zero files in the {name} scaffold — the walk found \
             nothing, so this test would pass vacuously"
        );
        assert!(
            offenders.is_empty(),
            "the {name} scaffold shipped unsubstituted template placeholders:\n{}",
            offenders.join("\n")
        );
    }
}

/// Every placeholder the templates use. A surviving one of these in
/// generated output is unambiguously a substitution bug, where a bare
/// `{name}` in a Rust format string is not.
const KNOWN_TEMPLATE_KEYS: &[&str] = &[
    "package_name",
    "project_name",
    "framework_tag",
    "description",
    "db_password",
    "minio_password",
    "app_key",
];

/// The tag in a scaffold on disk must be the tag this build would release.
///
/// `rendered_backend_cargo_toml_pins_the_running_version` asserts the same
/// thing about `templates::cargo_toml(...)`, but that calls the render
/// function directly and so cannot see anything the writer does afterwards.
/// This reads the file the user gets.
#[test]
fn a_scaffolded_manifest_on_disk_pins_the_running_tag() {
    let tmp = tempfile::TempDir::new().unwrap();
    let expected = format!("tag = \"{}\"", expected_tag());

    for (name, extra) in [("tagfull", &[][..]), ("tagapi", &["--api"][..])] {
        let project = scaffold_to_disk(&tmp, name, extra);
        let manifest = fs::read_to_string(project.join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("read {name}/Cargo.toml: {e}"));
        assert!(
            manifest.contains(&expected),
            "the {name} scaffold's Cargo.toml on disk must pin {expected}; got:\n{manifest}"
        );
    }
}

// ---------------------------------------------------------------------
// Env templates must name variables the code actually reads.
// ---------------------------------------------------------------------

/// Walk a directory for `.rs` files, skipping the template tree — a
/// template must not be allowed to vouch for itself.
fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "templates") {
                continue;
            }
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `SCREAMING_SNAKE` string literal appearing in framework or CLI
/// source, outside comments.
///
/// Deliberately loose. The precise question — "does something read this
/// variable?" — has no cheap syntactic answer, because the reads go
/// through at least five different call shapes (`std::env::var`, `env`,
/// `env_optional`, `env_strict`, `bool_env`) plus `envy`, which derives
/// names from struct fields and leaves no literal at all. A scanner tight
/// enough to model that would be wrong more often than the thing it
/// checks.
///
/// Looseness is safe *in this direction*: it can only make the assertion
/// weaker, never wrong. A name that appears nowhere in any source file is
/// unambiguously dead, and that is exactly the defect this catches.
/// Comment lines are stripped so prose mentioning a variable cannot vouch
/// for it — the dead keys below were all named in doc comments.
fn env_names_mentioned_in_source() -> std::collections::BTreeSet<String> {
    let root = cli_root()
        .parent()
        .expect("suprnova-cli sits inside the workspace")
        .to_path_buf();

    let mut files = Vec::new();
    rust_sources(&root.join("framework").join("src"), &mut files);
    rust_sources(&root.join("suprnova-cli").join("src"), &mut files);
    assert!(
        files.len() > 50,
        "expected to scan the framework and CLI sources, found only {} files — \
         the walk is broken and this test would pass vacuously",
        files.len()
    );

    let mut names = std::collections::BTreeSet::new();
    for file in files {
        let src = fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {file:?}: {e}"));
        for line in src.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for literal in line.split('"').skip(1).step_by(2) {
                if literal.len() > 3
                    && literal
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    && literal.starts_with(|c: char| c.is_ascii_uppercase())
                {
                    names.insert(literal.to_string());
                }
            }
        }
    }
    names
}

/// Variables a scaffolded `.env` assigns, in template order.
fn env_keys_assigned(template: &str) -> Vec<String> {
    template
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, _)| key.trim().to_string())
        .filter(|key| {
            !key.is_empty()
                && key.starts_with(|c: char| c.is_ascii_uppercase())
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .collect()
}

/// A scaffolded `.env` that names variables nothing reads is worse than an
/// empty one: the developer configures it, believes it took effect, and
/// discovers otherwise through behaviour rather than an error.
///
/// This shipped twice. First in the `.env` template, fixed in `a56a1a9e`
/// ("scaffold .env advertised mail keys the framework never reads"). Then
/// it turned out **`.env.example` still carried the same dead keys** —
/// `MAIL_HOST`, `MAIL_PORT`, `MAIL_USERNAME`, `MAIL_PASSWORD` and
/// `MAIL_FROM_ADDRESS`, against a transport that reads `MAIL_SMTP_HOST`,
/// `MAIL_SMTP_PORT`, `MAIL_SMTP_USER`, `MAIL_SMTP_PASS` and `MAIL_FROM`.
///
/// That was the worse half to miss. `.env` is gitignored; `.env.example`
/// is committed, so it is the file a teammate copies and the file CI
/// reads. And `MAIL_FROM` is not cosmetic — the auth flows refuse to send
/// without it, so a developer following `.env.example` got password reset
/// failing with "MAIL_FROM environment variable is not set" while their
/// `.env.example` plainly showed a from-address configured.
///
/// One fixed file and one missed file is exactly what a per-file review
/// misses and a mechanical sweep does not.
#[test]
fn env_templates_only_name_variables_the_code_reads() {
    let known = env_names_mentioned_in_source();

    for template in ["env.tpl", "env.example.tpl"] {
        let src = read(&format!("src/templates/files/root/{template}"));
        let keys = env_keys_assigned(&src);
        assert!(
            keys.len() > 5,
            "{template} parsed to only {} assignments — the parser is broken \
             and this test would pass vacuously",
            keys.len()
        );

        let dead: Vec<&String> = keys.iter().filter(|k| !known.contains(*k)).collect();
        assert!(
            dead.is_empty(),
            "{template} assigns {dead:?}, which appear nowhere in the framework \
             or CLI sources. Either the variable was renamed and the template \
             was not updated, or the template is advertising a knob that does \
             not exist. Both leave a developer configuring something that \
             silently does nothing."
        );
    }
}

/// The two env templates are copies of one another with different values,
/// so a variable added to one and forgotten in the other is the specific
/// mistake that produced the defect above. Compare the key *sets*.
#[test]
fn the_two_env_templates_agree_on_which_variables_exist() {
    let live: std::collections::BTreeSet<String> =
        env_keys_assigned(&read("src/templates/files/root/env.tpl"))
            .into_iter()
            .collect();
    let example: std::collections::BTreeSet<String> =
        env_keys_assigned(&read("src/templates/files/root/env.example.tpl"))
            .into_iter()
            .collect();

    let missing_from_example: Vec<&String> = live.difference(&example).collect();
    let missing_from_live: Vec<&String> = example.difference(&live).collect();

    assert!(
        missing_from_example.is_empty() && missing_from_live.is_empty(),
        "env.tpl and env.example.tpl must document the same variables.\n  \
         in env.tpl but not env.example.tpl: {missing_from_example:?}\n  \
         in env.example.tpl but not env.tpl: {missing_from_live:?}\n\
         `.env` is gitignored and `.env.example` is committed, so a variable \
         present in only one of them is a variable half the team never sees."
    );
}
