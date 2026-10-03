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
    let german = in_locale("de", || Lang::percentage(10.0, 0)).await;
    assert!(
        german.starts_with("10") && german.ends_with('%') && german.chars().count() == 4,
        "German writes a space before the sign: {german:?}"
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
