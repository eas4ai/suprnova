//! `suprnova live:make <name>` - scaffold a Live component, its view, and
//! its registration without ever overwriting user work.
//!
//! Every target path is validated and checked for conflicts before anything
//! is written, each file is written atomically through
//! [`crate::secure_fs::write_atomic`] (which refuses traversal and symlinks),
//! and a write failure rolls back every file this run created or changed, so
//! a run either leaves the complete set or nothing.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::cargo_meta;
use crate::secure_fs;
use crate::templates;
use crate::ui;

const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];
const MAX_NAME_LEN: usize = 64;

/// The three spellings one component name needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Naming {
    /// `todo_list` - module and file stem.
    pub snake: String,
    /// `TodoList` - struct name.
    pub pascal: String,
    /// `todo-list` - component name segment.
    pub kebab: String,
}

impl Naming {
    /// Accepts `Counter`, `TodoList`, `todo-list`, or `todo_list`; rejects
    /// anything that is not a plain ASCII identifier once normalized.
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_NAME_LEN || !raw.is_ascii() {
            return None;
        }
        if !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        for chunk in raw.split(['-', '_']) {
            if chunk.is_empty() {
                return None;
            }
            let mut current = String::new();
            let bytes: Vec<char> = chunk.chars().collect();
            for (index, ch) in bytes.iter().enumerate() {
                let boundary = index > 0
                    && ch.is_ascii_uppercase()
                    && (bytes[index - 1].is_ascii_lowercase()
                        || bytes[index - 1].is_ascii_digit()
                        || bytes
                            .get(index + 1)
                            .is_some_and(|next| next.is_ascii_lowercase()));
                if boundary && !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
                current.push(ch.to_ascii_lowercase());
            }
            if !current.is_empty() {
                parts.push(current);
            }
        }
        let snake = parts.join("_");
        if !snake
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
            || RUST_KEYWORDS.contains(&snake.as_str())
        {
            return None;
        }
        let pascal = parts
            .iter()
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<String>();
        let kebab = snake.replace('_', "-");
        Some(Self {
            snake,
            pascal,
            kebab,
        })
    }
}

/// Escape control characters before echoing untrusted input back.
fn display_name(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_control() {
                ch.escape_default().to_string()
            } else {
                ch.to_string()
            }
        })
        .take(80)
        .collect()
}

/// Insert `pub mod <snake>;` and `.register::<snake::Pascal>()?` into an
/// existing `src/live/mod.rs` produced by this command.
pub fn register_component(source: &str, snake: &str, pascal: &str) -> Result<String, String> {
    let declaration = format!("pub mod {snake};");
    if source.lines().any(|line| line.trim() == declaration) {
        return Err(format!("{declaration} is already declared"));
    }
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let last_mod = lines.iter().rposition(|line| {
        line.trim_start().starts_with("pub mod ") && line.trim_end().ends_with(';')
    });
    match last_mod {
        Some(index) => lines.insert(index + 1, declaration),
        None => {
            // No module yet: declare it after the `use` block, else right
            // before the registry function.
            let after_uses = lines
                .iter()
                .rposition(|line| line.starts_with("use "))
                .and_then(|start| {
                    lines[start..]
                        .iter()
                        .position(|line| line.trim_end().ends_with(';'))
                        .map(|offset| start + offset + 1)
                });
            let before_registry = lines.iter().position(|line| {
                line.trim_start().starts_with("/// Builds the registry")
                    || line.trim_start().starts_with("pub fn registry()")
            });
            match (after_uses, before_registry) {
                (Some(index), _) => {
                    lines.insert(index, String::new());
                    lines.insert(index + 1, declaration);
                }
                (None, Some(index)) => {
                    lines.insert(index, declaration);
                    lines.insert(index + 1, String::new());
                }
                (None, None) => lines.push(declaration),
            }
        }
    }
    let registry_start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("pub fn registry()"))
        .ok_or_else(|| "no `pub fn registry()` found".to_string())?;
    let build = lines[registry_start..]
        .iter()
        .position(|line| line.contains(".build()"))
        .map(|offset| registry_start + offset)
        .ok_or_else(|| "no `.build()` call found inside `registry()`".to_string())?;
    let line = lines[build].clone();
    let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
    if line.trim_start().starts_with(".build()") {
        // Already chained one call per line: keep the registration at the
        // chain's indentation.
        lines.insert(build, format!("{indent}.register::<{snake}::{pascal}>()?"));
    } else {
        // `LiveRegistry::builder().build()` on one line (rustfmt's shape for an
        // empty registry): split the chain before `.build()`.
        let split = line.find(".build()").unwrap_or(line.len());
        let (head, tail) = line.split_at(split);
        lines[build] = head.trim_end().to_string();
        lines.insert(
            build + 1,
            format!("{indent}    .register::<{snake}::{pascal}>()?"),
        );
        lines.insert(build + 2, format!("{indent}    {tail}"));
    }
    Ok(lines.join("\n") + "\n")
}

/// Insert `pub mod live;` into `src/lib.rs` after its last module declaration.
pub fn declare_live_module(source: &str) -> Option<String> {
    if source.lines().any(|line| line.trim() == "pub mod live;") {
        return None;
    }
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let last_mod = lines.iter().rposition(|line| {
        line.trim_start().starts_with("pub mod ") && line.trim_end().ends_with(';')
    });
    match last_mod {
        Some(index) => lines.insert(index + 1, "pub mod live;".to_string()),
        None => lines.push("pub mod live;".to_string()),
    }
    Some(lines.join("\n") + "\n")
}

enum Plan {
    Create(PathBuf, String),
    /// Path, new content, previous content (restored on rollback).
    Update(PathBuf, String, String),
}

impl Plan {
    fn path(&self) -> &Path {
        match self {
            Self::Create(path, _) | Self::Update(path, _, _) => path,
        }
    }

    fn content(&self) -> &str {
        match self {
            Self::Create(_, content) | Self::Update(_, content, _) => content,
        }
    }
}

/// Undo the writes of one run, newest first: created files are removed and
/// updated files get their previous content back.
fn rollback(written: &[&Plan]) {
    for plan in written.iter().rev() {
        match plan {
            Plan::Create(path, _) => {
                let _ = fs::remove_file(path);
            }
            Plan::Update(path, _, previous) => {
                let _ = secure_fs::write_atomic(path, previous.as_bytes());
            }
        }
    }
}

pub fn run(name: String, dry_run: bool) {
    if let Err(e) = run_inner(&name, dry_run) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(raw: &str, dry_run: bool) -> Result<(), String> {
    let naming = Naming::parse(raw).ok_or_else(|| {
        ui::hint("Use a plain ASCII name such as Counter, TodoList, or todo-list.");
        format!("'{}' is not a valid Live component name", display_name(raw))
    })?;
    if !Path::new("Cargo.toml").exists() || !Path::new("src").is_dir() {
        ui::hint("Make sure you're in a Suprnova project root directory.");
        return Err("No Cargo.toml and src/ directory found in the current directory (project root expected)".to_string());
    }
    let package = cargo_meta::package_name_from_path(Path::new("Cargo.toml"))
        .filter(|name| {
            name.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
        .unwrap_or_else(|| "app".to_string());
    let component_name = format!("{package}.{}", naming.kebab);
    let view = format!("live/{}.html", naming.snake);
    let component_path = PathBuf::from(format!("src/live/{}.rs", naming.snake));
    let view_path = PathBuf::from(format!("templates/{view}"));
    let mod_path = PathBuf::from("src/live/mod.rs");
    let lib_path = PathBuf::from("src/lib.rs");

    for path in [&component_path, &view_path, &mod_path, &lib_path] {
        secure_fs::ensure_contained(Path::new("."), path)?;
    }
    for path in [&component_path, &view_path] {
        if path.exists() {
            ui::warning(&format!(
                "{} already exists; nothing was written",
                path.display()
            ));
            return Ok(());
        }
    }

    let mut plans = vec![
        Plan::Create(
            component_path.clone(),
            templates::live_component(&naming.snake, &naming.pascal, &component_name, &view),
        ),
        Plan::Create(view_path.clone(), templates::live_view().to_string()),
    ];
    let mut manual_registration: Option<String> = None;
    if mod_path.exists() {
        let existing = fs::read_to_string(&mod_path)
            .map_err(|e| format!("Failed to read {}: {e}", mod_path.display()))?;
        if existing
            .lines()
            .any(|line| line.trim() == format!("pub mod {};", naming.snake))
        {
            ui::warning(&format!(
                "{} already declares `pub mod {};`; nothing was written",
                mod_path.display(),
                naming.snake
            ));
            return Ok(());
        }
        match register_component(&existing, &naming.snake, &naming.pascal) {
            Ok(updated) => plans.push(Plan::Update(mod_path.clone(), updated, existing)),
            Err(reason) => manual_registration = Some(reason),
        }
    } else {
        // An older project without a Live module: create the standard module
        // and register through the same insertion path a later run uses.
        let created = register_component(templates::live_mod_rs(), &naming.snake, &naming.pascal)?;
        plans.push(Plan::Create(mod_path.clone(), created));
    }
    let mut lib_plan: Option<Plan> = None;
    let mut lib_missing = false;
    if lib_path.exists() {
        let existing = fs::read_to_string(&lib_path)
            .map_err(|e| format!("Failed to read {}: {e}", lib_path.display()))?;
        if let Some(updated) = declare_live_module(&existing) {
            lib_plan = Some(Plan::Update(lib_path.clone(), updated, existing));
        }
    } else {
        lib_missing = true;
    }

    if dry_run {
        ui::info("Dry run: nothing will be written");
        for plan in plans.iter().chain(lib_plan.iter()) {
            match plan {
                Plan::Create(path, _) => ui::hint(&format!("Would create {}", path.display())),
                Plan::Update(path, _, _) => ui::hint(&format!("Would update {}", path.display())),
            }
        }
        if let Some(reason) = &manual_registration {
            ui::warning(&format!(
                "Would not update {}: {reason}; registration would need a manual edit",
                mod_path.display()
            ));
        }
        return Ok(());
    }

    for dir in ["src/live", "templates/live"] {
        fs::create_dir_all(dir).map_err(|e| format!("Failed to create {dir}: {e}"))?;
    }
    let mut written: Vec<&Plan> = Vec::with_capacity(plans.len());
    for plan in &plans {
        if let Err(e) = secure_fs::write_atomic(plan.path(), plan.content().as_bytes()) {
            rollback(&written);
            return Err(format!(
                "{e}; rolled back the {} file(s) this run had written, nothing was changed",
                written.len()
            ));
        }
        written.push(plan);
    }
    for plan in &plans {
        let verb = match plan {
            Plan::Create(..) => "Created",
            Plan::Update(..) => "Updated",
        };
        ui::success(&format!("{verb} {}", plan.path().display()));
    }
    if let Some(reason) = manual_registration {
        ui::warning(&format!(
            "Could not update {}: {reason}",
            mod_path.display()
        ));
        ui::hint(&format!(
            "Add `pub mod {};` and `.register::<{}::{}>()?` to the registry builder by hand.",
            naming.snake, naming.snake, naming.pascal
        ));
    }
    match lib_plan {
        Some(Plan::Update(path, content, _)) => {
            match secure_fs::write_atomic(&path, content.as_bytes()) {
                Ok(()) => ui::success(&format!("Updated {}", path.display())),
                Err(e) => {
                    ui::warning(&format!("Could not update {}: {e}", path.display()));
                    ui::hint("Add `pub mod live;` to src/lib.rs by hand.");
                }
            }
        }
        Some(Plan::Create(..)) => {}
        None if lib_missing => {
            ui::warning("No src/lib.rs found; declare `pub mod live;` in your crate root by hand.");
        }
        None => {}
    }

    ui::br();
    ui::info(&format!(
        "Component {} renders {}",
        console::style(&component_name).cyan().bold(),
        console::style(&view).cyan()
    ));
    ui::hint("Bind the registry during bootstrap if you have not already:");
    ui::command("suprnova::App::singleton(crate::live::registry().expect(\"Live registry\"));");
    ui::hint("Then check every view with:");
    ui::command("suprnova live:check");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_normalize_to_snake_pascal_and_kebab() {
        let counter = Naming::parse("Counter").unwrap();
        assert_eq!(
            (
                counter.snake.as_str(),
                counter.pascal.as_str(),
                counter.kebab.as_str()
            ),
            ("counter", "Counter", "counter")
        );
        let todo = Naming::parse("todo-list").unwrap();
        assert_eq!(
            (
                todo.snake.as_str(),
                todo.pascal.as_str(),
                todo.kebab.as_str()
            ),
            ("todo_list", "TodoList", "todo-list")
        );
        assert_eq!(Naming::parse("TodoList").unwrap().snake, "todo_list");
        assert_eq!(Naming::parse("todo_list").unwrap().pascal, "TodoList");
        assert_eq!(Naming::parse("HTTPServer").unwrap().snake, "http_server");
        assert_eq!(Naming::parse("user2Card").unwrap().snake, "user2_card");
    }

    #[test]
    fn invalid_names_are_rejected() {
        for name in [
            "",
            "9lives",
            "my component",
            "mod",
            "self",
            "a/b",
            "../evil",
            "counter.rs",
            "-x",
            "a--b",
            "Ünïcode",
        ] {
            assert!(Naming::parse(name).is_none(), "{name:?} must be rejected");
        }
        assert!(Naming::parse(&"a".repeat(MAX_NAME_LEN + 1)).is_none());
    }

    #[test]
    fn registration_is_inserted_before_build_and_after_the_last_module() {
        let source = register_component(templates::live_mod_rs(), "counter", "Counter").unwrap();
        assert!(source.contains("pub mod counter;"), "{source}");
        let updated = register_component(&source, "todo_list", "TodoList").unwrap();
        let counter_mod = updated.find("pub mod counter;").unwrap();
        let todo_mod = updated.find("pub mod todo_list;").unwrap();
        assert!(counter_mod < todo_mod);
        let counter_reg = updated.find(".register::<counter::Counter>()?").unwrap();
        let todo_reg = updated.find(".register::<todo_list::TodoList>()?").unwrap();
        let build = updated.find(".build();").unwrap();
        assert!(counter_reg < todo_reg && todo_reg < build);
        assert!(register_component(&updated, "todo_list", "TodoList").is_err());
        assert!(
            register_component("pub mod x;\n", "y", "Y").is_err(),
            "missing registry() anchor"
        );
    }

    #[test]
    fn registration_splits_a_single_line_builder_chain() {
        let formatted = "pub fn registry() -> Result<LiveRegistry, RegistryError> {\n    let registry = LiveRegistry::builder().build();\n    Ok(registry)\n}\n";
        let updated = register_component(formatted, "counter", "Counter").unwrap();
        assert!(
            updated.contains("let registry = LiveRegistry::builder()\n        .register::<counter::Counter>()?\n        .build();"),
            "{updated}"
        );
        assert!(updated.starts_with("pub mod counter;\n"), "{updated}");
        let again = register_component(&updated, "feed", "Feed").unwrap();
        assert_eq!(again.matches(".build()").count(), 1);
        assert!(
            again.contains(".register::<counter::Counter>()?\n        .register::<feed::Feed>()?\n        .build();"),
            "{again}"
        );
    }

    #[test]
    fn the_live_module_is_declared_once() {
        let lib = "pub mod bootstrap;\npub mod routes;\n";
        let updated = declare_live_module(lib).unwrap();
        assert_eq!(
            updated,
            "pub mod bootstrap;\npub mod routes;\npub mod live;\n"
        );
        assert!(declare_live_module(&updated).is_none());
        assert_eq!(
            declare_live_module("// empty\n").unwrap(),
            "// empty\npub mod live;\n"
        );
    }
}
