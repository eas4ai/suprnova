//! View substrate failure and publication contract tests.

use std::fmt;

use askama::Template;
use http::StatusCode;

use suprnova_live::identity::ViewName;
use suprnova_live::view::{
    AssetSet, DocumentResponseIntent, RenderLimits, ViewErrorKind, ViewRenderer,
};

mod filters {
    use askama::{Result, Values, get_value};

    #[askama::filter_fn]
    pub fn required_data<'a>(value: &'a str, values: &dyn Values) -> Result<&'a str> {
        let _: &String = get_value(values, "required")?;
        Ok(value)
    }
}

struct BrokenDisplay;

impl fmt::Display for BrokenDisplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("partial-secret")?;
        Err(fmt::Error)
    }
}

#[derive(Template)]
#[template(path = "tests/partial_failure.html")]
struct PartialFailureTemplate {
    broken: BrokenDisplay,
}

#[derive(Template)]
#[template(path = "tests/missing_view_data.html")]
struct MissingViewDataTemplate;

#[derive(Template)]
#[template(path = "tests/escaping.html")]
struct BoundedTemplate<'a> {
    value: &'a str,
}

fn renderer() -> ViewRenderer {
    ViewRenderer::new(RenderLimits::standard()).expect("renderer")
}

fn view(name: &str) -> ViewName {
    ViewName::parse(name).expect("view")
}

fn response() -> DocumentResponseIntent {
    DocumentResponseIntent::html(StatusCode::OK).expect("response")
}

#[test]
fn failed_render_discards_every_partial_byte() {
    let error = renderer()
        .render_document(
            view("tests/partial_failure.html"),
            &PartialFailureTemplate {
                broken: BrokenDisplay,
            },
            response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("display failure");

    assert_eq!(error.kind(), ViewErrorKind::TemplateRenderFailed);
    let diagnostic = format!("{error:?}");
    assert!(diagnostic.contains("tests/partial_failure.html"));
    assert!(!diagnostic.contains("partial-secret"));
    assert!(!diagnostic.contains("prefix-that-must-not-escape"));
}

#[test]
fn missing_runtime_view_data_has_a_distinct_source_oriented_failure() {
    let error = renderer()
        .render_document(
            view("tests/missing_view_data.html"),
            &MissingViewDataTemplate,
            response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("missing runtime value");

    assert_eq!(error.kind(), ViewErrorKind::MissingViewData);
    assert!(format!("{error}").contains("tests/missing_view_data.html"));
}

#[test]
fn bounded_writer_aborts_before_oversized_html_can_be_published() {
    let limits = RenderLimits::new(64, 8, 8, 8, 256).expect("limits");
    let error = ViewRenderer::new(limits)
        .expect("renderer")
        .render_document(
            view("tests/escaping.html"),
            &BoundedTemplate {
                value: &"x".repeat(1_024),
            },
            response(),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("bounded output");
    assert_eq!(error.kind(), ViewErrorKind::BodyTooLarge);
}
