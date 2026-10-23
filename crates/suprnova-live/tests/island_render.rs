//! Island rendering authority and boundary tests.

mod component_support;

use std::any::Any;
use std::sync::Arc;

use askama::Template;
use bytes::Bytes;

use suprnova_live::component::RenderContext;
use suprnova_live::component::generated::render_component_view;
use suprnova_live::identity::{InstanceId, Revision, UnixMillis, ViewName};
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

#[derive(Template)]
#[template(source = "<button>{{ content }}</button>", ext = "html")]
struct ComponentFragmentTemplate<'a> {
    content: &'a str,
}

/// A component fragment that shows the host's `app_name` value when there
/// is one and a fallback when there is none.
#[derive(Template)]
#[template(
    source = r#"<p>{% if let Ok(name) = "app_name"|value::<String> %}{{ name }}{% else %}no app name{% endif %}</p>"#,
    ext = "html"
)]
struct SharedNameFragment;

/// A component fragment that requires the host's `app_name` value.
#[derive(Template)]
#[template(source = r#"<p>{{ ("app_name"|value::<String>)? }}</p>"#, ext = "html")]
struct RequiredNameFragment;

/// Host runtime values holding one `app_name` entry.
struct AppName<T>(T);

impl<T: Any> askama::Values for AppName<T> {
    fn get_value<'a>(&'a self, key: &str) -> Option<&'a dyn Any> {
        (key == "app_name").then_some(&self.0 as &dyn Any)
    }
}

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

#[test]
fn raw_component_fragments_are_bounded_before_engine_wrapper_allocation() {
    let renderer =
        ViewRenderer::new(RenderLimits::new(8, 1, 1, 1, 64).expect("small renderer limits"))
            .expect("renderer");
    let accepted = IslandRender {
        body: Bytes::from_static(b"<p>x</p>"),
        assets: AssetSet::empty(),
        children: Vec::new(),
    };
    renderer
        .validate_island_fragment(view("tests/fragment.html"), &accepted)
        .expect("bounded fragment");

    let oversized = IslandRender {
        body: Bytes::from_static(b"<p>xx</p>"),
        assets: AssetSet::empty(),
        children: Vec::new(),
    };
    let error = renderer
        .validate_island_fragment(view("tests/fragment.html"), &oversized)
        .expect_err("body is rejected before wrapper allocation");
    assert_eq!(error.kind(), ViewErrorKind::BodyTooLarge);
}

#[test]
fn component_templates_render_checked_fragments_before_engine_root_assembly() {
    let rendered = renderer()
        .render_component_fragment(
            view("tests/component-fragment.html"),
            &ComponentFragmentTemplate { content: "ready" },
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("checked component fragment");

    assert_eq!(rendered.body, Bytes::from_static(b"<button>ready</button>"));
    assert!(
        !rendered
            .body
            .windows(23)
            .any(|bytes| bytes == b"data-suprnova-live-root")
    );
}

#[test]
fn component_fragments_read_the_runtime_values_the_host_supplies() {
    let rendered = renderer()
        .render_component_fragment_with_values(
            view("tests/component-fragment.html"),
            &SharedNameFragment,
            AssetSet::empty(),
            Vec::new(),
            &AppName(String::from("Acme <&>")),
        )
        .expect("component fragment with host values");

    assert_eq!(
        rendered.body,
        Bytes::from_static(b"<p>Acme &#60;&#38;&#62;</p>"),
        "a host value reaches the template, escaped"
    );
}

#[test]
fn component_fragments_receive_no_runtime_values_by_default() {
    let rendered = renderer()
        .render_component_fragment(
            view("tests/component-fragment.html"),
            &SharedNameFragment,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect("component fragment without host values");

    assert_eq!(rendered.body, Bytes::from_static(b"<p>no app name</p>"));
}

#[test]
fn a_component_fragment_that_requires_a_missing_or_mistyped_value_fails() {
    let missing = renderer()
        .render_component_fragment(
            view("tests/component-fragment.html"),
            &RequiredNameFragment,
            AssetSet::empty(),
            Vec::new(),
        )
        .expect_err("no host value");
    assert_eq!(missing.kind(), ViewErrorKind::MissingViewData);

    let mistyped = renderer()
        .render_component_fragment_with_values(
            view("tests/component-fragment.html"),
            &RequiredNameFragment,
            AssetSet::empty(),
            Vec::new(),
            &AppName(7_u32),
        )
        .expect_err("a host value of another type");
    assert_eq!(mistyped.kind(), ViewErrorKind::InvalidViewData);
}

#[test]
fn generated_component_views_read_the_view_values_the_host_installed() {
    let request = component_support::trusted_context_with_view_values(Arc::new(AppName(
        String::from("Acme"),
    )));
    let instance =
        InstanceId::from_bytes(&component_support::bytes::<16>(0x20)).expect("instance identity");
    let instanced = RenderContext::new(
        &request,
        &instance,
        Revision::new(0),
        UnixMillis::new(2_000),
    );
    let rendered = render_component_view(
        &instanced,
        component_support::metadata(),
        &SharedNameFragment,
    )
    .expect("instanced component view");
    assert_eq!(rendered.body, Bytes::from_static(b"<p>Acme</p>"));

    // A public seed reads the same values: a host installs only values it
    // shows every visitor.
    let seed = RenderContext::for_public_seed(&request, Revision::new(0), UnixMillis::new(2_000));
    let rendered = render_component_view(&seed, component_support::metadata(), &SharedNameFragment)
        .expect("public seed component view");
    assert_eq!(rendered.body, Bytes::from_static(b"<p>Acme</p>"));

    // A host that installs no values gives the view none.
    let plain = component_support::trusted_context();
    let instanced = RenderContext::new(&plain, &instance, Revision::new(0), UnixMillis::new(2_000));
    let rendered = render_component_view(
        &instanced,
        component_support::metadata(),
        &SharedNameFragment,
    )
    .expect("component view without host values");
    assert_eq!(rendered.body, Bytes::from_static(b"<p>no app name</p>"));
}
