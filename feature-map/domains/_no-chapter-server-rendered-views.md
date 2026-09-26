# Feature map: (no chapter) server-rendered views

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 19 checked.

## Rust API: suprnova

### Public but unnameable

- [ ] trait `suprnova::view::sealed::Sealed` · framework/src/view/mod.rs:42 (sealed trait: a supertrait that stops implementations outside the crate)

### Re-exported from other crates

- [ ] trait `suprnova::view::FilterValues` re-exports `askama::values::Values`

### `suprnova::view::response` (private module; items are public through re-exports)

- [ ] fn `suprnova::view::document_response` · framework/src/view/response.rs:54
- [ ] struct `suprnova::view::DocumentResponseError` · framework/src/view/response.rs:19
  - [ ] fn `suprnova::view::DocumentResponseError::kind` · framework/src/view/response.rs:27
- [ ] enum `suprnova::view::DocumentResponseErrorKind` · framework/src/view/response.rs:12
  - Variants: `NonTextHeaderValue`

### `suprnova::view`

- [ ] struct `suprnova::view::ViewRenderer` · framework/src/view/mod.rs:87
  - [ ] fn `suprnova::view::ViewRenderer::new` · framework/src/view/mod.rs:93
  - [ ] fn `suprnova::view::ViewRenderer::render_document` · framework/src/view/mod.rs:98
  - [ ] fn `suprnova::view::ViewRenderer::render_island` · framework/src/view/mod.rs:116
  - [ ] fn `suprnova::view::ViewRenderer::validate_island_fragment` · framework/src/view/mod.rs:132
  - [ ] fn `suprnova::view::ViewRenderer::validate_island_output` · framework/src/view/mod.rs:141
- [ ] enum `suprnova::view::TemplateFailure` · framework/src/view/mod.rs:32
  - Variants: `MissingData`, `InvalidData`, `Failed`
- [ ] trait `suprnova::view::ViewTemplate` · framework/src/view/mod.rs:61
  - [ ] fn `suprnova::view::ViewTemplate::render_view` · framework/src/view/mod.rs:63 (required)
- [ ] type `suprnova::view::FilterResult` · framework/src/view/mod.rs:24

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::view` · suprnova-macros/src/lib.rs:81 (re-exported as `suprnova::view`)
  - Form: attribute `#[view]`
  - [ ] argument `path = "..."` · suprnova-macros/src/view.rs:82
- [ ] proc macro `suprnova_macros::view_filter` · suprnova-macros/src/lib.rs:87 (re-exported as `suprnova::view_filter`)
  - Form: attribute `#[view_filter]`
