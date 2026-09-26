# Feature map: (no chapter) Markdown content and docs pipeline

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 16 checked.

## Rust API: suprnova

### `suprnova::content::docs` (private module; items are public through re-exports)

- [ ] fn `suprnova::content::build_docs` · framework/src/content/docs.rs:82
- [ ] struct `suprnova::content::DocsBuildConfig` · framework/src/content/docs.rs:12
  - Public fields: `source_dir`, `output_dir`, `toc_file`
- [ ] struct `suprnova::content::DocsCatalog` · framework/src/content/docs.rs:42
  - Public fields: `chapters`, `search`
- [ ] struct `suprnova::content::DocsCatalogEntry` · framework/src/content/docs.rs:51
  - Public fields: `slug`, `title`, `excerpt`, `headings`, `previous`, `next`
- [ ] struct `suprnova::content::DocsChapter` · framework/src/content/docs.rs:23
  - Public fields: `slug`, `title`, `html`, `excerpt`, `headings`, `previous`, `next`
- [ ] struct `suprnova::content::DocsSearchEntry` · framework/src/content/docs.rs:68
  - Public fields: `slug`, `title`, `excerpt`, `headings`, `plain_text`

### `suprnova::content::headings` (private module; items are public through re-exports)

- [ ] fn `suprnova::content::slugify_heading` · framework/src/content/headings.rs:19
- [ ] struct `suprnova::content::Heading` · framework/src/content/headings.rs:9
  - Public fields: `level`, `id`, `title`

### `suprnova::content::markdown` (private module; items are public through re-exports)

- [ ] struct `suprnova::content::MarkdownOptions` · framework/src/content/markdown.rs:33
  - Public fields: `unsafe_html`, `heading_anchor_prefix`, `render_math`
- [ ] struct `suprnova::content::MarkdownRenderer` · framework/src/content/markdown.rs:67
  - [ ] fn `suprnova::content::MarkdownRenderer::new` · framework/src/content/markdown.rs:73
  - [ ] fn `suprnova::content::MarkdownRenderer::options` · framework/src/content/markdown.rs:78
  - [ ] fn `suprnova::content::MarkdownRenderer::render` · framework/src/content/markdown.rs:83
- [ ] struct `suprnova::content::RenderedMarkdown` · framework/src/content/markdown.rs:54
  - Public fields: `html`, `plain_text`, `excerpt`, `headings`
- [ ] enum `suprnova::content::ContentError` · framework/src/content/markdown.rs:22
  - Variants: `Io`, `Json`
- [ ] type `suprnova::content::ContentResult` · framework/src/content/markdown.rs:18
