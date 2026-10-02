use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use magnetar::{Error, Result};

const REQUIRED_FEATURES: [&str; 19] = [
    "password",
    "email-verification",
    "password-management",
    "session-management",
    "magic-link",
    "passkey",
    "two-factor",
    "oauth",
    "oauth-apple",
    "oauth-google",
    "oauth-facebook",
    "oauth-x",
    "oauth-tiktok",
    "device-authorization",
    "seaorm-sqlite",
    "seaorm-postgres",
    "seaorm-mysql",
    "redis",
    "migration",
];

const FORBIDDEN_DEPENDENCY_FAMILIES: [&str; 3] = ["suprnova", "torii", "oauth2-broker"];
const ROOT_PACKAGE_NAME: &str = "suprnova-magnetar";

fn manifest() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    fs::read_to_string(path).expect("the crate manifest must be readable")
}

fn workspace_manifest() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    fs::read_to_string(path).expect("the workspace manifest must be readable")
}

fn key_name(line: &str) -> Option<&str> {
    let line = line
        .split_once('#')
        .map_or(line, |(content, _)| content)
        .trim();
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.trim_matches(['"', '\'']))
}

fn dependency_table(header: &str) -> bool {
    let header = header.trim();
    let inner = header
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'));
    let Some(inner) = inner else {
        return false;
    };

    matches!(
        inner,
        "dependencies" | "dev-dependencies" | "build-dependencies"
    ) || (inner.starts_with("target.")
        && ["dependencies", "dev-dependencies", "build-dependencies"]
            .iter()
            .any(|suffix| inner.ends_with(&format!(".{suffix}"))))
}

fn forbidden_dependency(name: &str) -> bool {
    if name == ROOT_PACKAGE_NAME {
        return false;
    }

    FORBIDDEN_DEPENDENCY_FAMILIES.iter().any(|family| {
        name == *family
            || name
                .strip_prefix(family)
                .is_some_and(|suffix| suffix.starts_with('-'))
    })
}

fn dependency_name_from_header(header: &str) -> Option<&str> {
    let inner = header
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))?;
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(name) = inner.strip_prefix(&format!("{table}.")) {
            return Some(
                name.split('.')
                    .next()
                    .unwrap_or(name)
                    .trim_matches(['"', '\'']),
            );
        }
        if let Some((_, name)) = inner.split_once(&format!(".{table}.")) {
            return Some(
                name.split('.')
                    .next()
                    .unwrap_or(name)
                    .trim_matches(['"', '\'']),
            );
        }
    }
    None
}

fn quoted_value(line: &str) -> Option<&str> {
    let (_, value) = line.split_once('=')?;
    quoted_string(value)
}

fn quoted_string(value: &str) -> Option<&str> {
    let value = value
        .split_once('#')
        .map_or(value, |(content, _)| content)
        .trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
}

fn inline_dependency_package(line: &str) -> Option<&str> {
    let (_, table) = line.split_once('=')?;
    let table = table.trim().strip_prefix('{')?;
    for field in table.split(',') {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        if key.trim() != "package" {
            continue;
        }
        let value = value.trim().trim_end_matches('}').trim();
        return quoted_string(value);
    }
    None
}

fn assert_allowed_dependency(name: &str) {
    assert!(
        !forbidden_dependency(name),
        "forbidden provider dependency declared: {name}"
    );
}

#[test]
fn package_and_library_names_are_stable() {
    let manifest = manifest();

    assert_eq!(env!("CARGO_PKG_NAME"), ROOT_PACKAGE_NAME);
    assert!(manifest.contains("name = \"suprnova-magnetar\""));
    assert!(manifest.contains("version.workspace = true"));
    assert!(manifest.contains("edition.workspace = true"));
    assert!(manifest.contains("rust-version.workspace = true"));
    assert!(manifest.contains("license.workspace = true"));
    assert!(workspace_manifest().contains("resolver = \"3\""));
    assert!(manifest.contains("name = \"magnetar\""));
    assert!(manifest.contains("path = \"src/lib.rs\""));

    let mut in_features = false;
    let declared_features = manifest
        .lines()
        .filter_map(|line| {
            if line.trim_start().starts_with('[') {
                in_features = line.trim() == "[features]";
                return None;
            }
            if !in_features {
                return None;
            }
            key_name(line).filter(|name| *name != "default")
        })
        .collect::<BTreeSet<_>>();
    let required_features = REQUIRED_FEATURES.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(
        declared_features, required_features,
        "Cargo.toml must declare exactly the required named features"
    );
}

#[test]
fn all_dependency_tables_exclude_provider_families_but_allow_the_root_package() {
    let manifest = manifest();
    assert!(!forbidden_dependency(ROOT_PACKAGE_NAME));

    let mut in_dependency_table = false;
    let mut nested_dependency_table = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            nested_dependency_table = dependency_name_from_header(trimmed).is_some();
            if let Some(name) = dependency_name_from_header(trimmed) {
                assert_allowed_dependency(name);
            }
            in_dependency_table = nested_dependency_table || dependency_table(trimmed);
            continue;
        }
        if !in_dependency_table {
            continue;
        }

        if let Some(name) = key_name(line) {
            assert_allowed_dependency(name);
            if nested_dependency_table
                && name == "package"
                && let Some(package_name) = quoted_value(line)
            {
                assert_allowed_dependency(package_name);
            }
        }
        if let Some(package_name) = inline_dependency_package(line) {
            assert_allowed_dependency(package_name);
        }
    }
}

#[test]
fn inline_dependency_tables_reject_provider_families() {
    let package_name =
        inline_dependency_package(r#"alias = { package = "torii-core", version = "1" }"#)
            .expect("inline dependency package field must be detected");
    assert_eq!(package_name, "torii-core");
    assert!(forbidden_dependency(package_name));
    assert!(!forbidden_dependency("alias"));
}

#[test]
fn foundation_exports_are_available() {
    let result: Result<()> = Ok(());
    assert!(result.is_ok());

    let error = Error::Internal {
        message: "database unavailable".to_owned(),
    };
    assert_eq!(error.to_string(), "internal error: database unavailable");
    let standard_error: &dyn std::error::Error = &error;
    assert_eq!(
        standard_error.to_string(),
        "internal error: database unavailable"
    );
}
