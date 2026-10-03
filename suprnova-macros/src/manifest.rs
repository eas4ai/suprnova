//! The `[package.metadata.suprnova.*]` tables of the application crate's
//! `Cargo.toml`, read at compile time by the macros they configure.
//!
//! Cargo does not fingerprint `[package.metadata]`, so a macro that reads a
//! table names the manifest in an `include_bytes!` ([`track`]): editing the
//! table then re-runs the macro.

use std::path::{Path, PathBuf};

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::LitStr;

/// Returns `[package.metadata.suprnova.<name>]` from a `Cargo.toml` body:
/// `Ok(None)` when it is absent, an error when it or a parent is not a table.
pub(crate) fn table(manifest: &str, name: &str) -> Result<Option<toml::Table>, String> {
    let document: toml::Table = manifest
        .parse()
        .map_err(|error| format!("could not parse the manifest: {error}"))?;
    let Some(metadata) = document
        .get("package")
        .and_then(|package| package.get("metadata"))
    else {
        return Ok(None);
    };
    let Some(suprnova) = metadata.get("suprnova") else {
        return Ok(None);
    };
    let Some(suprnova) = suprnova.as_table() else {
        return Err("`package.metadata.suprnova` must be a table".to_string());
    };
    match suprnova.get(name) {
        None => Ok(None),
        Some(toml::Value::Table(table)) => Ok(Some(table.clone())),
        Some(_) => Err(format!(
            "`package.metadata.suprnova.{name}` must be a table"
        )),
    }
}

/// Reads `[package.metadata.suprnova.<name>]` from the crate being compiled.
///
/// Returns the manifest path (for [`track`]) with the table. Without
/// `CARGO_MANIFEST_DIR` or a manifest (some IDEs and build systems) there is
/// nothing to read and the macro keeps its defaults. Errors start with the
/// table's name, ready for a compile error.
pub(crate) fn read(name: &str) -> Result<(Option<PathBuf>, Option<toml::Table>), String> {
    let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") else {
        return Ok((None, None));
    };
    let path = Path::new(&dir).join("Cargo.toml");
    let manifest = match std::fs::read_to_string(&path) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(error) => {
            return Err(format!(
                "[package.metadata.suprnova.{name}]: could not read {}: {error}",
                path.display()
            ));
        }
    };
    let table = table(&manifest, name).map_err(|problem| {
        format!("[package.metadata.suprnova.{name}] in Cargo.toml: {problem}")
    })?;
    Ok((Some(path), table))
}

/// An unnamed `include_bytes!` of `manifest`, which puts it in the crate's
/// dep-info so a change to the table re-runs the macro. Nothing reaches the
/// binary.
pub(crate) fn track(manifest: Option<&Path>) -> TokenStream {
    match manifest.and_then(Path::to_str) {
        Some(path) => {
            let path = LitStr::new(path, Span::call_site());
            quote! { const _: &[u8] = ::core::include_bytes!(#path); }
        }
        None => quote! {},
    }
}

/// Fails on a key `allowed` does not list, naming the keys that exist.
pub(crate) fn reject_unknown_keys(table: &toml::Table, allowed: &[&str]) -> Result<(), String> {
    match table.keys().find(|key| !allowed.contains(&key.as_str())) {
        None => Ok(()),
        Some(key) => {
            let keys = allowed
                .iter()
                .map(|key| format!("`{key}`"))
                .collect::<Vec<_>>()
                .join(" and ");
            Err(format!("unknown key `{key}`; the keys are {keys}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_table_is_none() {
        assert_eq!(table("[package]\nname = \"app\"\n", "model"), Ok(None));
        assert_eq!(
            table(
                "[package.metadata.suprnova.inertia]\npages_dir = \"x\"\n",
                "model"
            ),
            Ok(None)
        );
    }

    #[test]
    fn the_named_table_is_returned() {
        let found = table(
            "[package.metadata.suprnova.model]\ndatetime_cast = \"native\"\n",
            "model",
        )
        .unwrap()
        .unwrap();
        assert_eq!(found["datetime_cast"].as_str(), Some("native"));
    }

    #[test]
    fn a_non_table_is_an_error() {
        assert_eq!(
            table("[package.metadata.suprnova]\nmodel = 1\n", "model"),
            Err("`package.metadata.suprnova.model` must be a table".to_string())
        );
        assert_eq!(
            table("[package.metadata]\nsuprnova = 1\n", "model"),
            Err("`package.metadata.suprnova` must be a table".to_string())
        );
    }

    #[test]
    fn unknown_keys_are_named() {
        let table: toml::Table = "unsigned_id = true".parse().unwrap();
        assert_eq!(
            reject_unknown_keys(&table, &["unsigned_ids"]),
            Err("unknown key `unsigned_id`; the keys are `unsigned_ids`".to_string())
        );
    }
}
