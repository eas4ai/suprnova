//! The Live starter kit (PAR-192): what `suprnova new --frontend live`
//! writes beside the backend every kit shares.
//!
//! The kit has no `frontend/` directory and no Node toolchain. Its pages
//! are server-rendered Live documents: a controller per page under
//! `src/controllers/`, the views and two layouts under `src/views/` and
//! `templates/live/`, and the Live components that act under `src/live/`.
//! The views import the shipped component library, which the scaffold
//! installs under `templates/suprnova-ui/` the way `live:add` does
//! ([`LIBRARY_COMPONENTS`]). `tests/documents.rs` renders every document
//! through the test client as its visitor sees it; the kit's compile check
//! runs it.

use std::fs;
use std::path::Path;

use super::{project_title, render_placeholders};

/// The shipped library components the kit's views import. `suprnova new`
/// installs each one, with every component it depends on, through the plan
/// `live:add` runs, so `suprnova.toml` records them and a later `live:add`
/// keeps them current.
pub const LIBRARY_COMPONENTS: &[&str] = &[
    "account-menu",
    "button",
    "card",
    "checkbox",
    "empty-state",
    "field",
    "form-actions",
    "header-bar",
    "input",
    "list-group",
    "load-more",
    "password-input",
    "search-input",
    "stat-card",
    "textarea",
    "toast",
];

const BOOTSTRAP: &str = include_str!("files/live/src/bootstrap.rs.tpl");

/// Every file the kit writes, by its path in the project. The templates may
/// carry `{package_name}`, which names the kit's components and the gate
/// ability of their stream, and `{project_title}`, the application's name.
const FILES: &[(&str, &str)] = &[
    (".gitignore", include_str!("files/live/gitignore.tpl")),
    (
        "lang/en/app.ftl",
        include_str!("files/live/lang/app.ftl.tpl"),
    ),
    (
        "public/css/app.css",
        include_str!("files/live/public/css/app.css.tpl"),
    ),
    ("src/lib.rs", include_str!("files/live/src/lib.rs.tpl")),
    ("src/bootstrap.rs", BOOTSTRAP),
    (
        "src/routes.rs",
        include_str!("files/live/src/routes.rs.tpl"),
    ),
    ("src/flash.rs", include_str!("files/live/src/flash.rs.tpl")),
    (
        "src/controllers/mod.rs",
        include_str!("files/live/src/controllers/mod.rs.tpl"),
    ),
    (
        "src/controllers/auth.rs",
        include_str!("files/live/src/controllers/auth.rs.tpl"),
    ),
    (
        "src/controllers/dashboard.rs",
        include_str!("files/live/src/controllers/dashboard.rs.tpl"),
    ),
    (
        "src/controllers/email_verification.rs",
        include_str!("files/live/src/controllers/email_verification.rs.tpl"),
    ),
    (
        "src/controllers/errors.rs",
        include_str!("files/live/src/controllers/errors.rs.tpl"),
    ),
    (
        "src/controllers/home.rs",
        include_str!("files/live/src/controllers/home.rs.tpl"),
    ),
    (
        "src/controllers/notes.rs",
        include_str!("files/live/src/controllers/notes.rs.tpl"),
    ),
    (
        "src/controllers/password_reset.rs",
        include_str!("files/live/src/controllers/password_reset.rs.tpl"),
    ),
    (
        "src/controllers/profile.rs",
        include_str!("files/live/src/controllers/profile.rs.tpl"),
    ),
    (
        "src/views/mod.rs",
        include_str!("files/live/src/views/mod.rs.tpl"),
    ),
    (
        "src/live/mod.rs",
        include_str!("files/live/src/live/mod.rs.tpl"),
    ),
    (
        "src/live/forgot_password_form.rs",
        include_str!("files/live/src/live/forgot_password_form.rs.tpl"),
    ),
    (
        "src/live/login_form.rs",
        include_str!("files/live/src/live/login_form.rs.tpl"),
    ),
    (
        "src/live/note_form.rs",
        include_str!("files/live/src/live/note_form.rs.tpl"),
    ),
    (
        "src/live/note_list.rs",
        include_str!("files/live/src/live/note_list.rs.tpl"),
    ),
    (
        "src/live/note_stats.rs",
        include_str!("files/live/src/live/note_stats.rs.tpl"),
    ),
    (
        "src/live/profile_name.rs",
        include_str!("files/live/src/live/profile_name.rs.tpl"),
    ),
    (
        "src/live/register_form.rs",
        include_str!("files/live/src/live/register_form.rs.tpl"),
    ),
    (
        "src/live/reset_password_form.rs",
        include_str!("files/live/src/live/reset_password_form.rs.tpl"),
    ),
    (
        "src/live/verify_email.rs",
        include_str!("files/live/src/live/verify_email.rs.tpl"),
    ),
    (
        "tests/documents.rs",
        include_str!("files/live/tests/documents.rs.tpl"),
    ),
    (
        "templates/live/layouts/head.html",
        include_str!("files/live/templates/layouts/head.html.tpl"),
    ),
    (
        "templates/live/layouts/toast.html",
        include_str!("files/live/templates/layouts/toast.html.tpl"),
    ),
    (
        "templates/live/layouts/guest.html",
        include_str!("files/live/templates/layouts/guest.html.tpl"),
    ),
    (
        "templates/live/layouts/app.html",
        include_str!("files/live/templates/layouts/app.html.tpl"),
    ),
    (
        "templates/live/pages/home.html",
        include_str!("files/live/templates/pages/home.html.tpl"),
    ),
    (
        "templates/live/pages/login.html",
        include_str!("files/live/templates/pages/login.html.tpl"),
    ),
    (
        "templates/live/pages/register.html",
        include_str!("files/live/templates/pages/register.html.tpl"),
    ),
    (
        "templates/live/pages/forgot-password.html",
        include_str!("files/live/templates/pages/forgot-password.html.tpl"),
    ),
    (
        "templates/live/pages/reset-password.html",
        include_str!("files/live/templates/pages/reset-password.html.tpl"),
    ),
    (
        "templates/live/pages/verify-email.html",
        include_str!("files/live/templates/pages/verify-email.html.tpl"),
    ),
    (
        "templates/live/pages/dashboard.html",
        include_str!("files/live/templates/pages/dashboard.html.tpl"),
    ),
    (
        "templates/live/pages/notes-index.html",
        include_str!("files/live/templates/pages/notes-index.html.tpl"),
    ),
    (
        "templates/live/pages/notes-show.html",
        include_str!("files/live/templates/pages/notes-show.html.tpl"),
    ),
    (
        "templates/live/pages/error.html",
        include_str!("files/live/templates/pages/error.html.tpl"),
    ),
    (
        "templates/live/forgot_password_form.html",
        include_str!("files/live/templates/components/forgot_password_form.html.tpl"),
    ),
    (
        "templates/live/login_form.html",
        include_str!("files/live/templates/components/login_form.html.tpl"),
    ),
    (
        "templates/live/note_form.html",
        include_str!("files/live/templates/components/note_form.html.tpl"),
    ),
    (
        "templates/live/note_list.html",
        include_str!("files/live/templates/components/note_list.html.tpl"),
    ),
    (
        "templates/live/note_stats.html",
        include_str!("files/live/templates/components/note_stats.html.tpl"),
    ),
    (
        "templates/live/profile_name.html",
        include_str!("files/live/templates/components/profile_name.html.tpl"),
    ),
    (
        "templates/live/register_form.html",
        include_str!("files/live/templates/components/register_form.html.tpl"),
    ),
    (
        "templates/live/reset_password_form.html",
        include_str!("files/live/templates/components/reset_password_form.html.tpl"),
    ),
    (
        "templates/live/verify_email.html",
        include_str!("files/live/templates/components/verify_email.html.tpl"),
    ),
];

/// The ports and frontend section of `env.tpl` an Inertia kit's `.env`
/// carries, which the Live kit's replaces with [`LIVE_ENV_SECTION`].
const INERTIA_ENV_SECTION: &str =
    "# Backend + Vite ports. Distinctive defaults to dodge the universally
# squatted 8080/5173. `suprnova serve` treats these as a base and scans
# upward if they're busy, so two Suprnova apps can run at once without a
# clash. Set SERVER_PORT explicitly (or rely on $PORT in production) to
# pin the backend port.
SERVER_HOST=127.0.0.1
SERVER_PORT=8765

VITE_PORT=5765

# Which frontend this project was generated with. The server does not
# depend on this - `bootstrap.rs` pins the same value on the InertiaConfig
# it installs - but the framework's `Frontend::detect_from_env()` and the
# `suprnova` CLI's own generators read it, and it is the documented
# contract (manual/env-vars.md).
SUPRNOVA_FRONTEND={frontend}
";

/// The Live kit's section in place of [`INERTIA_ENV_SECTION`]: no Vite
/// port, and a `SUPRNOVA_FRONTEND` that says what reads it.
const LIVE_ENV_SECTION: &str =
    "# Backend port. A distinctive default to dodge the universally squatted
# 8080. `suprnova serve` treats it as a base and scans upward if it is
# busy, so two Suprnova apps can run at once without a clash. Set
# SERVER_PORT explicitly (or rely on $PORT in production) to pin it.
SERVER_HOST=127.0.0.1
SERVER_PORT=8765

# The starter kit this project was generated with: the Live kit, whose
# pages are server-rendered Live documents. The server does not read it,
# since this project installs no Inertia layer, but the `suprnova` CLI's
# generators do: `make:inertia` refuses in a Live project.
SUPRNOVA_FRONTEND={frontend}

# Session blocking: the requests of one session run one at a time, with a
# lock taken through the cache store. A Live request that refreshes an
# open dashboard can otherwise write a session it loaded before a saved
# note kept its toast, and drop the toast.
SESSION_BLOCK=true
";

/// `env.tpl` with the Live kit's ports and frontend section.
pub fn env_section(template: &str) -> String {
    template.replacen(INERTIA_ENV_SECTION, LIVE_ENV_SECTION, 1)
}

/// The kit's `src/bootstrap.rs`: the database, the auth provider, the Live
/// registry and its gates, and an HTTP stack with no Inertia layer.
pub fn bootstrap() -> &'static str {
    BOOTSTRAP
}

/// Writes every file of the kit into `project_path`, which `suprnova new`
/// has created with the shared backend.
pub fn scaffold(project_path: &Path, project_name: &str, package_name: &str) -> Result<(), String> {
    let title = project_title(project_name);
    for (path, template) in FILES {
        let target = project_path.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        let content = render_placeholders(
            template,
            &[
                ("{package_name}", package_name),
                ("{project_title}", &title),
            ],
        );
        fs::write(&target, content).map_err(|e| format!("Failed to write {path}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Live `.env` drops the Vite port only when `env.tpl` still holds
    /// the section this kit replaces; an edit there must reach this file.
    #[test]
    fn env_tpl_holds_the_section_the_live_kit_replaces() {
        let template = include_str!("files/root/env.tpl");
        assert!(template.contains(INERTIA_ENV_SECTION));
        let live = env_section(template);
        assert!(!live.contains("VITE_PORT"), "{live}");
        assert!(live.contains(LIVE_ENV_SECTION));
    }

    /// Every library component a kit template imports is one the scaffold
    /// installs, and every one it installs is imported somewhere.
    #[test]
    fn the_installed_components_are_the_ones_the_templates_import() {
        let mut imported = std::collections::BTreeSet::new();
        for (path, template) in FILES {
            for part in template.split("suprnova-ui/").skip(1) {
                let name: String = part
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                    .collect();
                if !name.is_empty() {
                    imported.insert(name);
                }
            }
            assert!(
                !path.ends_with(".tpl"),
                "{path} is a project path, not a template name"
            );
        }
        let installed: std::collections::BTreeSet<String> = LIBRARY_COMPONENTS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(imported, installed);
    }

    /// Every component the kit installs is one the CLI ships.
    #[test]
    fn every_installed_component_is_shipped() {
        for name in LIBRARY_COMPONENTS {
            assert!(
                crate::registry::fetch::COMPONENTS
                    .iter()
                    .any(|component| component.directory == *name),
                "{name} is not a shipped component"
            );
        }
    }

    /// Rendering resolves every placeholder the templates carry.
    #[test]
    fn rendering_leaves_no_placeholder() {
        for (path, template) in FILES {
            let rendered = render_placeholders(
                template,
                &[("{package_name}", "demo"), ("{project_title}", "Demo")],
            );
            for placeholder in ["{package_name}", "{project_title}"] {
                assert!(
                    !rendered.contains(placeholder),
                    "{path} keeps {placeholder}"
                );
            }
        }
    }
}
