//! `Lang`'s ICU4X-backed formatting surface (`number`/`currency`/`date`/
//! `time`/`datetime`/`list`/`relative`) and the `DATETIME()` Fluent
//! function. Formatting reads only the current locale - catalogs can be
//! empty - but every test still binds a `Translator` (via
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

/// A `Translator` with no catalogs beyond the framework's own embedded
/// English one - formatting doesn't consult it, but every test binds one
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

/// `DATETIME($when, ...)` also accepts `$when` as an epoch-milliseconds
/// number, not just an ISO-8601 string - the other half of the
/// documented `$value` contract, previously untested.
#[tokio::test]
#[serial_test::serial]
async fn datetime_fluent_function_accepts_epoch_milliseconds() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "app.ftl",
        r#"published = Published { DATETIME($when, dateStyle: "medium") }"#,
    );
    bind_translator(tmp.path());
    scope_locale(Locale::parse("en").unwrap(), async {
        // 2026-08-01T14:30:00Z, in epoch milliseconds.
        let out = suprnova::__!("published", when: 1_785_594_600_000_i64);
        assert!(out.contains("2026"), "got: {out}");
    })
    .await;
}

/// An unrecognized `dateStyle`/`timeStyle` keyword must not fail silently -
/// `DATETIME()` warns (naming the option and the bad value) and falls
/// back to treating it as absent, same as `$value` itself does.
#[tokio::test]
#[serial_test::serial]
#[tracing_test::traced_test]
async fn datetime_fluent_function_warns_on_an_unrecognized_style_keyword() {
    let tmp = tempfile::tempdir().unwrap();
    write_lang(
        tmp.path(),
        "en",
        "app.ftl",
        r#"published = Published { DATETIME($when, dateStyle: "shrot", timeStyle: "short") }"#,
    );
    bind_translator(tmp.path());
    scope_locale(Locale::parse("en").unwrap(), async {
        let out = suprnova::__!("published", when: "2026-08-01T14:30:00");
        // The bad `dateStyle` is ignored; `timeStyle: "short"` still
        // applies, so this renders a time (no year), not a date.
        assert!(!out.contains("2026"), "got: {out}");
    })
    .await;

    assert!(logs_contain("dateStyle"));
    assert!(logs_contain("shrot"));
}

/// [`DateStyle::Full`] is the one mapping this crate invents beyond
/// ICU4X's own three-length `Length` enum (see `format.rs`'s doc
/// comment on `DateStyle`) - pin that it actually differs from `Long` by
/// carrying the weekday, not just alias it silently.
#[tokio::test]
#[serial_test::serial]
async fn date_style_full_includes_the_weekday() {
    bind_empty_translator();
    // 2026-08-01 is a Saturday.
    let dt = chrono::NaiveDate::from_ymd_opt(2026, 8, 1)
        .unwrap()
        .and_hms_opt(14, 30, 0)
        .unwrap();
    scope_locale(Locale::parse("en-US").unwrap(), async {
        let full = Lang::date(&dt, DateStyle::Full);
        let long = Lang::date(&dt, DateStyle::Long);
        assert!(full.contains("Saturday"), "got: {full}");
        assert!(!long.contains("Saturday"), "got: {long}");
        assert_ne!(full, long);
    })
    .await;
}

/// `try_currency`'s fraction digits come from `iso_currency`'s ISO 4217
/// minor-unit table, not a hardcoded 2 - JPY (0 decimals) and BHD (3
/// decimals) must render accordingly, and a code the table doesn't know
/// still falls back to 2 (unchanged prior behavior).
#[tokio::test]
#[serial_test::serial]
async fn currency_fraction_digits_follow_iso_4217() {
    bind_empty_translator();
    scope_locale(Locale::parse("en-US").unwrap(), async {
        let jpy = Lang::currency(1000.0, "JPY");
        assert_eq!(fraction_digit_count(&jpy), 0, "got: {jpy}");

        let bhd = Lang::currency(1.2345, "BHD");
        assert_eq!(fraction_digit_count(&bhd), 3, "got: {bhd}");

        // "AAA" isn't a real ISO 4217 code (iso_currency's own test
        // fixture for "not a currency"); the fallback stays 2 decimals.
        let unknown = Lang::currency(19.99, "AAA");
        assert_eq!(fraction_digit_count(&unknown), 2, "got: {unknown}");
    })
    .await;
}

/// Count the digits after the last `.` in a formatted amount - robust to
/// currency symbol/placement, which varies by code and locale.
fn fraction_digit_count(s: &str) -> usize {
    match s.rfind('.') {
        Some(pos) => s[pos + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count(),
        None => 0,
    }
}
