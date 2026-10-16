//! Canonical document rendering and conformance projection tests.

use askama::Template;
use bytes::Bytes;
use http::StatusCode;
use http::header::{HeaderName, HeaderValue, SET_COOKIE};

use suprnova_live::identity::{ComponentName, IslandSlot, ViewName};
use suprnova_live::view::{
    AssetSet, CanonicalDocumentConformance, CanonicalDocumentRequest, DocumentCachePolicy,
    DocumentMediaType, DocumentResponseIntent, MountMetadata, MountSnapshotKind, RenderLimits,
    ViewErrorKind, ViewRenderer,
};

#[derive(Template)]
#[template(path = "tests/document.html")]
struct DocumentTemplate<'a> {
    title: &'a str,
    content: &'a str,
}

#[derive(Template)]
#[template(path = "tests/incomplete_document.html")]
struct IncompleteDocumentTemplate;

#[derive(Template)]
#[template(path = "tests/raw_text_document.html")]
struct RawTextDocumentTemplate;

fn view(name: &str) -> ViewName {
    ViewName::parse(name).expect("view")
}

fn mount() -> MountMetadata {
    MountMetadata::new(
        IslandSlot::parse("primary").expect("slot"),
        ComponentName::parse("catalog.search").expect("component"),
        MountSnapshotKind::PublicSeed,
        Bytes::from_static(b"signed-public-seed"),
    )
    .expect("mount")
}

#[test]
fn document_render_owns_typed_status_headers_cache_and_mount_metadata() {
    let response = DocumentResponseIntent::html(StatusCode::CREATED)
        .expect("response")
        .with_header(
            HeaderName::from_static("x-suprnova-view"),
            HeaderValue::from_static("canonical"),
        )
        .expect("header")
        .with_cache(DocumentCachePolicy::Private);
    let rendered = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            view("tests/document.html"),
            &DocumentTemplate {
                title: "Catalog",
                content: "Initial search content",
            },
            response,
            AssetSet::empty(),
            vec![mount()],
        )
        .expect("document");

    assert_eq!(rendered.response.status(), StatusCode::CREATED);
    assert_eq!(rendered.response.headers()["x-suprnova-view"], "canonical");
    assert_eq!(rendered.response.cache(), DocumentCachePolicy::Private);
    assert_eq!(rendered.response.media_type(), DocumentMediaType::HtmlUtf8);
    assert_eq!(rendered.mounts.len(), 1);
    assert!(
        std::str::from_utf8(&rendered.body)
            .expect("utf-8")
            .contains("Initial search content")
    );
    let diagnostic = format!("{rendered:?}");
    assert!(!diagnostic.contains("Initial search content"));
    assert!(!diagnostic.contains("canonical"));
}

#[test]
fn canonical_conformance_exposes_get_suppresses_head_and_honors_validator() {
    let rendered = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            view("tests/document.html"),
            &DocumentTemplate {
                title: "Catalog",
                content: "Visible before JavaScript",
            },
            DocumentResponseIntent::html(StatusCode::OK).expect("response"),
            AssetSet::empty(),
            vec![mount()],
        )
        .expect("document");

    let get =
        CanonicalDocumentConformance::project(&rendered, &CanonicalDocumentRequest::get(None));
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.response().media_type(), DocumentMediaType::HtmlUtf8);
    assert_eq!(get.response().cache(), DocumentCachePolicy::Private);
    assert!(get.body().starts_with(b"<!doctype html>"));
    assert!(
        std::str::from_utf8(get.body())
            .expect("utf-8")
            .contains("Visible before JavaScript")
    );

    let head =
        CanonicalDocumentConformance::project(&rendered, &CanonicalDocumentRequest::head(None));
    assert!(head.body().is_empty());
    assert_eq!(head.representation_length(), get.body().len());

    let conditional = CanonicalDocumentConformance::project(
        &rendered,
        &CanonicalDocumentRequest::get(Some(get.validator().clone())),
    );
    assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
    assert!(conditional.body().is_empty());
}

#[test]
fn incomplete_html_never_becomes_a_successful_canonical_document() {
    let error = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            view("tests/incomplete_document.html"),
            &IncompleteDocumentTemplate,
            DocumentResponseIntent::html(StatusCode::OK).expect("response"),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("complete html required");
    assert_eq!(error.kind(), ViewErrorKind::InvalidDocument);
}

#[test]
fn document_metadata_rejects_cookie_authority_and_unmatched_mounts() {
    let cookie_error = DocumentResponseIntent::html(StatusCode::OK)
        .expect("response")
        .with_header(SET_COOKIE, HeaderValue::from_static("session=secret"))
        .expect_err("cookies belong to host policy");
    assert_eq!(cookie_error.kind(), ViewErrorKind::ForbiddenResponseIntent);

    let mount_error = ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            view("tests/document.html"),
            &DocumentTemplate {
                title: "Catalog",
                content: "Initial content",
            },
            DocumentResponseIntent::html(StatusCode::OK).expect("response"),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("template declares an unmatched mount");
    assert_eq!(mount_error.kind(), ViewErrorKind::MountMetadataMismatch);
}

#[test]
fn html_inspection_does_not_treat_raw_text_as_mount_markup() {
    ViewRenderer::new(RenderLimits::standard())
        .expect("renderer")
        .render_document(
            view("tests/raw_text_document.html"),
            &RawTextDocumentTemplate,
            DocumentResponseIntent::html(StatusCode::OK).expect("response"),
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("raw text is not markup");
}
