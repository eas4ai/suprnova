use suprnova::content::MarkdownRenderer;

#[test]
fn markdown_renderer_adds_anchors_highlights_and_strips_html() {
    let out = MarkdownRenderer::default()
        .render("# Install\n\n```rust\nfn main() {}\n```\n\n<script>alert(1)</script>")
        .unwrap();

    assert!(out.html.contains("id=\"install\""));
    assert!(out.html.contains("language-rust"));
    assert!(!out.html.contains("<script>"));
    assert_eq!(out.headings[0].title, "Install");
}

#[test]
fn markdown_renderer_suffixes_duplicate_headings_and_extracts_plain_text() {
    let out = MarkdownRenderer::default()
        .render("# Intro\n\nBody text.\n\n## Intro\n\nMore text.")
        .unwrap();

    assert_eq!(out.headings[0].id, "intro");
    assert_eq!(out.headings[1].id, "intro-2");
    assert!(out.plain_text.contains("Body text."));
    assert_eq!(out.excerpt, "Intro Body text. Intro More text.");
}

/// DRIVERS-008: a heading whose own text already reads like a suffixed
/// duplicate (`Overview 2`) must not take an id another heading was given,
/// in either order. Every `id` in the HTML and every `Heading.id` must be
/// unique, and the two must agree.
#[test]
fn heading_ids_stay_unique_when_a_title_looks_like_a_suffixed_duplicate() {
    for (markdown, expected) in [
        (
            "# Overview\n\n## Overview\n\n## Overview 2\n",
            ["overview", "overview-2", "overview-2-2"],
        ),
        (
            "# Overview 2\n\n## Overview\n\n## Overview\n",
            ["overview-2", "overview", "overview-3"],
        ),
    ] {
        let out = MarkdownRenderer::default().render(markdown).unwrap();
        let ids: Vec<&str> = out.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, expected, "heading ids for {markdown:?}");
        for id in expected {
            assert_eq!(
                out.html.matches(&format!("id=\"{id}\"")).count(),
                1,
                "the HTML carries `{id}` exactly once: {}",
                out.html
            );
        }
    }
}

/// The output of a renderer that calls none of the builder calls
/// (`html_input`, `allow_unsafe_links`, `autolink`), pinned from the
/// renderer as it was before those calls existed (comrak 0.52.0 and
/// ammonia 4.1.4). Raw HTML, an unsafe
/// link and a bare URL each keep today's treatment.
#[test]
fn a_renderer_without_builder_calls_renders_todays_output() {
    let markdown = "Hello <b>bold</b> and [link](javascript:alert(1)) at https://example.com\n\n<div onclick=\"x()\">block</div>\n\n# Title\n";

    let safe = MarkdownRenderer::default().render(markdown).unwrap();
    assert_eq!(
        safe.html,
        "<p>Hello bold and <a href=\"\" rel=\"noopener noreferrer\">link</a> at https://example.com</p>\n\n<h1 id=\"title\">Title</h1>"
    );

    let raw = MarkdownRenderer::new(suprnova::content::MarkdownOptions {
        unsafe_html: true,
        ..Default::default()
    })
    .render(markdown)
    .unwrap();
    assert_eq!(
        raw.html,
        "<p>Hello <b>bold</b> and <a href=\"javascript:alert(1)\">link</a> at https://example.com</p>\n<div onclick=\"x()\">block</div>\n<h1 id=\"title\">Title</h1>"
    );
}
