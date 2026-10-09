//! The application-wide settings `#[suprnova::model]` and
//! `#[suprnova::main]` read from a package's own `Cargo.toml`:
//!
//! ```toml
//! [package.metadata.suprnova.model]
//! datetime_cast = "native"   # or "naive"
//!
//! [package.metadata.suprnova.schema]
//! unsigned_ids = true
//! ```
//!
//! An application ported from Laravel's MySQL schema wants every model's
//! date-time fields cast to the native columns that schema has, and every
//! migration's `id()` and `foreign_id()` unsigned, without repeating that
//! on each model and each migration. The manifest is the one file every
//! build of the crate has, at `CARGO_MANIFEST_DIR`.
//!
//! `#[model]` reads the model table from the package that declares the
//! model. `#[suprnova::main]` reads the schema table from the binary's
//! package, because migrations build their schema at run time, where no
//! macro runs; `main` installs it before anything else. A key or value the
//! framework does not know fails the build of the macro that reads it, so a
//! typo never silently keeps the default.

use std::path::{Path, PathBuf};

/// The table `#[suprnova::model]` reads, as error messages name it.
pub(crate) const MODEL_TABLE: &str = "[package.metadata.suprnova.model]";
/// The table `#[suprnova::main]` reads, as error messages name it.
pub(crate) const SCHEMA_TABLE: &str = "[package.metadata.suprnova.schema]";

/// The cast a `DateTime<Utc>` field gets when it names none of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DateTimeCast {
    /// No setting: `AsDateTime`, RFC 3339 text.
    #[default]
    Text,
    /// `datetime_cast = "native"`: `AsNativeDateTime`, a column that keeps
    /// the zone.
    Native,
    /// `datetime_cast = "naive"`: `AsNaiveDateTime`, a column without one.
    Naive,
}

/// What `[package.metadata.suprnova.model]` sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelSettings {
    pub(crate) datetime_cast: DateTimeCast,
}

/// What `[package.metadata.suprnova.schema]` sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SchemaSettings {
    pub(crate) unsigned_ids: bool,
}

/// A package's settings and the manifest they came from. The expansion
/// names the manifest so cargo re-runs the macro when it changes: cargo
/// does not fingerprint `[package.metadata]`, and a file a macro opens on
/// its own never reaches the crate's dep-info.
pub(crate) struct Read<T> {
    pub(crate) settings: T,
    pub(crate) manifest: Option<PathBuf>,
}

/// Reads the model table of the package at `crate_dir`.
pub(crate) fn model_settings(crate_dir: &Path) -> Result<Read<ModelSettings>, String> {
    read(crate_dir, MODEL_TABLE, model_settings_from_manifest)
}

/// Reads the schema table of the package at `crate_dir`.
pub(crate) fn schema_settings(crate_dir: &Path) -> Result<Read<SchemaSettings>, String> {
    read(crate_dir, SCHEMA_TABLE, schema_settings_from_manifest)
}

/// The directory of the crate being compiled. Unset in some IDE states, in
/// which the defaults apply.
pub(crate) fn crate_dir() -> Option<PathBuf> {
    std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from)
}

fn read<T: Default>(
    crate_dir: &Path,
    table: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<Read<T>, String> {
    let path = crate_dir.join("Cargo.toml");
    let manifest = match std::fs::read_to_string(&path) {
        Ok(manifest) => manifest,
        // Some build systems set `CARGO_MANIFEST_DIR` without a manifest.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Read {
                settings: T::default(),
                manifest: None,
            });
        }
        Err(error) => {
            return Err(format!(
                "{table}: could not read {}: {error}",
                path.display()
            ));
        }
    };
    let settings =
        parse(&manifest).map_err(|problem| format!("{table} in Cargo.toml: {problem}"))?;
    Ok(Read {
        settings,
        manifest: Some(path),
    })
}

/// The `[package.metadata.suprnova.<name>]` table of a manifest, `None`
/// when any part of the path is absent.
fn suprnova_table(manifest: &str, name: &str) -> Result<Option<toml::Table>, String> {
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

/// A setting's value as an error quotes it: `` `true` ``, `` `"yes"` ``,
/// or the kind of value for a table or an array.
fn describe(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => format!("`{text:?}`"),
        toml::Value::Integer(number) => format!("`{number}`"),
        toml::Value::Float(number) => format!("`{number}`"),
        toml::Value::Boolean(flag) => format!("`{flag}`"),
        toml::Value::Datetime(moment) => format!("`{moment}`"),
        toml::Value::Array(_) => "an array".to_string(),
        toml::Value::Table(_) => "a table".to_string(),
    }
}

pub(crate) fn model_settings_from_manifest(manifest: &str) -> Result<ModelSettings, String> {
    let Some(table) = suprnova_table(manifest, "model")? else {
        return Ok(ModelSettings::default());
    };
    if let Some(key) = table.keys().find(|key| *key != "datetime_cast") {
        return Err(format!("unknown key `{key}`; the key is `datetime_cast`"));
    }
    let datetime_cast = match table.get("datetime_cast") {
        None => DateTimeCast::Text,
        Some(toml::Value::String(value)) => match value.as_str() {
            "native" => DateTimeCast::Native,
            "naive" => DateTimeCast::Naive,
            other => {
                return Err(format!(
                    "unknown `datetime_cast` value `{other}`; it is \"native\" or \"naive\""
                ));
            }
        },
        Some(other) => {
            return Err(format!(
                "`datetime_cast` must be the string \"native\" or \"naive\", got {}",
                describe(other)
            ));
        }
    };
    Ok(ModelSettings { datetime_cast })
}

pub(crate) fn schema_settings_from_manifest(manifest: &str) -> Result<SchemaSettings, String> {
    let Some(table) = suprnova_table(manifest, "schema")? else {
        return Ok(SchemaSettings::default());
    };
    if let Some(key) = table.keys().find(|key| *key != "unsigned_ids") {
        return Err(format!("unknown key `{key}`; the key is `unsigned_ids`"));
    }
    let unsigned_ids = match table.get("unsigned_ids") {
        None => false,
        Some(toml::Value::Boolean(value)) => *value,
        Some(other) => {
            return Err(format!(
                "`unsigned_ids` must be `true` or `false`, got {}",
                describe(other)
            ));
        }
    };
    Ok(SchemaSettings { unsigned_ids })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(tables: &str) -> String {
        format!("[package]\nname = \"app\"\n\n{tables}\n")
    }

    #[test]
    fn without_the_tables_the_defaults_apply() {
        for body in [
            manifest(""),
            manifest("[package.metadata.docs.rs]\nall-features = true"),
            manifest("[package.metadata.suprnova.inertia]\npages_dir = \"pages\""),
        ] {
            assert_eq!(
                model_settings_from_manifest(&body),
                Ok(ModelSettings::default())
            );
            assert_eq!(
                schema_settings_from_manifest(&body),
                Ok(SchemaSettings::default())
            );
        }
    }

    #[test]
    fn the_known_values_are_read() {
        let body = manifest(
            "[package.metadata.suprnova.model]\ndatetime_cast = \"native\"\n\n\
             [package.metadata.suprnova.schema]\nunsigned_ids = true",
        );
        assert_eq!(
            model_settings_from_manifest(&body).map(|s| s.datetime_cast),
            Ok(DateTimeCast::Native)
        );
        assert_eq!(
            schema_settings_from_manifest(&body).map(|s| s.unsigned_ids),
            Ok(true)
        );
        let naive = manifest("[package.metadata.suprnova.model]\ndatetime_cast = \"naive\"");
        assert_eq!(
            model_settings_from_manifest(&naive).map(|s| s.datetime_cast),
            Ok(DateTimeCast::Naive)
        );
    }

    #[test]
    fn an_unknown_key_or_value_is_named() {
        let cases = [
            (
                model_settings_from_manifest(&manifest(
                    "[package.metadata.suprnova.model]\ndatetime_casts = \"native\"",
                ))
                .err(),
                "`datetime_casts`",
            ),
            (
                model_settings_from_manifest(&manifest(
                    "[package.metadata.suprnova.model]\ndatetime_cast = \"zoned\"",
                ))
                .err(),
                "`zoned`",
            ),
            (
                model_settings_from_manifest(&manifest(
                    "[package.metadata.suprnova.model]\ndatetime_cast = true",
                ))
                .err(),
                "`true`",
            ),
            (
                schema_settings_from_manifest(&manifest(
                    "[package.metadata.suprnova.schema]\nunsigned_id = true",
                ))
                .err(),
                "`unsigned_id`",
            ),
            (
                schema_settings_from_manifest(&manifest(
                    "[package.metadata.suprnova.schema]\nunsigned_ids = \"yes\"",
                ))
                .err(),
                "\"yes\"",
            ),
            (
                schema_settings_from_manifest(&manifest("[package.metadata.suprnova]\nschema = 1"))
                    .err(),
                "`package.metadata.suprnova.schema` must be a table",
            ),
        ];
        for (error, fragment) in cases {
            let error = error.expect("the setting is refused");
            assert!(error.contains(fragment), "`{fragment}` in: {error}");
        }
    }
}
