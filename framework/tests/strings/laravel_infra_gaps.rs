//! Laravel infrastructure gaps owned by the strings suite: inline and
//! whole-document Markdown through `Str` and the renderer's builder
//! calls, and the fluent `Stringable` with purpose-bound encryption.

use std::sync::Once;

use suprnova::content::{HtmlInput, MarkdownRenderer};
use suprnova::{Crypt, CryptPurpose, EncryptionKey, Str, Stringable};

fn crypt() {
    static INIT: Once = Once::new();
    INIT.call_once(|| Crypt::init(EncryptionKey::generate()));
}

// --- Inline Markdown -------------------------------------------------------

#[test]
fn inline_markdown_converts_inline_syntax_with_no_block_wrapper() {
    let html = Str::inline_markdown("**Laravel**", &MarkdownRenderer::default()).unwrap();
    assert_eq!(html, "<strong>Laravel</strong>");
    assert!(!html.contains("<p"), "no paragraph wrapper: {html}");
}

#[test]
fn inline_markdown_keeps_emphasis_code_and_links() {
    let html = Str::inline_markdown(
        "_Taylor_ wrote `Str` and [the docs](https://laravel.com)",
        &MarkdownRenderer::default(),
    )
    .unwrap();
    assert!(
        html.starts_with("<em>Taylor</em> wrote <code>Str</code>"),
        "{html}"
    );
    assert!(html.contains("href=\"https://laravel.com\""), "{html}");
    assert!(html.contains(">the docs</a>"), "{html}");
}

#[test]
fn inline_markdown_keeps_block_markers_as_text() {
    let renderer = MarkdownRenderer::default();
    assert_eq!(
        Str::inline_markdown("# Title", &renderer).unwrap(),
        "# Title"
    );
    for (markdown, block_tag) in [
        ("# Title", "<h1"),
        ("## Title", "<h2"),
        ("- item", "<ul"),
        ("* item", "<ul"),
        ("+ item", "<ul"),
        ("1. first", "<ol"),
        ("> quoted", "<blockquote"),
        ("```\ncode\n```", "<pre"),
        ("    indented", "<pre"),
        ("---", "<hr"),
        ("Title\n=====", "<h1"),
        ("a | b\n--- | ---\n1 | 2", "<table"),
    ] {
        let html = Str::inline_markdown(markdown, &renderer).unwrap();
        assert!(
            !html.contains(block_tag) && !html.contains("<p"),
            "{markdown:?} rendered a block: {html}"
        );
    }
    assert_eq!(Str::inline_markdown("- item", &renderer).unwrap(), "- item");
    assert_eq!(
        Str::inline_markdown("> quoted", &renderer).unwrap(),
        "&gt; quoted"
    );
    assert_eq!(
        Str::inline_markdown("1. first", &renderer).unwrap(),
        "1. first"
    );
}

#[test]
fn inline_markdown_keeps_lines_and_blank_lines() {
    let html = Str::inline_markdown("one *two*\n\nthree", &MarkdownRenderer::default()).unwrap();
    assert_eq!(html, "one <em>two</em>\n\nthree");
}

#[test]
fn inline_markdown_of_empty_text_is_empty() {
    assert_eq!(
        Str::inline_markdown("", &MarkdownRenderer::default()).unwrap(),
        ""
    );
}

#[test]
fn render_inline_is_what_str_inline_markdown_answers() {
    let renderer = MarkdownRenderer::default();
    assert_eq!(
        renderer.render_inline("*a* and **b**").unwrap(),
        Str::inline_markdown("*a* and **b**", &renderer).unwrap()
    );
}

#[test]
fn inline_markdown_keeps_a_character_the_renderer_marks_lines_with() {
    // Rare spaces in the text must come through untouched.
    let text = "a\u{2000}b\u{3000}c\u{205F}d";
    let html = Str::inline_markdown(text, &MarkdownRenderer::default()).unwrap();
    assert_eq!(html, text);
}

#[test]
fn text_holding_every_line_marker_renders_as_escaped_text() {
    let markers = "\u{3000}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200A}\u{202F}\u{205F}\u{1680}";
    let text = format!("**bold** <b>x</b> {markers}");
    let html = Str::inline_markdown(&text, &MarkdownRenderer::default()).unwrap();
    assert_eq!(html, format!("**bold** &lt;b&gt;x&lt;/b&gt; {markers}"));
}

#[test]
fn the_default_renderer_lets_no_script_through() {
    let renderer = MarkdownRenderer::default();
    let inline = Str::inline_markdown("Inject: <script>alert(1)</script>", &renderer).unwrap();
    assert!(!inline.contains("<script"), "{inline}");
    let document = Str::markdown("<script>alert(1)</script>\n\nText", &renderer).unwrap();
    assert!(!document.contains("<script"), "{document}");
}

// --- Whole documents -------------------------------------------------------

#[test]
fn markdown_renders_a_whole_document_as_a_string() {
    let renderer = MarkdownRenderer::default();
    let html = Str::markdown("# Laravel\n\nA *framework*.", &renderer).unwrap();
    assert!(html.contains("<h1 id=\"laravel\">Laravel</h1>"), "{html}");
    assert!(html.contains("<p>A <em>framework</em>.</p>"), "{html}");
    assert_eq!(
        html,
        renderer.render("# Laravel\n\nA *framework*.").unwrap().html
    );
}

// --- The builder calls -----------------------------------------------------

#[test]
fn html_input_strip_removes_raw_html_and_keeps_its_text() {
    let renderer = MarkdownRenderer::default().html_input(HtmlInput::Strip);
    assert_eq!(
        Str::inline_markdown("Inject: <script>alert(\"Hello XSS!\");</script>", &renderer).unwrap(),
        "Inject: alert(&quot;Hello XSS!&quot;);"
    );
    let html = Str::markdown("# Taylor <b>Otwell</b>", &renderer).unwrap();
    assert!(html.contains(">Taylor Otwell</h1>"), "{html}");
    let block = Str::markdown("<div>gone</div>\n\nkept", &renderer).unwrap();
    assert!(
        !block.contains("gone") && block.contains("<p>kept</p>"),
        "{block}"
    );
}

#[test]
fn html_input_escape_shows_raw_html_as_text() {
    let renderer = MarkdownRenderer::default().html_input(HtmlInput::Escape);
    assert_eq!(
        Str::inline_markdown("a <b>bold</b> move", &renderer).unwrap(),
        "a &lt;b&gt;bold&lt;/b&gt; move"
    );
}

#[test]
fn html_input_allow_passes_raw_html_through() {
    let renderer = MarkdownRenderer::default().html_input(HtmlInput::Allow);
    assert_eq!(
        Str::inline_markdown("a <b onclick=\"x()\">bold</b> move", &renderer).unwrap(),
        "a <b onclick=\"x()\">bold</b> move"
    );
}

#[test]
fn html_input_sanitize_keeps_safe_raw_html_and_drops_the_rest() {
    let renderer = MarkdownRenderer::default().html_input(HtmlInput::Sanitize);
    let html = Str::inline_markdown(
        "a <b onclick=\"x()\">bold</b> move<script>alert(1)</script>",
        &renderer,
    )
    .unwrap();
    assert!(html.contains("<b>bold</b>"), "{html}");
    assert!(
        !html.contains("onclick") && !html.contains("<script"),
        "{html}"
    );
}

#[test]
fn unsafe_links_are_refused_unless_allowed() {
    let markdown = "[x](javascript:alert(1))";
    let strip = MarkdownRenderer::default().html_input(HtmlInput::Strip);
    let refused = Str::inline_markdown(markdown, &strip).unwrap();
    assert!(!refused.contains("javascript:"), "{refused}");

    let allowed = Str::inline_markdown(markdown, &strip.clone().allow_unsafe_links(true)).unwrap();
    assert!(
        allowed.contains("href=\"javascript:alert(1)\""),
        "{allowed}"
    );

    let raw = MarkdownRenderer::default()
        .html_input(HtmlInput::Allow)
        .allow_unsafe_links(false);
    let refused_with_raw_html = Str::inline_markdown(markdown, &raw).unwrap();
    assert!(
        !refused_with_raw_html.contains("javascript:"),
        "{refused_with_raw_html}"
    );

    let default = Str::markdown(markdown, &MarkdownRenderer::default()).unwrap();
    assert!(!default.contains("javascript:"), "{default}");
}

#[test]
fn autolink_turns_bare_urls_into_links_only_when_on() {
    let text = "Visit https://example.com today";
    let off = Str::inline_markdown(text, &MarkdownRenderer::default()).unwrap();
    assert!(!off.contains("<a"), "{off}");
    let on = Str::inline_markdown(text, &MarkdownRenderer::default().autolink(true)).unwrap();
    assert!(on.contains("href=\"https://example.com\""), "{on}");
}

// --- Stringable ------------------------------------------------------------

#[test]
fn stringable_chains_every_str_helper() {
    assert_eq!(
        Str::of("Hello World").slug("-").to_string(),
        Str::slug("Hello World", "-")
    );
    assert_eq!(
        Str::of("Ärger").slug_in("-", "de").to_string(),
        Str::slug_in("Ärger", "-", "de")
    );
    assert_eq!(
        Str::of("taylor@example.com").mask('*', 3, None).to_string(),
        Str::mask("taylor@example.com", '*', 3, None)
    );
    assert_eq!(
        Str::of("The quick brown fox").limit(9, "...").to_string(),
        Str::limit("The quick brown fox", 9, "...")
    );
    assert_eq!(
        Str::of("The quick brown fox").words(2, ">>>").to_string(),
        Str::words("The quick brown fox", 2, ">>>")
    );
    assert_eq!(
        Str::of("The quick brown fox")
            .limit_words(12, "...")
            .to_string(),
        Str::limit_words("The quick brown fox", 12, "...")
    );
    assert_eq!(
        Str::of("This is my name")
            .excerpt("my", 3, "...")
            .map(String::from),
        Str::excerpt("This is my name", "my", 3, "...")
    );
    assert_eq!(Str::of("This is my name").excerpt("absent", 3, "..."), None);
    assert_eq!(
        Str::of("child").plural(2).to_string(),
        Str::plural("child", 2)
    );
    assert_eq!(
        Str::of("car").plural_with_count(3).to_string(),
        Str::plural_with_count("car", 3)
    );
    assert_eq!(
        Str::of("VerifiedHuman").plural_studly(2).to_string(),
        Str::plural_studly("VerifiedHuman", 2)
    );
    assert_eq!(
        Str::of("VerifiedHuman").plural_pascal(2).to_string(),
        Str::plural_pascal("VerifiedHuman", 2)
    );
    assert_eq!(
        Str::of("children").singular().to_string(),
        Str::singular("children")
    );
}

#[test]
fn stringable_chains_helpers_one_after_another() {
    let chained = Str::of("Child").plural(2).slug("-");
    assert_eq!(chained.as_str(), "children");
}

#[test]
fn stringable_converts_to_and_from_string_and_displays_its_value() {
    let from_string: Stringable = String::from("value").into();
    let from_str: Stringable = "value".into();
    assert_eq!(from_string, from_str);
    assert_eq!(format!("{from_str}"), "value");
    let back: String = from_str.into();
    assert_eq!(back, "value");
    assert_eq!(Str::of("value").into_string(), "value");
    assert_eq!(Str::of("value"), "value");
}

#[test]
fn stringable_renders_markdown() {
    let renderer = MarkdownRenderer::default();
    assert_eq!(
        Str::of("**Laravel**")
            .inline_markdown(&renderer)
            .unwrap()
            .as_str(),
        "<strong>Laravel</strong>"
    );
    assert_eq!(
        Str::of("# Laravel").markdown(&renderer).unwrap().as_str(),
        Str::markdown("# Laravel", &renderer).unwrap()
    );
}

#[test]
fn stringable_encrypts_and_decrypts_under_one_purpose() {
    crypt();
    let sealed = Str::of("ssn-1").encrypt(CryptPurpose::Cast).unwrap();
    assert_ne!(sealed.as_str(), "ssn-1");
    let opened = sealed.clone().decrypt(CryptPurpose::Cast).unwrap();
    assert_eq!(opened.as_str(), "ssn-1");
    assert_eq!(
        Crypt::decrypt_string(CryptPurpose::Cast, sealed.as_str()).unwrap(),
        "ssn-1",
        "the Stringable wraps Crypt's purpose-bound string"
    );
}

#[test]
fn stringable_refuses_another_purpose_and_the_cookie_purpose() {
    crypt();
    let sealed = Str::of("ssn-1").encrypt(CryptPurpose::Cast).unwrap();
    assert!(sealed.decrypt(CryptPurpose::Cursor).is_err());
    assert!(Str::of("ssn-1").encrypt(CryptPurpose::Cookie).is_err());
}

#[test]
fn stringable_encrypts_and_decrypts_with_a_context() {
    crypt();
    let sealed = Str::of("token")
        .encrypt_for(CryptPurpose::Cast, "users.secret")
        .unwrap();
    assert_eq!(
        sealed
            .clone()
            .decrypt_for(CryptPurpose::Cast, "users.secret")
            .unwrap()
            .as_str(),
        "token"
    );
    assert!(
        sealed
            .decrypt_for(CryptPurpose::Cast, "users.other")
            .is_err()
    );
}
