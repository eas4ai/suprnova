//! Keyed validation messages, translated at the serialization boundary.
//!
//! Rules emit a [`ValidationMessage`] (key + args + English fallback +
//! optional context prefix); `ValidationErrors::to_json` is the single
//! place that consults the catalog. These tests drive that seam end to
//! end: a built-in rule's key, a per-locale translation, the
//! untranslated keyless path, the `field-<name>` attribute-name lookup,
//! the top-level banner, a `.context()`-prefixed message still
//! translating, and the `#[derive(Validate)]` code → key mapping
//! including the bound-selecting `length`/`range` messages.

#![cfg(feature = "localization")]

use std::fs;
use std::sync::Arc;
use suprnova::validation::rule::{Rule, ValueRule, rules};
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
        parents: Default::default(),
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

#[tokio::test]
#[serial_test::serial]
async fn contexted_message_keeps_its_key_and_translates_with_the_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "es",
        "validation.ftl",
        "validation-required = El campo { $field } es obligatorio.\n",
    );
    bind_translator(tmp.path());

    let mut errs = ValidationErrors::new();
    errs.add("email", rules::Required.passes("").unwrap_err());
    let wrapped = suprnova::FrameworkError::Validation(errs).context("registration");

    let contexted = match wrapped {
        suprnova::FrameworkError::Validation(v) => v,
        other => panic!("expected Validation, got {other:?}"),
    };

    // The key survives contexting; the prefix rides beside it.
    let m = &contexted.errors["email"][0];
    assert_eq!(m.key, "validation-required");
    assert_eq!(m.prefix.as_deref(), Some("registration"));
    // Untranslated rendering is byte-identical to the pre-localization
    // flattened string.
    assert_eq!(m.to_string(), "registration: required");

    let bag = contexted.clone();
    scope_locale(Locale::parse("es").unwrap(), async move {
        let json = bag.to_json();
        assert_eq!(
            json["errors"]["email"][0].as_str().unwrap(),
            "registration: El campo email es obligatorio."
        );
    })
    .await;

    // English resolves the embedded catalog, still behind the prefix.
    scope_locale(Locale::parse("en").unwrap(), async move {
        let json = contexted.to_json();
        assert_eq!(
            json["errors"]["email"][0].as_str().unwrap(),
            "registration: The email field is required."
        );
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn derive_length_bounds_select_the_right_phrasing() {
    let tmp = tempfile::tempdir().unwrap();
    bind_translator(tmp.path());

    // One-sided `#[validate(length(min = 2))]` — the shape the scaffold
    // templates use — must still resolve, not fall back.
    let mut ve = validator::ValidationErrors::new();
    let mut err = validator::ValidationError::new("length");
    err.add_param("min".into(), &2);
    ve.add("name", err);
    let one_sided = ValidationErrors::from_validator(ve);

    let mut ve = validator::ValidationErrors::new();
    let mut err = validator::ValidationError::new("length");
    err.add_param("min".into(), &2);
    err.add_param("max".into(), &5);
    ve.add("name", err);
    let two_sided = ValidationErrors::from_validator(ve);

    scope_locale(Locale::parse("en").unwrap(), async move {
        assert_eq!(
            one_sided.to_json()["errors"]["name"][0].as_str().unwrap(),
            "The name field must be at least 2 characters."
        );
        assert_eq!(
            two_sided.to_json()["errors"]["name"][0].as_str().unwrap(),
            "The name field must be between 2 and 5 characters."
        );
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn author_supplied_message_beats_the_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "es",
        "validation.ftl",
        "validation-length = El campo { $field } es demasiado corto.\n",
    );
    bind_translator(tmp.path());

    // `#[validate(length(min = 1, message = "Password is required"))]` —
    // the shape the scaffold templates use.
    let mut ve = validator::ValidationErrors::new();
    let mut err = validator::ValidationError::new("length");
    err.add_param("min".into(), &1);
    err.message = Some("Password is required".into());
    ve.add("password", err);
    let errs = ValidationErrors::from_validator(ve);

    // Keyless: no catalog id can reach it.
    assert!(!errs.errors["password"][0].is_keyed());

    let spanish = errs.clone();
    scope_locale(Locale::parse("es").unwrap(), async move {
        assert_eq!(
            spanish.to_json()["errors"]["password"][0].as_str().unwrap(),
            "Password is required"
        );
    })
    .await;

    // Same under English, where the embedded catalog would otherwise
    // have answered with its own generic length wording.
    scope_locale(Locale::parse("en").unwrap(), async move {
        assert_eq!(
            errs.to_json()["errors"]["password"][0].as_str().unwrap(),
            "Password is required"
        );
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
        "validation-array-keys",
        "validation-distinct",
        "validation-password-mixed",
        "validation-password-letters",
        "validation-password-symbols",
        "validation-password-numbers",
        "validation-password-uncompromised",
    ];

    // The `#[derive(Validate)]` path speaks the validator crate's codes;
    // they reach the catalog through the same `_` → `-` transform
    // `from_validator` applies.
    let derive_codes = [
        "email",
        "url",
        "required",
        "length",
        "range",
        "must_match",
        "regex",
    ];

    scope_locale(Locale::parse("en").unwrap(), async move {
        for key in keys {
            assert!(Lang::has(key), "embedded catalog is missing `{key}`");
        }
        for code in derive_codes {
            let key = format!("validation-{}", code.replace('_', "-"));
            assert!(
                Lang::has(&key),
                "embedded catalog is missing `{key}` for derive code `{code}`"
            );
        }
    })
    .await;
}

/// I2 — `Lang::has` is documented as chain-aware: true when `key`
/// resolves in the *current* locale's catalog **or** the fallback's
/// (mirroring `Lang::get`/`try_get`, which also retry the fallback on a
/// miss). Every other `has`/`to_json` test in this file defines the
/// asserted key in the current locale — `every_builtin_key_resolves_in_
/// the_embedded_catalog` scopes `en` and checks the `en`-embedded
/// catalog, `builtin_rule_message_translates_per_locale` scopes `es`
/// and defines its key in `es`, and so on. A regression from
/// `Lang::has`'s chain-aware body to a current-only
/// `translator.has(&current, key)` would still pass every one of them.
///
/// Here `only-in-fallback` is defined solely in the `en` catalog (the
/// `fallback_locale` this file's `config()` configures); `es` gets its
/// own loaded catalog with an unrelated key, so the "current-locale
/// catalog exists but doesn't define the key" branch is exercised
/// rather than "no catalog loaded for `es` at all". Scoping `es` as
/// current and asserting `Lang::has("only-in-fallback")` is `true`
/// only passes if the fallback arm actually ran. A key defined in
/// neither catalog must still be `false`.
#[tokio::test]
#[serial_test::serial]
async fn has_is_true_for_a_key_defined_only_in_the_fallback_locale() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "custom.ftl",
        "only-in-fallback = English only\n",
    );
    write_lang(
        tmp.path(),
        "es",
        "custom.ftl",
        "unrelated-es-key = No tiene nada que ver\n",
    );
    bind_translator(tmp.path());

    scope_locale(Locale::parse("es").unwrap(), async move {
        assert!(
            Lang::has("only-in-fallback"),
            "a key defined only in the fallback locale's catalog must still resolve \
             via Lang::has's fallback arm"
        );
        assert!(
            !Lang::has("nowhere-at-all"),
            "a key defined in neither the current nor the fallback catalog must be false"
        );
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn array_keys_message_translates_per_locale() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "es",
        "validation.ftl",
        "validation-array-keys = El campo { $field } solo puede contener estas claves: \
         { $values }.\n",
    );
    bind_translator(tmp.path());

    let msg = rules::ArrayKeys(&["nombre", "correo"])
        .passes(&serde_json::json!({"nombre": "Ada", "rol": "admin"}))
        .unwrap_err();
    assert_eq!(msg.key, "validation-array-keys");

    let mut errs = ValidationErrors::new();
    errs.add("perfil", msg);

    scope_locale(Locale::parse("es").unwrap(), async move {
        let json = errs.to_json();
        let text = json["errors"]["perfil"][0].as_str().unwrap();
        assert_eq!(
            text,
            "El campo perfil solo puede contener estas claves: nombre, correo."
        );
    })
    .await;
}
