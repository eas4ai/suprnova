//! `suprnova make:view <name> [--force] [--test]` - scaffold a checked
//! view, Laravel's `make:view`.
//!
//! `admin.dashboard` and `admin/dashboard` both write the template
//! `templates/admin/dashboard.html` and a `#[suprnova::view]` struct naming
//! it in `src/views/admin/dashboard.rs`, declaring the modules on the way
//! (`pub mod views;` in `src/lib.rs`, `pub mod admin;` in
//! `src/views/mod.rs`, `pub mod dashboard;` in `src/views/admin/mod.rs`).
//! An existing template, view file or test is kept unless `--force` is
//! given; every file is planned before anything is written, and the run
//! writes all of them or none.

use console::style;
use std::path::{Path, PathBuf};

use crate::commands::generator::{self, WriteSet};
use crate::commands::live_make::Naming;
use crate::templates;
use crate::ui;

pub fn run(name: String, force: bool, test: bool) {
    if let Err(e) = run_inner(&name, force, test) {
        ui::error(&e);
        std::process::exit(1);
    }
}

/// One view's names.
struct Target {
    /// The name's segments as written (`admin`, `dashboard`), which the
    /// template path keeps, as Laravel's view path does.
    segments: Vec<String>,
    /// The same segments as module names (`admin`, `dashboard`).
    modules: Vec<String>,
    /// The struct (`DashboardView`).
    struct_name: String,
}

impl Target {
    fn parse(raw: &str) -> Result<Self, String> {
        let segments = generator::segments(raw, true)?;
        let mut modules = Vec::with_capacity(segments.len());
        let mut last = None;
        for segment in &segments {
            let naming = Naming::parse(segment).ok_or_else(|| {
                format!(
                    "'{segment}' is not a valid view name segment (in '{raw}'): use letters, \
                     digits, `_` and `-`, start with a letter, and avoid Rust keywords"
                )
            })?;
            modules.push(naming.snake.clone());
            last = Some(naming);
        }
        let struct_name = match last {
            Some(naming) => format!("{}View", naming.pascal),
            None => return Err(format!("'{raw}' is not a valid view name")),
        };
        Ok(Self {
            segments,
            modules,
            struct_name,
        })
    }

    /// `admin/dashboard.html`, the path under `templates/` the
    /// `#[suprnova::view]` attribute names.
    fn view_path(&self) -> String {
        format!("{}.html", self.segments.join("/"))
    }

    /// `src/views/admin/dashboard.rs`.
    fn rust_file(&self) -> PathBuf {
        let mut path = PathBuf::from("src/views");
        for module in &self.modules {
            path.push(module);
        }
        path.set_extension("rs");
        path
    }

    /// `views::admin::dashboard`, the module a test imports the struct from.
    fn module_path(&self) -> String {
        std::iter::once("views")
            .chain(self.modules.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    }
}

fn run_inner(raw: &str, force: bool, test: bool) -> Result<(), String> {
    let target = Target::parse(raw)?;
    let view_path = target.view_path();
    let template_file = Path::new("templates").join(&view_path);
    let rust_file = target.rust_file();
    let stem = target.modules.join("_");
    let test_file = PathBuf::from(format!("tests/{stem}_view.rs"));
    let crate_name = if test {
        Some(generator::test_crate_name()?)
    } else {
        None
    };

    let mut owned = vec![&template_file, &rust_file];
    if test {
        owned.push(&test_file);
    }
    let existing: Vec<String> = owned
        .iter()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect();
    if !existing.is_empty() && !force {
        // Laravel's generators report an existing file and exit 0, as
        // `live:make` and `make:inertia` do.
        ui::warning(&format!(
            "{} already exists; nothing was written (pass --force to overwrite)",
            existing.join(" and ")
        ));
        return Ok(());
    }

    let mut files = WriteSet::default();
    files.put(
        template_file.clone(),
        templates::view_html_template().to_string(),
    )?;
    files.put(
        rust_file.clone(),
        templates::view_template(&view_path, &target.struct_name),
    )?;

    // `pub mod views;` in the crate root, then one `pub mod` per segment,
    // each in the module file of the directory above it.
    let lib_rs = PathBuf::from("src/lib.rs");
    if lib_rs.is_file() {
        files.edit(lib_rs, |current| {
            let content = current?;
            (!generator::declares_module(content, "views"))
                .then(|| generator::insert_declaration(content, "pub mod views;"))
        })?;
    } else {
        ui::warning("src/lib.rs not found: declare `pub mod views;` in your crate root");
    }
    let mut dir = PathBuf::from("src");
    let mut dir_module = "views".to_string();
    for (depth, module) in target.modules.iter().enumerate() {
        let mod_file = generator::module_file(&dir, &dir_module);
        let declaration = format!("pub mod {module};");
        let header = match depth {
            0 => "//! The application's checked views, one per template under `templates/`.\n"
                .to_string(),
            _ => format!(
                "//! The views of `templates/{}/`.\n",
                target.segments[..depth].join("/")
            ),
        };
        files.edit(mod_file, |current| match current {
            Some(content) if generator::declares_module(content, module) => None,
            Some(content) => Some(generator::insert_declaration(content, &declaration)),
            None => Some(format!("{header}\n{declaration}\n")),
        })?;
        dir = dir.join(&dir_module);
        dir_module = module.clone();
    }

    if let Some(crate_name) = crate_name {
        files.put(
            test_file,
            templates::view_test_template(
                &crate_name,
                &target.module_path(),
                &target.struct_name,
                &view_path,
                &format!("{stem}_view_renders"),
            ),
        )?;
    }

    generator::report(&files.apply()?);

    ui::br();
    ui::info(&format!(
        "View {} created for templates/{}",
        style(&target.struct_name).cyan().bold(),
        view_path
    ));
    ui::br();
    ui::hint("Render it from a handler:");
    ui::command(&format!(
        "use crate::{}::{};",
        target.module_path(),
        target.struct_name
    ));
    ui::command("use suprnova::view::ViewTemplate;");
    ui::command(&format!(
        "{} {{ title: \"Dashboard\".into() }}.render_view(&mut html)",
        target.struct_name
    ));
    ui::br();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_and_slashes_name_the_same_view() {
        for raw in ["admin.dashboard", "admin/dashboard"] {
            let target = Target::parse(raw).unwrap();
            assert_eq!(target.view_path(), "admin/dashboard.html");
            assert_eq!(
                target.rust_file(),
                Path::new("src/views/admin/dashboard.rs")
            );
            assert_eq!(target.module_path(), "views::admin::dashboard");
            assert_eq!(target.struct_name, "DashboardView");
        }
    }

    #[test]
    fn a_segment_keeps_its_spelling_in_the_template_path() {
        let target = Target::parse("reports.user-summary").unwrap();
        assert_eq!(target.view_path(), "reports/user-summary.html");
        assert_eq!(
            target.rust_file(),
            Path::new("src/views/reports/user_summary.rs")
        );
        assert_eq!(target.struct_name, "UserSummaryView");
        assert_eq!(
            Target::parse("welcome").unwrap().module_path(),
            "views::welcome"
        );
    }

    #[test]
    fn bad_names_are_refused() {
        for bad in [
            "",
            "admin..x",
            "admin/",
            "admin/type",
            "admin/1st",
            "a b",
            "../x",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad:?}");
        }
    }
}
