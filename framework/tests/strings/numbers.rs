//! PAR-037: percentage and abbreviation in the current locale.

use suprnova::{Lang, Locale, scope_locale};

async fn in_locale<T: Send + 'static>(locale: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    scope_locale(Locale::parse(locale).unwrap(), async move { f() }).await
}

#[tokio::test]
async fn percentages_follow_the_locale() {
    assert_eq!(in_locale("en", || Lang::percentage(10.0, 0)).await, "10%");
    assert_eq!(
        in_locale("en", || Lang::percentage(12.345, 1)).await,
        "12.3%"
    );
    assert_eq!(in_locale("en", || Lang::percentage(0.5, 2)).await, "0.50%");
    assert_eq!(
        in_locale("de", || Lang::percentage(10.0, 0)).await,
        "10\u{a0}%",
        "German writes a no-break space before the sign"
    );
    assert_eq!(
        in_locale("de", || Lang::percentage(12.5, 1))
            .await
            .chars()
            .next(),
        Some('1')
    );
    assert!(
        in_locale("de", || Lang::percentage(12.5, 1))
            .await
            .contains("12,5")
    );
}

#[tokio::test]
async fn abbreviations_use_laravels_suffixes_and_the_locales_digits() {
    assert_eq!(in_locale("en", || Lang::abbreviate(1000.0, 0)).await, "1K");
    assert_eq!(
        in_locale("en", || Lang::abbreviate(489_939.0, 0)).await,
        "490K"
    );
    assert_eq!(
        in_locale("en", || Lang::abbreviate(1_230_000.0, 2)).await,
        "1.23M"
    );
    assert_eq!(
        in_locale("en", || Lang::abbreviate(-2500.0, 1)).await,
        "-2.5K"
    );
    assert_eq!(in_locale("en", || Lang::abbreviate(999.0, 0)).await, "999");
    assert_eq!(
        in_locale("en", || Lang::abbreviate(5_000_000_000.0, 0)).await,
        "5B"
    );
    assert_eq!(
        in_locale("en", || Lang::abbreviate(7.2e12, 1)).await,
        "7.2T"
    );
    assert_eq!(in_locale("en", || Lang::abbreviate(3e15, 0)).await, "3Q");
    assert_eq!(
        in_locale("de", || Lang::abbreviate(1_230_000.0, 2)).await,
        "1,23M"
    );
}

/// Laravel 13.34.0 gave each of these.
#[tokio::test]
async fn abbreviations_round_as_laravel_does() {
    for (value, precision, written) in [
        (0.005, 0, "0"),
        (0.0042, 2, "0.00"),
        (0.001, 0, "0"),
        (0.000_002, 1, "0.0"),
        (-0.0001, 2, "0.00"),
        (-0.4, 0, "0"),
        (0.0, 2, "0.00"),
        (999_999.0, 0, "1M"),
        (999_950.0, 1, "1.0M"),
        (999.9, 0, "1K"),
        (999.4, 0, "999"),
        (999_999_999.0, 0, "1B"),
        (-999_999.0, 0, "-1M"),
        (1.5e15, 1, "1.5Q"),
        (3e18, 0, "3KQ"),
        (1e21, 0, "1MQ"),
    ] {
        assert_eq!(
            in_locale("en", move || Lang::abbreviate(value, precision)).await,
            written,
            "abbreviate({value}, {precision})"
        );
    }
    assert_eq!(in_locale("de", || Lang::abbreviate(-0.4, 0)).await, "0");
}

#[tokio::test]
async fn a_percentage_that_rounds_to_zero_has_no_sign() {
    assert_eq!(in_locale("en", || Lang::percentage(-0.001, 0)).await, "0%");
    assert_eq!(in_locale("en", || Lang::percentage(-0.4, 0)).await, "0%");
    assert_eq!(in_locale("en", || Lang::percentage(-0.6, 0)).await, "-1%");
}

#[tokio::test]
async fn percentages_round_decimal_ties_half_up() {
    for (value, precision, expected) in [
        (0.12345, 4, "0.1235%"),
        (12.345, 2, "12.35%"),
        (12.5, 0, "13%"),
        (-12.5, 0, "-13%"),
        (0.00005, 4, "0.0001%"),
    ] {
        assert_eq!(
            in_locale("en", move || Lang::percentage(value, precision)).await,
            expected
        );
    }
    assert_eq!(Lang::percentage_in(10.0, 0, "de").unwrap(), "10\u{a0}%");
    assert!(Lang::percentage_in(10.0, 0, "not a locale!").is_err());
}

#[tokio::test]
async fn maximum_precision_drops_zeros_and_overrides_fixed_precision() {
    in_locale("en", || {
        assert_eq!(Lang::format(1.23, 4), "1.2300");
        assert_eq!(Lang::format_with_max_precision(1.2300, 2, Some(4)), "1.23");
        assert_eq!(
            Lang::format_with_max_precision(1.23456, 2, Some(4)),
            "1.2346"
        );
        assert_eq!(Lang::format_with_max_precision(-0.0001, 2, Some(2)), "0");
        assert_eq!(Lang::format_with_max_precision(1000.0, 2, Some(4)), "1,000");
        assert_eq!(
            Lang::percentage_with_max_precision(12.5, 0, Some(2)),
            "12.5%"
        );
        assert_eq!(Lang::percentage_with_max_precision(12.0, 0, Some(2)), "12%");
        assert_eq!(
            Lang::percentage_with_max_precision(12.345, 0, Some(2)),
            "12.35%"
        );
        assert_eq!(Lang::percentage_with_max_precision(12.0, 2, None), "12.00%");
    })
    .await;
    assert_eq!(
        in_locale("de", || Lang::format_with_max_precision(
            1234.50,
            0,
            Some(3)
        ))
        .await,
        "1.234,5"
    );
}

#[tokio::test]
async fn special_numbers_follow_icu_spellings() {
    in_locale("en", || {
        assert_eq!(Lang::format(f64::INFINITY, 0), "∞");
        assert_eq!(Lang::format(f64::NEG_INFINITY, 2), "-∞");
        assert_eq!(Lang::format(f64::NAN, 0), "NaN");
        assert_eq!(Lang::percentage(f64::INFINITY, 2), "∞%");
        assert_eq!(Lang::percentage(f64::NAN, 2), "NaN%");
        assert_eq!(Lang::try_abbreviate(f64::INFINITY, 0).unwrap(), "∞");
    })
    .await;
    assert_eq!(
        Lang::percentage_in(f64::NEG_INFINITY, 0, "de").unwrap(),
        "-∞\u{a0}%"
    );
}

#[test]
fn default_locale_and_scoped_closures_restore_without_request_state() {
    // Nextest runs each test in its own process, so this global change is isolated.
    Lang::use_locale("de").unwrap();
    assert_eq!(Lang::percentage(10.0, 0), "10\u{a0}%");
    assert_eq!(
        Lang::with_locale("fr", || {
            assert_eq!(Lang::locale().as_str(), "fr");
            Lang::with_locale("en", || Lang::percentage(10.0, 0)).unwrap()
        })
        .unwrap(),
        "10%"
    );
    assert_eq!(Lang::locale().as_str(), "de");
    let panic = std::panic::catch_unwind(|| {
        let _ = Lang::with_locale("fr", || panic!("closure failed"));
    });
    assert!(panic.is_err());
    assert_eq!(Lang::locale().as_str(), "de");
    assert!(Lang::use_locale("not a locale!").is_err());
    assert!(Lang::with_locale("not a locale!", || panic!("must not run")).is_err());
    assert_eq!(Lang::locale().as_str(), "de");
}

#[tokio::test]
async fn request_locale_wins_over_the_default_and_closures_restore_it() {
    Lang::use_locale("de").unwrap();
    scope_locale(Locale::parse("en").unwrap(), async {
        assert_eq!(Lang::percentage(10.0, 0), "10%");
        Lang::with_locale("fr", || assert_eq!(Lang::locale().as_str(), "fr")).unwrap();
        assert_eq!(Lang::locale().as_str(), "en");
        Lang::use_locale("fr").unwrap();
        assert_eq!(Lang::locale().as_str(), "en");
    })
    .await;
    assert_eq!(Lang::locale().as_str(), "fr");
}

#[tokio::test]
async fn plain_formats_and_abbreviations_keep_laravels_half_even_rounding() {
    in_locale("en", || {
        assert_eq!(Lang::format(2.5, 0), "2");
        assert_eq!(Lang::format(3.5, 0), "4");
        assert_eq!(Lang::abbreviate(2500.0, 0), "2K");
        assert_eq!(Lang::percentage(2.5, 0), "3%");
    })
    .await;
}
