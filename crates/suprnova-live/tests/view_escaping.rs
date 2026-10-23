//! Escaping and explicit trusted-markup boundary tests.

use askama::Template;
use http::StatusCode;

use suprnova_live::identity::ViewName;
use suprnova_live::view::{
    AssetSet, DocumentResponseIntent, RegisteredSanitizer, RenderLimits, SanitizerFailure,
    SanitizerId, TrustedHtml, TrustedMarkupErrorKind, TrustedMarkupReason, ViewRenderer,
};

mod filters {
    pub use suprnova_live::view::filters::trusted_html;
}

#[derive(Template)]
#[template(path = "tests/escaping.html")]
struct EscapingTemplate<'a> {
    value: &'a str,
}

#[derive(Template)]
#[template(path = "tests/trusted.html")]
struct TrustedTemplate<'a> {
    markup: &'a TrustedHtml,
}

fn renderer() -> ViewRenderer {
    ViewRenderer::new(RenderLimits::standard()).expect("renderer")
}

fn view(name: &str) -> ViewName {
    ViewName::parse(name).expect("view name")
}

fn html_response() -> DocumentResponseIntent {
    DocumentResponseIntent::html(StatusCode::OK).expect("response")
}

#[test]
fn askama_html_interpolation_is_escaped_by_default() {
    let rendered = renderer()
        .render_document(
            view("tests/escaping.html"),
            &EscapingTemplate {
                value: "<script>alert('nope')</script>",
            },
            html_response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("render");

    let body = std::str::from_utf8(&rendered.body).expect("utf-8");
    assert!(body.contains("&#60;script&#62;"));
    assert!(!body.contains("<script>alert"));
}

#[test]
fn only_explicit_trusted_html_filter_emits_unescaped_markup() {
    let reason = TrustedMarkupReason::new("framework-owned static status icon").expect("reason");
    let markup = TrustedHtml::framework_static("<strong>ready</strong>", reason).expect("markup");
    let rendered = renderer()
        .render_document(
            view("tests/trusted.html"),
            &TrustedTemplate { markup: &markup },
            html_response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("render");

    let body = std::str::from_utf8(&rendered.body).expect("utf-8");
    assert!(body.contains("<strong>ready</strong>"));
    assert!(!format!("{markup:?}").contains("ready"));
}

#[test]
fn registered_sanitizer_output_can_cross_the_same_audited_boundary() {
    fn strip_script(input: &str, output: &mut dyn std::fmt::Write) -> Result<(), SanitizerFailure> {
        output
            .write_str(&input.replace("<script>", "").replace("</script>", ""))
            .map_err(|_| SanitizerFailure)
    }

    let sanitizer = RegisteredSanitizer::new(
        SanitizerId::parse("tests.strip-script").expect("sanitizer id"),
        strip_script,
    );
    let markup = sanitizer
        .sanitize(
            "<em>safe</em><script>bad()</script>",
            TrustedMarkupReason::new("registered test sanitizer").expect("reason"),
            1_024,
        )
        .expect("sanitized markup");
    let rendered = renderer()
        .render_document(
            view("tests/trusted.html"),
            &TrustedTemplate { markup: &markup },
            html_response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("render");
    let body = std::str::from_utf8(&rendered.body).expect("utf-8");

    assert!(body.contains("<em>safe</em>bad()"));
    assert!(!body.contains("<script>"));
}

#[test]
fn registered_sanitizer_writes_through_the_framework_byte_ceiling() {
    fn overproduce(_: &str, output: &mut dyn std::fmt::Write) -> Result<(), SanitizerFailure> {
        output
            .write_str(&"x".repeat(2_048))
            .map_err(|_| SanitizerFailure)
    }

    let sanitizer = RegisteredSanitizer::new(
        SanitizerId::parse("tests.overproduce").expect("sanitizer id"),
        overproduce,
    );
    let error = sanitizer
        .sanitize(
            "input",
            TrustedMarkupReason::new("bounded sanitizer output").expect("reason"),
            1_024,
        )
        .expect_err("bounded output");
    assert_eq!(error.kind(), TrustedMarkupErrorKind::MarkupTooLarge);
}
