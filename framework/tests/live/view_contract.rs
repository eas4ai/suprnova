//! Checked view authoring and document-response adaptation through `suprnova`.

use std::error::Error as _;

use suprnova::view::{
    AssetSet, DocumentCachePolicy, DocumentResponseIntent, HeaderName, HeaderValue, RenderLimits,
    TrustedHtml, TrustedMarkupReason, ViewName, ViewRenderer, document_response,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/document.html")]
struct DocumentView<'a> {
    title: &'a str,
    ordinary: &'a str,
    trusted: &'a TrustedHtml,
}

#[test]
fn checked_view_escapes_strings_and_emits_only_audited_trusted_html() {
    let trusted = TrustedHtml::framework_static(
        "<strong>audited</strong>",
        TrustedMarkupReason::new("framework test status markup").expect("reason"),
    )
    .expect("trusted markup");
    let rendered = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            ViewName::parse("live/document.html").expect("view name"),
            &DocumentView {
                title: "Checked view",
                ordinary: "<script>alert('escaped')</script>",
                trusted: &trusted,
            },
            DocumentResponseIntent::html(suprnova::StatusCode::CREATED)
                .expect("response intent")
                .with_header(
                    HeaderName::from_static("x-suprnova-view"),
                    HeaderValue::from_static("checked"),
                )
                .expect("typed header")
                .with_cache(DocumentCachePolicy::NoStore),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("checked document");

    let response = document_response(rendered).expect("framework response");
    let body = std::str::from_utf8(response.body()).expect("utf-8 body");

    assert_eq!(response.status_code(), 201);
    assert_eq!(response.header_value("x-suprnova-view"), Some("checked"));
    assert_eq!(response.header_value("cache-control"), Some("no-store"));
    assert_eq!(
        response.header_value("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(body.contains("&#60;script&#62;"));
    assert!(!body.contains("<script>alert"));
    assert!(body.contains("<strong>audited</strong>"));
}

#[test]
fn generated_view_implements_the_framework_owned_checked_trait() {
    fn assert_checked<T: suprnova::view::ViewTemplate>() {}

    assert_checked::<DocumentView<'static>>();
}

#[test]
fn non_text_typed_headers_fail_without_publishing_a_partial_response() {
    let trusted = TrustedHtml::framework_static(
        "<strong>audited</strong>",
        TrustedMarkupReason::new("framework test status markup").expect("reason"),
    )
    .expect("trusted markup");
    let intent = DocumentResponseIntent::html(suprnova::StatusCode::OK)
        .expect("response intent")
        .with_header(
            HeaderName::from_static("x-binary"),
            HeaderValue::from_bytes(&[0x80]).expect("valid opaque header bytes"),
        )
        .expect("bounded typed header");
    let rendered = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            ViewName::parse("live/document.html").expect("view name"),
            &DocumentView {
                title: "Checked view",
                ordinary: "safe",
                trusted: &trusted,
            },
            intent,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("checked document");

    let error = match document_response(rendered) {
        Ok(_) => panic!("text response adapter must fail closed"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        suprnova::view::DocumentResponseErrorKind::NonTextHeaderValue
    );
    assert!(error.source().is_some());
}
