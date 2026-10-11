//! `suprnova make:inertia <name> [--data] [--force] [--test]` - scaffold an
//! Inertia page, or with `--data` a Data struct.
//!
//! A nested page name (`Admin/Users`) writes under the same directory of
//! `frontend/src/pages`, and the component is named with it
//! (`Admin/UsersPage`). An existing page or test is kept unless `--force`
//! is given; the page and its test are written together or not at all.

use console::style;
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::generator::{self, WriteSet};
use crate::commands::live_make::Naming;
use crate::templates::{self, Frontend};
use crate::ui;

const DATA_TEMPLATE: &str = r#"//! {name} - unified inbound + outbound DTO.

use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct {name} {{
    pub id: i64,
    // Add fields here.
    //
    // Available field attributes:
    //   #[data(input_only)]     - accepted on Deserialize, omitted from Serialize
    //   #[data(output_only)]    - rejected on Deserialize, included in Serialize
    //   #[data(allow_include)]  - registers as ?include=-eligible (default-deny)
    //
    // For PATCH endpoints, use suprnova::data::Field<T> to distinguish
    // absent from null. For lazy outbound fields, use suprnova::inertia::Prop<T>.
}}
"#;

pub fn run(name: String, data: bool, force: bool, test: bool) {
    if data {
        run_data_struct(name, force);
    } else if let Err(e) = run_inertia_page(&name, force, test) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_data_struct(name: String, force: bool) {
    let struct_name = to_pascal_case(&name);

    if !is_valid_rust_identifier(&struct_name) {
        ui::error(&format!("'{}' is not a valid struct name", name));
        std::process::exit(1);
    }

    let file_name = to_snake_case(&struct_name);
    let props_dir = Path::new("src/props");
    let props_file = props_dir.join(format!("{}.rs", file_name));

    // Create the props directory if it doesn't exist. Warn on
    // first-time creation so the user remembers to add the module
    // declaration to `src/lib.rs` - the file is otherwise orphaned
    // and the new Data struct is invisible to the rest of the crate.
    let first_time = !props_dir.exists();
    if first_time && let Err(e) = fs::create_dir_all(props_dir) {
        ui::error(&format!("Failed to create directory src/props: {}", e));
        std::process::exit(1);
    }
    if first_time {
        ui::warning("Make sure to add `pub mod props;` to your src/lib.rs");
    }

    if props_file.exists() && !force {
        ui::warning(&format!(
            "Props struct '{}' already exists at {} (pass --force to overwrite)",
            struct_name,
            props_file.display()
        ));
        std::process::exit(0);
    }

    let content = DATA_TEMPLATE.replace("{name}", &struct_name);

    if let Err(e) = crate::secure_fs::write_generated(&props_file, &content) {
        ui::error(&format!("Failed to write props file: {}", e));
        std::process::exit(1);
    }
    ui::success(&format!("Created {}", props_file.display()));

    ui::br();
    ui::info(&format!(
        "Data struct {} created at {}",
        style(&struct_name).cyan().bold(),
        style(props_file.display().to_string().as_str()).dim(),
    ));
    ui::br();
    ui::hint("Use in a controller with automatic serde + validation:");
    ui::command(&format!(
        "let dto: {} = req.validate_json().await?;",
        struct_name
    ));
    ui::br();
}

fn run_inertia_page(name: &str, force: bool, test: bool) -> Result<(), String> {
    let _ = dotenvy::from_path(".env");

    let frontend = Frontend::detect_from_env();
    let ext = frontend.page_ext();
    let mut segments = generator::segments(name, false)?;
    let last = segments.pop().unwrap_or_default();
    let page_name = to_page_name(&last);
    if !is_valid_component_name(&page_name) {
        return Err(format!("'{}' is not a valid page name", name));
    }
    if let Some(bad) = segments.iter().find(|segment| !is_valid_directory(segment)) {
        return Err(format!(
            "'{bad}' is not a valid page directory (in '{name}'): use letters, digits, `_` \
             and `-`, starting with a letter"
        ));
    }
    let component = segments
        .iter()
        .map(String::as_str)
        .chain([page_name.as_str()])
        .collect::<Vec<_>>()
        .join("/");

    let pages_dir = Path::new("frontend/src/pages");
    if !pages_dir.exists() {
        ui::hint("Make sure you're in a Suprnova project root directory.");
        return Err("Pages directory not found at frontend/src/pages".to_string());
    }
    let page_file = pages_dir.join(format!("{component}.{ext}"));
    let stem = component
        .split('/')
        .map(snake_segment)
        .collect::<Vec<_>>()
        .join("_");
    let test_file = PathBuf::from(format!("tests/{stem}.rs"));
    if test && !Path::new("Cargo.toml").is_file() {
        // Checked before anything is written: the test belongs to the
        // package in the current directory.
        return Err(
            "--test needs a Cargo.toml in the current directory (run it from the project root)"
                .to_string(),
        );
    }

    let mut owned = vec![&page_file];
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
        // `live:make` and this command always have.
        ui::warning(&format!(
            "{} already exists; nothing was written (pass --force to overwrite)",
            existing.join(" and ")
        ));
        return Ok(());
    }

    let mut files = WriteSet::default();
    files.put(
        page_file,
        templates::inertia_page_template(&component, frontend),
    )?;
    if test {
        files.put(
            test_file,
            templates::inertia_page_test_template(&component, &format!("{stem}_renders")),
        )?;
    }
    generator::report(&files.apply()?);

    ui::br();
    ui::info(&format!(
        "Page {} ({}) created",
        style(&component).cyan().bold(),
        style(frontend.as_str()).dim(),
    ));
    ui::br();
    ui::hint("Use the page in a controller:");
    ui::command(&format!(
        "inertia_response!(&req, \"{}\", props)",
        component
    ));
    ui::br();
    Ok(())
}

/// A page directory segment: ASCII letters, digits, `_` and `-`, starting
/// with a letter, so it is a path segment on every platform and in an
/// Inertia component name.
fn is_valid_directory(segment: &str) -> bool {
    segment
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A component path segment as part of a test file stem: `UsersPage`
/// reads `users_page`, `user-admin` reads `user_admin`.
fn snake_segment(segment: &str) -> String {
    match Naming::parse(segment) {
        Some(naming) => naming.snake,
        // A Rust keyword is still a file stem.
        None => to_snake_case(&segment.replace('-', "_")),
    }
}

fn is_valid_component_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric())
}

fn is_valid_rust_identifier(name: &str) -> bool {
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

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_uppercase().next().unwrap());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

fn to_page_name(input: &str) -> String {
    let pascal = to_pascal_case(input);
    if pascal.ends_with("Page") {
        pascal
    } else {
        format!("{}Page", pascal)
    }
}
