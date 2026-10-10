//! Translation sources beyond the application's `lang/` directory:
//! `Lang::add_path`, `add_fallback_path`, `add_namespace` and `loader`, and
//! `FluentTranslator::from_sources`.
//!
//! The Lang tests bind a translator into the process-global container and
//! register sources in the process-wide registry, so they run `#[serial]`
//! and clear the registry when they end.

#![cfg(feature = "localization")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suprnova::{
    FluentTranslator, Lang, Locale, LocalizationConfig, TranslateArgs, TranslationSources,
    Translator, scope_locale,
};

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

fn write_lang(dir: &Path, locale: &str, file: &str, ftl: &str) {
    let d = dir.join(locale);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(file), ftl).unwrap();
}

fn en() -> Locale {
    Locale::parse("en").unwrap()
}

/// `lang/en/app.ftl` holding `hello = Hi` and `pkg/en/extra.ftl` holding
/// `bye = Bye`, under one temporary directory.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        write_lang(&root.path().join("lang"), "en", "app.ftl", "hello = Hi\n");
        write_lang(&root.path().join("pkg"), "en", "extra.ftl", "bye = Bye\n");
        Self { root }
    }

    fn lang(&self) -> PathBuf {
        self.root.path().join("lang")
    }

    fn pkg(&self) -> PathBuf {
        self.root.path().join("pkg")
    }
}

/// Clears the registered sources when a test starts and when it ends,
/// passed or not.
struct Registry;

impl Registry {
    fn cleared() -> Self {
        Lang::clear_sources_for_test();
        Self
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        Lang::clear_sources_for_test();
    }
}

fn bind(translator: FluentTranslator) {
    suprnova::container::App::bind::<dyn Translator>(Arc::new(translator));
}

async fn in_en<F: std::future::Future>(f: F) -> F::Output {
    scope_locale(en(), f).await
}

// --- Lang::add_path ----------------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn add_path_merges_a_directory_after_the_applications() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_path(fixture.pkg()).unwrap();

    in_en(async {
        assert_eq!(Lang::get("bye"), "Bye");
        assert_eq!(Lang::get("hello"), "Hi");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn a_message_both_define_answers_the_added_paths() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    write_lang(
        &fixture.lang(),
        "en",
        "shared.ftl",
        "shared = From the app\n",
    );
    write_lang(
        &fixture.pkg(),
        "en",
        "shared.ftl",
        "shared = From the package\n",
    );
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_path(fixture.pkg()).unwrap();

    in_en(async { assert_eq!(Lang::get("shared"), "From the package") }).await;
}

#[tokio::test]
#[serial_test::serial]
async fn a_later_added_path_wins_over_an_earlier_one() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    let later = fixture.root.path().join("later");
    write_lang(&fixture.pkg(), "en", "x.ftl", "which = earlier\n");
    write_lang(&later, "en", "x.ftl", "which = later\n");
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_path(fixture.pkg()).unwrap();
    Lang::add_path(&later).unwrap();

    in_en(async { assert_eq!(Lang::get("which"), "later") }).await;
}

#[tokio::test]
#[serial_test::serial]
async fn a_malformed_added_path_is_an_error_and_changes_nothing() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    let broken = fixture.root.path().join("broken");
    write_lang(&broken, "en", "bad.ftl", "this is not fluent {\n");
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    assert!(Lang::add_path(&broken).is_err());

    assert!(
        Lang::loader().paths().is_empty(),
        "a refused path stays registered"
    );
    in_en(async { assert_eq!(Lang::get("hello"), "Hi") }).await;
}

// --- Lang::add_fallback_path -------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn add_fallback_path_merges_a_directory_before_the_applications() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    write_lang(
        &fixture.lang(),
        "en",
        "shared.ftl",
        "shared = From the app\n",
    );
    write_lang(
        &fixture.pkg(),
        "en",
        "shared.ftl",
        "shared = From the package\n",
    );
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_fallback_path(fixture.pkg()).unwrap();

    in_en(async {
        assert_eq!(Lang::get("shared"), "From the app");
        assert_eq!(Lang::get("bye"), "Bye");
    })
    .await;
}

// --- Lang::add_namespace -----------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn add_namespace_reads_a_package_catalog_as_namespace_keys() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_namespace("courier", fixture.pkg()).unwrap();

    in_en(async {
        assert_eq!(Lang::get("courier::bye"), "Bye");
        assert!(Lang::has("courier::bye"));
        assert_eq!(
            Lang::get("bye"),
            "bye",
            "a namespaced message is not a plain one"
        );
        assert_eq!(Lang::get("other::bye"), "other::bye");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn the_applications_vendor_files_override_a_namespace() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    let vendor = fixture.lang().join("vendor").join("courier");
    write_lang(&vendor, "en", "x.ftl", "bye = Ciao\n");
    bind(FluentTranslator::from_dir(fixture.lang(), &config()).unwrap());

    Lang::add_namespace("courier", fixture.pkg()).unwrap();

    in_en(async { assert_eq!(Lang::get("courier::bye"), "Ciao") }).await;
    assert!(
        !Lang::available_locales()
            .iter()
            .any(|locale| locale.as_str() == "vendor"),
        "lang/vendor is not a locale"
    );
}

#[test]
#[serial_test::serial]
fn add_namespace_refuses_a_name_that_could_leave_its_directory() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    for name in [
        "../x", "", "..", "a/b", "a\\b", "a\0b", "a.b", "a::b", "a__b", "1st",
    ] {
        assert!(
            Lang::add_namespace(name, fixture.pkg()).is_err(),
            "{name:?} should be refused"
        );
    }
    assert!(Lang::loader().namespaces().is_empty());
}

#[test]
fn namespaced_messages_reach_the_served_catalog_in_the_encoded_form() {
    let fixture = Fixture::new();
    write_lang(
        &fixture.pkg(),
        "en",
        "more.ftl",
        "-brand = Courier\nfarewell = { bye } from { -brand }\n",
    );
    let sources = TranslationSources::new(fixture.lang())
        .namespace("courier", fixture.pkg())
        .unwrap();
    let translator = FluentTranslator::from_sources(sources, &config()).unwrap();

    let catalog = translator.catalog(&en()).unwrap();
    assert!(
        catalog.text.contains("courier__bye = Bye"),
        "{}",
        catalog.text
    );
    assert!(
        catalog
            .text
            .contains("courier__farewell = { courier__bye } from { -courier__brand }"),
        "references inside the namespace follow its ids: {}",
        catalog.text
    );
    assert_eq!(
        translator
            .translate(&en(), "courier::farewell", &TranslateArgs::new())
            .unwrap(),
        "Bye from Courier"
    );
}

#[test]
fn every_scaffolded_client_resolves_namespaced_keys_the_same_way() {
    let templates =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../suprnova-cli/src/templates/files/frontend");
    for client in [
        "vue/src/lib/lang.ts.tpl",
        "react/src/lib/lang.ts.tpl",
        "svelte/src/lib/lang.svelte.ts.tpl",
    ] {
        let text = fs::read_to_string(templates.join(client)).unwrap();
        assert!(
            text.contains("key.replace('::', '__')"),
            "{client} does not encode `namespace::key` as `namespace__key`"
        );
    }
}

// --- Sources registered before the translator is bound -----------------------

#[tokio::test]
#[serial_test::serial]
async fn sources_registered_before_the_translator_is_built_reach_it() {
    let _registry = Registry::cleared();
    let fixture = Fixture::new();
    let fallback = fixture.root.path().join("fallback");
    write_lang(
        &fallback,
        "en",
        "f.ftl",
        "only-fallback = From the fallback\n",
    );

    Lang::add_path(fixture.pkg()).unwrap();
    Lang::add_fallback_path(&fallback).unwrap();
    Lang::add_namespace("courier", fixture.pkg()).unwrap();

    let before = suprnova::app::paths::lang_path("");
    suprnova::app::paths::use_lang_path(fixture.lang());
    let loader = Lang::loader();
    suprnova::app::paths::use_lang_path(before);

    assert_eq!(loader.dir(), fixture.lang().as_path());
    assert_eq!(loader.paths(), [fixture.pkg()]);
    assert_eq!(loader.fallback_paths(), std::slice::from_ref(&fallback));
    assert_eq!(loader.namespaces(), [("courier".to_owned(), fixture.pkg())]);

    // The bootstrap builds the translator from the loader.
    bind(FluentTranslator::from_sources(loader, &config()).unwrap());
    in_en(async {
        assert_eq!(Lang::get("hello"), "Hi");
        assert_eq!(Lang::get("bye"), "Bye");
        assert_eq!(Lang::get("only-fallback"), "From the fallback");
        assert_eq!(Lang::get("courier::bye"), "Bye");
    })
    .await;
}

// --- FluentTranslator::from_sources ------------------------------------------

#[test]
fn from_dir_keeps_reading_one_directory() {
    let fixture = Fixture::new();
    let translator = FluentTranslator::from_dir(fixture.lang(), &config()).unwrap();
    assert!(translator.has(&en(), "hello"));
    assert!(!translator.has(&en(), "bye"));
}

#[test]
fn from_sources_reads_every_source_and_reloads_them() {
    let fixture = Fixture::new();
    let sources = TranslationSources::new(fixture.lang()).path(fixture.pkg());
    let translator = FluentTranslator::from_sources(sources, &config()).unwrap();
    assert_eq!(
        translator
            .translate(&en(), "bye", &TranslateArgs::new())
            .unwrap(),
        "Bye"
    );

    write_lang(&fixture.pkg(), "en", "extra.ftl", "bye = Farewell\n");
    translator.reload().unwrap();
    assert_eq!(
        translator
            .translate(&en(), "bye", &TranslateArgs::new())
            .unwrap(),
        "Farewell"
    );
}
