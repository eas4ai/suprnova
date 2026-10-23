//! Framework-generated trusted markup shares the static boundary's bounds and reasons.

use suprnova_live::view::{TrustedHtml, TrustedMarkupErrorKind, TrustedMarkupReason};

#[test]
fn framework_generated_markup_keeps_its_bytes_and_reason() {
    let reason = TrustedMarkupReason::new("Live bootstrap tags built from validated digests")
        .expect("reason");
    let markup = TrustedHtml::framework_generated(
        "<script type=\"module\" src=\"/a.js\"></script>".to_owned(),
        reason.clone(),
    )
    .expect("bounded framework markup");
    assert_eq!(
        markup.to_string(),
        "<script type=\"module\" src=\"/a.js\"></script>"
    );
    assert_eq!(markup.reason(), &reason);
    assert_eq!(format!("{markup:?}"), "<TrustedHtml:framework-generated>");
}

#[test]
fn framework_generated_markup_is_bounded() {
    let reason = TrustedMarkupReason::new("bound check").expect("reason");
    let oversized = "x".repeat(2 * 1024 * 1024 + 1);
    let error = TrustedHtml::framework_generated(oversized, reason).expect_err("too large");
    assert_eq!(error.kind(), TrustedMarkupErrorKind::MarkupTooLarge);
}
