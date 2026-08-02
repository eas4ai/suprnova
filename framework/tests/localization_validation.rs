//! Keyed validation messages, translated at the serialization boundary.
//!
//! Rules emit a [`ValidationMessage`] (key + args + English fallback);
//! `ValidationErrors::to_json` is the single place that consults the
//! catalog. These tests drive that seam end to end: a built-in rule's
//! key, a per-locale translation, the untranslated keyless path, the
//! `field-<name>` attribute-name lookup, the top-level banner, and the
//! `#[derive(Validate)]` code → key mapping.

#![cfg(feature = "localization")]

use std::fs;
use std::sync::Arc;
use suprnova::validation::rule::{Rule, rules};
use suprnova::{FluentTranslator, Locale, LocalizationConfig, Translator, ValidationErrors};
use suprnova::{Lang, scope_locale};

fn config() -> LocalizationConfig {
    LocalizationConfig {
        default_locale: Locale::parse("en").unwrap(),
        fallback_locale: Locale::parse("en").unwrap(),
        use_isolating: false,
        detection: vec![],
        session_key: "locale".into(),
        cookie_name: "locale".into(),
    }
}

fn write_lang(dir: &std::path::Path, locale: &str, file: &str, ftl: &str) {
    let d = dir.join(locale);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(file), ftl).unwrap();
}

fn bind_translator(dir: &std::path::Path) {
    let t = FluentTranslator::from_dir(dir, &config()).unwrap();
    suprnova::container::App::bind::<dyn Translator>(Arc::new(t));
}

#[tokio::test]
#[serial_test::serial]
async fn builtin_rule_message_translates_per_locale() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "es",
        "validation.ftl",
        "validation-min = El campo { $field } debe tener al menos { $min } caracteres.\n",
    );
    bind_translator(tmp.path());

    let msg = rules::Min(8).passes("abc").unwrap_err();
    assert_eq!(msg.key, "validation-min");
    // The fallback is the pre-localization English text, unchanged.
    assert_eq!(msg.to_string(), "must be at least 8 characters");

    let mut errs = ValidationErrors::new();
    errs.add("password", msg);

    scope_locale(Locale::parse("es").unwrap(), async move {
        let json = errs.to_json();
        let text = json["errors"]["password"][0].as_str().unwrap();
        assert_eq!(text, "El campo password debe tener al menos 8 caracteres.");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn keyless_custom_message_passes_through_untranslated() {
    let tmp = tempfile::tempdir().unwrap();
    bind_translator(tmp.path());

    let mut errs = ValidationErrors::new();
    errs.add("name", "custom check failed"); // &str → keyless ValidationMessage

    scope_locale(Locale::parse("es").unwrap(), async move {
        let json = errs.to_json();
        assert_eq!(json["errors"]["name"][0], "custom check failed");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn field_name_message_is_used_when_defined() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "validation.ftl",
        "field-email = email address\n",
    );
    bind_translator(tmp.path());

    let mut errs = ValidationErrors::new();
    errs.add("email", rules::Required.passes("").unwrap_err());

    scope_locale(Locale::parse("en").unwrap(), async move {
        let json = errs.to_json();
        let text = json["errors"]["email"][0].as_str().unwrap();
        assert_eq!(text, "The email address field is required.");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn invalid_data_banner_translates() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "es",
        "validation.ftl",
        "validation-invalid-data = Datos no válidos.\n",
    );
    bind_translator(tmp.path());

    let mut errs = ValidationErrors::new();
    errs.add("x", rules::Required.passes("").unwrap_err());

    scope_locale(Locale::parse("es").unwrap(), async move {
        assert_eq!(errs.to_json()["message"], "Datos no válidos.");
    })
    .await;
}

#[test]
fn derive_flow_codes_map_to_keys() {
    // validator crate error with code "email", no message → keyed.
    let mut ve = validator::ValidationErrors::new();
    ve.add("mail", validator::ValidationError::new("email"));
    let ours = ValidationErrors::from_validator(ve);
    let m = &ours.errors["mail"][0];
    assert_eq!(m.key, "validation-email");
    // Multi-word codes kebab-case into the catalog id.
    let mut ve = validator::ValidationErrors::new();
    ve.add("site", validator::ValidationError::new("required_with"));
    let ours = ValidationErrors::from_validator(ve);
    assert_eq!(ours.errors["site"][0].key, "validation-required-with");
}

#[tokio::test]
#[serial_test::serial]
async fn every_builtin_key_resolves_in_the_embedded_catalog() {
    // The embedded English catalog is the contract: every key a built-in
    // rule can emit must be defined, or the message renders as the raw
    // key for an app that ships no catalog of its own.
    let tmp = tempfile::tempdir().unwrap();
    bind_translator(tmp.path());

    let keys = [
        "validation-invalid-data",
        "validation-required",
        "validation-email",
        "validation-min",
        "validation-max",
        "validation-between",
        "validation-in",
        "validation-not-in",
        "validation-integer",
        "validation-numeric",
        "validation-boolean",
        "validation-alpha",
        "validation-alpha-num",
        "validation-alpha-dash",
        "validation-url",
        "validation-http-url",
        "validation-uuid",
        "validation-required-if",
        "validation-required-with",
        "validation-required-with-all",
        "validation-required-unless",
        "validation-same",
        "validation-different",
        "validation-confirmed",
        "validation-unique",
    ];

    scope_locale(Locale::parse("en").unwrap(), async move {
        for key in keys {
            assert!(Lang::has(key), "embedded catalog is missing `{key}`");
        }
    })
    .await;
}
