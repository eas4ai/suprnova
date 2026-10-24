#![allow(dead_code)]

use std::fmt;

use suprnova::live::{LiveComponent, live};
use suprnova::view::{
    AssetSet, DocumentResponseIntent, RenderLimits, TrustedHtml, ViewName, ViewRenderer,
    ViewTemplate,
};

pub struct OuterLabel(pub &'static str);

impl fmt::Display for OuterLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

pub mod filters {
    use suprnova::view::{FilterResult, FilterValues};
    pub use suprnova::view::filters::trusted_html;

    #[suprnova::view_filter]
    #[cfg(any())]
    fn disabled_filter(value: &str, _: &dyn FilterValues) -> FilterResult<String> {
        Ok(value.to_owned())
    }

    #[suprnova::view_filter]
    pub fn uppercase(value: &str, _: &dyn FilterValues) -> FilterResult<String> {
        Ok(value.to_uppercase())
    }
}

#[suprnova::view(path = "live/card.html")]
pub struct CardView<'a, T>
where
    T: fmt::Display,
{
    pub title: &'a str,
    pub value: &'a T,
}

#[suprnova::view(path = "live/status.html")]
pub(crate) struct StatusView<'a> {
    label: &'a OuterLabel,
    trusted: &'a TrustedHtml,
}

#[derive(LiveComponent)]
#[live(name = "fixture.status", view = "live/status.html")]
pub struct StatusComponent {
    #[model]
    label: String,
}

mod restricted {
    use super::*;

    #[suprnova::view(path = "live/restricted.html")]
    pub(super) struct RestrictedView<'a> {
        label: &'a OuterLabel,
        pub(self) self_label: &'a OuterLabel,
        pub(super) parent_label: &'a OuterLabel,
        pub(in super) relative_parent_label: &'a OuterLabel,
        pub(crate) crate_label: &'a OuterLabel,
        pub(in crate::restricted) absolute_self_label: &'a OuterLabel,
    }

    pub(super) fn new(label: &OuterLabel) -> RestrictedView<'_> {
        RestrictedView {
            label,
            self_label: label,
            parent_label: label,
            relative_parent_label: label,
            crate_label: label,
            absolute_self_label: label,
        }
    }
}

mod conditional {
    use super::*;

    #[suprnova::view(path = "live/disabled.html")]
    #[cfg(any())]
    pub(super) struct PlatformView;

    #[suprnova::view(path = "live/card.html")]
    #[cfg(all())]
    pub(super) struct PlatformView<'a> {
        pub title: &'a str,
        pub value: &'a OuterLabel,
    }
}

#[live]
impl StatusComponent {
    #[action]
    pub async fn clear(&mut self) {
        self.label.clear();
    }
}

pub fn render_card<T: fmt::Display>(value: &T) -> Result<Vec<u8>, suprnova::view::ViewError> {
    let rendered = ViewRenderer::new(RenderLimits::standard())?.render_document(
        ViewName::parse("live/card.html").expect("static view name"),
        &CardView {
            title: "fixture",
            value,
        },
        DocumentResponseIntent::html(suprnova::StatusCode::OK)?,
        AssetSet::empty(),
        Vec::new(),
    )?;
    Ok(rendered.body.to_vec())
}

pub fn render_component_view(
    label: &OuterLabel,
    trusted: &TrustedHtml,
) -> Result<Vec<u8>, suprnova::view::ViewError> {
    let rendered = ViewRenderer::new(RenderLimits::standard())?.render_island(
        ViewName::parse("live/status.html").expect("static view name"),
        &StatusView { label, trusted },
        AssetSet::empty(),
        Vec::new(),
    )?;
    Ok(rendered.body.to_vec())
}

pub fn inspect_restricted_view(label: &OuterLabel) -> String {
    let view = restricted::new(label);
    format!(
        "{}{}{}",
        view.parent_label, view.relative_parent_label, view.crate_label
    )
}

pub fn render_checked<T: ViewTemplate>(
    template: &T,
) -> Result<Vec<u8>, suprnova::view::ViewError> {
    let rendered = ViewRenderer::new(RenderLimits::standard())?.render_document(
        ViewName::parse("live/card.html").expect("static view name"),
        template,
        DocumentResponseIntent::html(suprnova::StatusCode::OK)?,
        AssetSet::empty(),
        Vec::new(),
    )?;
    Ok(rendered.body.to_vec())
}
