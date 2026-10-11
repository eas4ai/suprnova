//! `suprnova make:middleware <name> [--test]` - scaffold a middleware.
//!
//! A nested name (`Admin/EnsureRole`) writes under a subdirectory of
//! `src/middleware` and declares the modules on the way to it, as
//! Laravel's `make:middleware Admin/EnsureRole` writes under
//! `app/Http/Middleware/Admin`. Every file is planned before anything is
//! written, and the run writes all of them or none.

use console::style;
use std::path::PathBuf;

use crate::commands::generator::{self, WriteSet};
use crate::commands::live_make::Naming;
use crate::templates;
use crate::ui;

pub fn run(name: String, test: bool) {
    if let Err(e) = run_inner(&name, test) {
        ui::error(&e);
        std::process::exit(1);
    }
}

/// One middleware's names: the directories it sits in, its file stem and
/// its struct.
struct Target {
    /// Snake-case directory modules under `src/middleware`, outermost first.
    dirs: Vec<String>,
    /// File stem and module name (`ensure_role`).
    file: String,
    /// Struct name (`EnsureRoleMiddleware`).
    struct_name: String,
    /// The name the doc comment shows (`EnsureRole`).
    display: String,
}

impl Target {
    fn parse(raw: &str) -> Result<Self, String> {
        let mut segments = generator::segments(raw, false)?;
        let last = segments.pop().unwrap_or_default();
        let mut dirs = Vec::with_capacity(segments.len());
        for segment in &segments {
            let naming = Naming::parse(segment)
                .ok_or_else(|| format!("'{segment}' is not a valid module name (in '{raw}')"))?;
            dirs.push(naming.snake);
        }
        let base = last.strip_suffix("Middleware").unwrap_or(&last);
        if !is_valid_identifier(base) {
            return Err(format!("'{last}' is not a valid Rust identifier"));
        }
        let file = Naming::parse(base)
            .ok_or_else(|| format!("'{last}' is not a valid middleware name"))?
            .snake;
        let display = to_pascal_case(base);
        Ok(Self {
            dirs,
            file,
            struct_name: format!("{display}Middleware"),
            display,
        })
    }

    /// `src/middleware/admin` for `Admin/EnsureRole`.
    fn dir(&self) -> PathBuf {
        self.dirs
            .iter()
            .fold(PathBuf::from("src/middleware"), |dir, name| dir.join(name))
    }

    /// `middleware::admin`, the path a test imports the struct through.
    fn module_path(&self) -> String {
        std::iter::once("middleware")
            .chain(self.dirs.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    }
}

fn run_inner(raw: &str, test: bool) -> Result<(), String> {
    let target = Target::parse(raw)?;
    let dir = target.dir();
    let middleware_file = dir.join(format!("{}.rs", target.file));
    if middleware_file.exists() {
        return Err(format!(
            "Middleware '{}' already exists at {}",
            target.struct_name,
            middleware_file.display()
        ));
    }

    let mut files = WriteSet::default();
    files.put(
        middleware_file,
        templates::middleware_template(&target.display, &target.struct_name),
    )?;

    // The module file of the middleware's own directory declares it and
    // re-exports the struct; each directory above declares the next one.
    let own_mod = innermost_mod_file(&target.dirs);
    let header = match target.dirs.last() {
        Some(dir_name) => format!("//! {} middleware\n", to_pascal_case(dir_name)),
        None => "//! Application middleware\n".to_string(),
    };
    let (file, struct_name) = (target.file.clone(), target.struct_name.clone());
    let mut declared = Ok(());
    files.edit(own_mod.clone(), |current| match current {
        Some(content) => match with_middleware(content, &file, &struct_name) {
            Ok(updated) => Some(updated),
            Err(e) => {
                declared = Err(e);
                None
            }
        },
        None => Some(format!(
            "{header}\nmod {file};\n\npub use {file}::{struct_name};\n"
        )),
    })?;
    declared.map_err(|e| format!("Failed to update {}: {e}", own_mod.display()))?;

    let mut parent = PathBuf::from("src/middleware");
    let mut parent_mod = PathBuf::from("src/middleware/mod.rs");
    let mut parent_header = "//! Application middleware\n".to_string();
    for name in &target.dirs {
        let declaration = format!("pub mod {name};");
        files.edit(parent_mod.clone(), |current| match current {
            Some(content) if generator::declares_module(content, name) => None,
            Some(content) => Some(generator::insert_declaration(content, &declaration)),
            None => Some(format!("{parent_header}\n{declaration}\n")),
        })?;
        parent_mod = generator::module_file(&parent, name);
        parent = parent.join(name);
        parent_header = format!("//! {} middleware\n", to_pascal_case(name));
    }

    if test {
        let crate_name = generator::test_crate_name()?;
        let stem = target
            .dirs
            .iter()
            .chain([&target.file])
            .cloned()
            .collect::<Vec<_>>()
            .join("_");
        let test_file = PathBuf::from(format!("tests/{stem}_middleware.rs"));
        if test_file.exists() {
            return Err(format!("{} already exists", test_file.display()));
        }
        files.put(
            test_file,
            templates::middleware_test_template(
                &crate_name,
                &target.module_path(),
                &target.struct_name,
                &format!("{stem}_middleware_passes_the_request_to_the_route"),
            ),
        )?;
    }

    generator::report(&files.apply()?);

    let use_path = format!("crate::{}::{}", target.module_path(), target.struct_name);
    ui::br();
    ui::info(&format!(
        "Middleware {} created",
        style(&target.struct_name).cyan().bold()
    ));
    ui::br();
    ui::hint("Import and use in routes:");
    ui::command(&format!("use {use_path};"));
    ui::command(&format!(
        ".get(\"/path\", handler).middleware({})",
        target.struct_name
    ));
    ui::br();
    ui::hint("Or apply globally in bootstrap.rs:");
    ui::command(&format!("global_middleware!({use_path})"));
    ui::br();
    Ok(())
}

/// The module file of the directory the middleware is written to:
/// `src/middleware/mod.rs`, or for `Admin/EnsureRole` the `admin` module's
/// file (`admin/mod.rs`, or `admin.rs` when the project uses that layout).
fn innermost_mod_file(dirs: &[String]) -> PathBuf {
    match dirs.split_last() {
        None => PathBuf::from("src/middleware/mod.rs"),
        Some((last, outer)) => {
            let parent = outer
                .iter()
                .fold(PathBuf::from("src/middleware"), |dir, name| dir.join(name));
            generator::module_file(&parent, last)
        }
    }
}

fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Upper-case the first letter of each `_`-separated part and keep the
/// rest as written, so `RateLimit` and `HTTPLog` stay as they are and
/// `rate_limit` reads `RateLimit`.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// `content` with `mod <file_name>;` after its last `mod` line and
/// `pub use <file_name>::<struct_name>;` after its last `pub use` line.
fn with_middleware(content: &str, file_name: &str, struct_name: &str) -> Result<String, String> {
    if generator::declares_module(content, file_name) {
        return Err(format!("module '{}' is already declared", file_name));
    }
    let mod_decl = format!("mod {};", file_name);

    let mut lines: Vec<&str> = content.lines().collect();

    let mut last_mod_idx = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("mod ") {
            last_mod_idx = Some(i);
        }
    }

    let mod_insert_idx = match last_mod_idx {
        Some(idx) => idx + 1,
        None => {
            let mut insert_idx = 0;
            for (i, line) in lines.iter().enumerate() {
                if line.starts_with("//!") || line.is_empty() {
                    insert_idx = i + 1;
                } else {
                    break;
                }
            }
            insert_idx
        }
    };
    lines.insert(mod_insert_idx, &mod_decl);

    let pub_use_decl = format!("pub use {}::{};", file_name, struct_name);
    let mut last_pub_use_idx = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("pub use ") {
            last_pub_use_idx = Some(i);
        }
    }

    match last_pub_use_idx {
        Some(idx) => {
            lines.insert(idx + 1, &pub_use_decl);
        }
        None => {
            let mut insert_idx = mod_insert_idx + 1;
            while insert_idx < lines.len() && lines[insert_idx].trim().starts_with("mod ") {
                insert_idx += 1;
            }
            if insert_idx < lines.len() && !lines[insert_idx].is_empty() {
                lines.insert(insert_idx, "");
                insert_idx += 1;
            }
            lines.insert(insert_idx, &pub_use_decl);
        }
    }

    Ok(lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_nested_name_splits_into_directories_file_and_struct() {
        let target = Target::parse("Admin/EnsureRole").unwrap();
        assert_eq!(target.dirs, ["admin"]);
        assert_eq!(target.file, "ensure_role");
        assert_eq!(target.struct_name, "EnsureRoleMiddleware");
        assert_eq!(target.dir(), Path::new("src/middleware/admin"));
        assert_eq!(target.module_path(), "middleware::admin");
    }

    #[test]
    fn a_plain_name_keeps_its_spelling_and_gets_one_suffix() {
        let target = Target::parse("RateLimit").unwrap();
        assert!(target.dirs.is_empty());
        assert_eq!(target.file, "rate_limit");
        assert_eq!(target.struct_name, "RateLimitMiddleware");
        let suffixed = Target::parse("AuthMiddleware").unwrap();
        assert_eq!(suffixed.file, "auth");
        assert_eq!(suffixed.struct_name, "AuthMiddleware");
        assert_eq!(
            Target::parse("rate_limit").unwrap().struct_name,
            "RateLimitMiddleware"
        );
    }

    #[test]
    fn bad_names_are_refused() {
        for bad in [
            "",
            "Admin/",
            "a b",
            "Admin/type/X",
            "Middleware",
            "1st",
            "../X",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_middleware_is_declared_beside_the_others() {
        let source = "//! Application middleware\n\npub mod authenticate;\nmod logging;\n\npub use logging::LoggingMiddleware;\n";
        assert_eq!(
            with_middleware(source, "audit", "AuditMiddleware").unwrap(),
            "//! Application middleware\n\npub mod authenticate;\nmod logging;\nmod audit;\n\npub use logging::LoggingMiddleware;\npub use audit::AuditMiddleware;\n"
        );
        assert!(with_middleware(source, "logging", "LoggingMiddleware").is_err());
    }
}
