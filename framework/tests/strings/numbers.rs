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
