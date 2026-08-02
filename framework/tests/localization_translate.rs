//! FluentTranslator behavior: loading, merging, overriding the embedded
//! framework catalog, args, isolation, reload.

#![cfg(feature = "localization")]

use std::fs;
use suprnova::{FluentTranslator, Locale, LocalizationConfig, TranslateArgs, Translator};

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

#[test]
fn loads_merges_and_translates_with_args() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "app.ftl",
        "welcome = Welcome, { $name }!\n",
    );
    write_lang(tmp.path(), "en", "extra.ftl", "bye = Goodbye\n");
    write_lang(
        tmp.path(),
        "es",
        "app.ftl",
        "welcome = ¡Bienvenido, { $name }!\n",
    );

    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();

    let mut args = TranslateArgs::new();
    args.insert("name".into(), serde_json::json!("Ada"));
    let en = Locale::parse("en").unwrap();
    let es = Locale::parse("es").unwrap();

    // No isolation marks: exact string equality must hold.
    assert_eq!(t.translate(&en, "welcome", &args).unwrap(), "Welcome, Ada!");
    assert_eq!(
        t.translate(&es, "welcome", &args).unwrap(),
        "¡Bienvenido, Ada!"
    );
    // Files in one locale dir merge into one bundle.
    assert_eq!(
        t.translate(&en, "bye", &TranslateArgs::new()).unwrap(),
        "Goodbye"
    );
    // Missing key is an Err at the trait level (fallback lives in Lang).
    assert!(t.translate(&es, "bye", &TranslateArgs::new()).is_err());
    assert!(t.has(&en, "bye"));
    assert!(!t.has(&es, "bye"));

    let mut locales: Vec<String> = t.available_locales().iter().map(|l| l.as_str()).collect();
    locales.sort();
    assert_eq!(locales, vec!["en", "es"]);
}

#[test]
fn app_files_override_the_embedded_framework_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    // The framework embeds validation-invalid-data in English. An app
    // file redefining it must win.
    write_lang(
        tmp.path(),
        "en",
        "validation.ftl",
        "validation-invalid-data = Nope.\n",
    );
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();
    assert_eq!(
        t.translate(&en, "validation-invalid-data", &TranslateArgs::new())
            .unwrap(),
        "Nope."
    );
}

#[test]
fn embedded_catalog_exists_even_without_app_files() {
    let tmp = tempfile::tempdir().unwrap();
    // Empty lang dir: still boots, en still answers the embedded keys.
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();
    assert_eq!(
        t.translate(&en, "validation-invalid-data", &TranslateArgs::new())
            .unwrap(),
        "The given data was invalid."
    );
}

#[test]
fn malformed_ftl_fails_loudly_naming_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "bad.ftl",
        "this is not = = valid ftl {{{\n",
    );
    let err = FluentTranslator::from_dir(tmp.path(), &config()).unwrap_err();
    assert!(
        err.to_string().contains("bad.ftl"),
        "error must name the file: {err}"
    );
}

#[test]
fn isolation_marks_appear_only_when_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "hi = Hi { $name }\n");
    let mut args = TranslateArgs::new();
    args.insert("name".into(), serde_json::json!("X"));
    let en = Locale::parse("en").unwrap();

    let plain = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    assert_eq!(plain.translate(&en, "hi", &args).unwrap(), "Hi X");

    let isolating = FluentTranslator::from_dir(tmp.path(), &config().use_isolating(true)).unwrap();
    let out = isolating.translate(&en, "hi", &args).unwrap();
    assert!(out.contains('\u{2068}') && out.contains('\u{2069}'));
}

#[test]
fn reload_picks_up_changed_files() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "v = one\n");
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();
    assert_eq!(t.translate(&en, "v", &TranslateArgs::new()).unwrap(), "one");

    write_lang(tmp.path(), "en", "app.ftl", "v = two\n");
    t.reload().unwrap();
    assert_eq!(t.translate(&en, "v", &TranslateArgs::new()).unwrap(), "two");
}

#[test]
fn catalog_source_and_hash_are_stable_and_change_on_edit() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "v = one\n");
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();

    let c1 = t.catalog(&en).unwrap();
    assert!(c1.text.contains("v = one"));
    let c1b = t.catalog(&en).unwrap();
    assert_eq!(c1.hash, c1b.hash);

    write_lang(tmp.path(), "en", "app.ftl", "v = two\n");
    t.reload().unwrap();
    let c2 = t.catalog(&en).unwrap();
    assert_ne!(c1.hash, c2.hash);
    assert!(t.catalog(&Locale::parse("zz").unwrap()).is_none());
}

#[test]
fn reload_if_stale_detects_a_new_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "v = one\n");
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();

    // Nothing on disk changed yet: no reload should fire.
    assert!(!t.reload_if_stale().unwrap());

    // A brand new file changes the file set, not any existing mtime —
    // an mtime high-water mark alone wouldn't necessarily catch this on
    // a filesystem with coarse mtime resolution, but a file-set
    // comparison always does.
    write_lang(tmp.path(), "en", "extra.ftl", "w = two\n");
    assert!(t.reload_if_stale().unwrap());
    assert_eq!(t.translate(&en, "w", &TranslateArgs::new()).unwrap(), "two");
}

#[test]
fn reload_if_stale_detects_a_deleted_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(tmp.path(), "en", "app.ftl", "v = one\n");
    write_lang(tmp.path(), "en", "extra.ftl", "w = two\n");
    let t = FluentTranslator::from_dir(tmp.path(), &config()).unwrap();
    let en = Locale::parse("en").unwrap();
    assert_eq!(t.translate(&en, "w", &TranslateArgs::new()).unwrap(), "two");

    // Deleting a file can only hold or lower a max-mtime watermark, never
    // raise it — that's the bug this test guards against. A file-set
    // comparison catches it regardless.
    fs::remove_file(tmp.path().join("en").join("extra.ftl")).unwrap();
    assert!(t.reload_if_stale().unwrap());
    assert!(t.translate(&en, "w", &TranslateArgs::new()).is_err());
}

/// `Lang` facade + `__!` macro tests. These bind a process-global
/// container binding (`App::bind::<dyn Translator>`), and tests within
/// one integration-test binary run concurrently by default — a later
/// bind would race/overwrite an earlier one. `#[serial_test::serial]`
/// forces the three tests in this module to run one at a time relative
/// to each other (the other tests in this file never touch the
/// container, so they're unaffected and stay concurrent).
mod lang_facade {
    use super::*;
    use std::sync::Arc;
    use suprnova::{Lang, scope_locale};

    fn bind_translator(dir: &std::path::Path) {
        let t = FluentTranslator::from_dir(dir, &super::config()).unwrap();
        suprnova::container::App::bind::<dyn Translator>(Arc::new(t));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn falls_back_current_to_fallback_to_key() {
        let tmp = tempfile::tempdir().unwrap();
        super::write_lang(tmp.path(), "en", "app.ftl", "only-en = English only\n");
        super::write_lang(tmp.path(), "es", "app.ftl", "greet = Hola\n");
        bind_translator(tmp.path());

        scope_locale(Locale::parse("es").unwrap(), async {
            assert_eq!(Lang::get("greet"), "Hola");
            // Missing in es → falls back to en.
            assert_eq!(Lang::get("only-en"), "English only");
            // Missing everywhere → the key itself.
            assert_eq!(Lang::get("nope"), "nope");
            assert!(Lang::try_get("nope").is_err());
        })
        .await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn set_locale_inside_a_scope_switches_translation() {
        let tmp = tempfile::tempdir().unwrap();
        super::write_lang(tmp.path(), "en", "app.ftl", "greet = Hello\n");
        super::write_lang(tmp.path(), "es", "app.ftl", "greet = Hola\n");
        bind_translator(tmp.path());

        scope_locale(Locale::parse("en").unwrap(), async {
            assert_eq!(Lang::get("greet"), "Hello");
            Lang::set_locale(Locale::parse("es").unwrap());
            assert_eq!(Lang::locale().as_str(), "es");
            assert_eq!(Lang::get("greet"), "Hola");
        })
        .await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn macro_builds_args() {
        let tmp = tempfile::tempdir().unwrap();
        super::write_lang(tmp.path(), "en", "app.ftl", "hi = Hi { $name }, { $n }\n");
        bind_translator(tmp.path());
        scope_locale(Locale::parse("en").unwrap(), async {
            assert_eq!(suprnova::__!("hi", name: "Ada", n: 2), "Hi Ada, 2");
            // Missing required args → Fluent resolver error → try_get
            // errs → get()'s chain exhausts → the key comes back.
            assert_eq!(suprnova::__!("hi"), "hi");
        })
        .await;
    }
}
