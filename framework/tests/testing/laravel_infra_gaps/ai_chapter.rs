//! The manual's chapter on AI-assisted development: linked from the table
//! of contents, and covering the Markdown pages, `llms.txt`, the language
//! server, pointing an assistant at a project, and the Boost divergence.

use std::path::Path;

fn manual(file: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../manual");
    std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("read manual/{file}: {e}"))
}

#[test]
fn the_table_of_contents_links_the_chapter() {
    assert!(
        manual("documentation.md")
            .contains("[AI-Assisted Development](ai-assisted-development.md)")
    );
}

#[test]
fn the_chapter_covers_each_topic_in_order() {
    let chapter = manual("ai-assisted-development.md");
    let mut lines = chapter.lines();
    for heading in [
        "# AI-Assisted Development",
        "## Read the manual as Markdown",
        "## Install the language server",
        "## Point an assistant at your project",
        "### Why Suprnova diverges",
        "## Next",
    ] {
        assert!(
            lines.any(|line| line == heading),
            "the chapter lacks {heading} in the required order"
        );
    }
}

#[test]
fn the_chapter_names_the_markdown_forms_the_language_server_and_boost() {
    let chapter = manual("ai-assisted-development.md");
    for needle in [
        "https://suprnova.app/docs/routing.md",
        "https://suprnova.app/llms.txt",
        "https://suprnova.app/llms-full.txt",
        "Suprnova LSP",
        "language server",
        "Laravel Boost",
    ] {
        assert!(
            chapter.contains(needle),
            "the chapter does not name {needle}"
        );
    }
    let divergence = chapter
        .split("### Why Suprnova diverges")
        .nth(1)
        .expect("the chapter has a divergence callout");
    assert!(
        divergence.contains("Boost"),
        "the divergence callout does not say what replaces Boost"
    );
}
