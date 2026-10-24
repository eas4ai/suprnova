#[test]
fn report_009_manual_covers_drivers_settings_helpers_scrubbing_and_kits() {
    let chapter = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../manual/error-reporting.md"
    ))
    .expect("the reporting manual chapter exists");
    for subject in [
        "sentry",
        "log",
        "null",
        "REPORTING_DRIVER",
        "REPORTING_DSN",
        "REPORTING_RELEASE",
        "APP_ENV",
        "REPORTING_SEND_PII",
        "REPORTING_LOGS",
        "REPORTING_TRACES_SAMPLE_RATE",
        "REPORTING_PUBLIC_DSN",
        "VITE_REPORTING_DSN",
        "report_if",
        "report_unless",
        "rescue",
        "dont_report",
        "dont_report_when",
        "dont_report_duplicates",
        "hidden",
        "SecretString",
        "request bodies",
        "Svelte",
        "React",
        "Vue",
        "Live",
        "docker:compose --with-rustrak",
        "docker run",
        "SESSION_SECRET_KEY",
        "### Why Suprnova diverges",
        "sentry/sentry-laravel",
        "## Next",
    ] {
        assert!(
            chapter.contains(subject),
            "missing manual subject {subject}"
        );
    }
    let errors = include_str!("../../../manual/errors.md");
    assert!(errors.contains("error-reporting.md"));
    assert!(!errors.contains("pub struct SentryReporter"));
    assert!(!errors.contains("sentry::capture_message"));
    let index = include_str!("../../../manual/documentation.md");
    assert!(index.contains("[Error Reporting](error-reporting.md)"));
}
