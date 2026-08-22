//! Island rendering authority and boundary tests.

use askama::Template;

use suprnova_live::identity::ViewName;
use suprnova_live::view::{AssetSet, IslandRender, RenderLimits, ViewErrorKind, ViewRenderer};

#[derive(Template)]
#[template(path = "tests/island.html")]
struct IslandTemplate<'a> {
    content: &'a str,
}

#[derive(Template)]
#[template(path = "tests/multiple_island_roots.html")]
struct MultipleRootsTemplate;

#[derive(Template)]
#[template(path = "tests/executable_mount.html")]
struct ExecutableMountTemplate;

#[derive(Template)]
#[template(path = "tests/directives.html")]
struct DirectiveTemplate;

fn view(name: &str) -> ViewName {
    ViewName::parse(name).expect("view")
}

fn renderer() -> ViewRenderer {
    ViewRenderer::new(RenderLimits::standard()).expect("renderer")
}

#[test]
fn island_result_has_markup_assets_and_children_but_no_response_authority() {
    let rendered = renderer()
        .render_island(
            view("tests/island.html"),
            &IslandTemplate {
                content: "server rendered",
            },
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("island");
    let diagnostic = format!("{rendered:?}");

    let IslandRender {
        body,
        assets,
        children,
    } = rendered;
    assert!(
        std::str::from_utf8(&body)
            .expect("utf-8")
            .contains("server rendered")
    );
    assert!(assets.is_empty());
    assert!(children.is_empty());
    assert!(!diagnostic.contains("server rendered"));
}

#[test]
fn multiple_island_roots_fail_deterministically() {
    let error = renderer()
        .render_island(
            view("tests/multiple_island_roots.html"),
            &MultipleRootsTemplate,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("one root");
    assert_eq!(error.kind(), ViewErrorKind::MultipleIslandRoots);
}

#[test]
fn executable_elements_cannot_be_used_as_mount_boundaries() {
    let error = renderer()
        .render_island(
            view("tests/executable_mount.html"),
            &ExecutableMountTemplate,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("inert boundary");
    assert_eq!(error.kind(), ViewErrorKind::ExecutableMountMetadata);
}

#[test]
fn declarative_live_and_stimulus_attributes_pass_through_as_html() {
    let rendered = renderer()
        .render_island(
            view("tests/directives.html"),
            &DirectiveTemplate,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("directive markup");
    let body = std::str::from_utf8(&rendered.body).expect("utf-8");
    assert!(body.contains("data-controller=\"menu\""));
    assert!(body.contains("live:click=\"open\""));
}
