//! `Lang`'s ICU4X-backed formatting surface (`number`/`currency`/`date`/
//! `time`/`datetime`/`list`/`relative`) and the `DATETIME()` Fluent
//! function. Formatting reads only the current locale — catalogs can be
//! empty — but every test still binds a `Translator` (via
//! `bind_empty_translator`/`bind_translator`, mirroring
//! `localization_translate.rs`'s `lang_facade` module) and runs
//! `#[serial_test::serial]`, since binding the container is shared
//! process-global state that would otherwise race across the tests in
//! this file.

#![cfg(feature = "localization")]

use std::fs;
use std::sync::Arc;
use suprnova::{
    DateStyle, FluentTranslator, Lang, ListStyle, Locale, LocalizationConfig, RelativeUnit,
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

/// A `Translator` with no catalogs beyond the framework's own embedded
/// English one — formatting doesn't consult it, but every test binds one
/// anyway to mirror the established harness shape.
fn bind_empty_translator() {
    let tmp = tempfile::tempdir().unwrap();
    bind_translator(tmp.path());
}

#[tokio::test]
#[serial_test::serial]
async fn numbers_localize() {
    bind_empty_translator();
    scope_locale(Locale::parse("en-US").unwrap(), async {
        assert_eq!(Lang::number(1234567.89), "1,234,567.89");
    })
    .await;
    scope_locale(Locale::parse("de-DE").unwrap(), async {
        assert_eq!(Lang::number(1234567.89), "1.234.567,89");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn lists_localize() {
    bind_empty_translator();
    scope_locale(Locale::parse("en").unwrap(), async {
        assert_eq!(Lang::list(&["a", "b", "c"], ListStyle::And), "a, b, and c");
    })
    .await;
    scope_locale(Locale::parse("es").unwrap(), async {
        assert_eq!(Lang::list(&["a", "b", "c"], ListStyle::And), "a, b y c");
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn dates_and_relative_and_currency_render_nonempty_and_locale_distinct() {
    // Exact ICU output strings for dates/currency are CLDR-version
    // dependent; pin *distinctness and shape*, not bytes.
    bind_empty_translator();
    let dt = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
        .unwrap()
        .and_hms_opt(14, 30, 0)
        .unwrap();
    let (en, de) = (
        Locale::parse("en-US").unwrap(),
        Locale::parse("de-DE").unwrap(),
    );
    let d_en = scope_locale(en.clone(), async { Lang::date(&dt, DateStyle::Long) }).await;
    let d_de = scope_locale(de.clone(), async { Lang::date(&dt, DateStyle::Long) }).await;
    assert!(d_en.contains("2026") && d_de.contains("2026") && d_en != d_de);

    let c_en = scope_locale(en.clone(), async { Lang::currency(19.99, "USD") }).await;
    assert!(c_en.contains("19.99") || c_en.contains("19,99"));

    let r_en = scope_locale(en, async { Lang::relative(-3, RelativeUnit::Day) }).await;
    assert_eq!(r_en, "3 days ago");
}

#[tokio::test]
#[serial_test::serial]
async fn datetime_fluent_function_formats_inside_a_message() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "app.ftl",
        r#"published = Published { DATETIME($when, dateStyle: "medium") }"#,
    );
    bind_translator(tmp.path());
    scope_locale(Locale::parse("en").unwrap(), async {
        let out = suprnova::__!("published", when: "2026-08-01T14:30:00");
        assert!(out.contains("2026"), "got: {out}");
    })
    .await;
}
