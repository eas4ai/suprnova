//! The `live_key` filter admits exactly the keys the checker admits.

use askama::Template;
use suprnova_live::view::{LiveKeyErrorKind, check_live_key};

mod filters {
    pub use suprnova_live::view::filters::live_key;
}

#[derive(Template)]
#[template(
    source = "{% for item in items %}<li live:key=\"{{ item|live_key }}\">{{ item }}</li>{% endfor %}",
    ext = "html"
)]
struct KeyedList<'a> {
    items: &'a [&'a str],
}

#[test]
fn well_formed_keys_render_unchanged() {
    let rendered = KeyedList {
        items: &["row-1", "row_2.b:c", "ROW3"],
    }
    .render()
    .expect("render");
    assert_eq!(
        rendered,
        "<li live:key=\"row-1\">row-1</li><li live:key=\"row_2.b:c\">row_2.b:c</li><li live:key=\"ROW3\">ROW3</li>"
    );
}

#[test]
fn a_key_with_a_forbidden_byte_fails_the_render() {
    let error = KeyedList {
        items: &["row-1", "row 2"],
    }
    .render()
    .expect_err("space is not a key byte");
    assert!(
        error.to_string().contains("live_key_forbidden_byte"),
        "{error}"
    );
}

#[test]
fn an_empty_key_fails_the_render() {
    let error = KeyedList { items: &[""] }.render().expect_err("empty key");
    assert!(error.to_string().contains("empty_live_key"), "{error}");
}

#[test]
fn the_rule_matches_the_checker_bounds() {
    assert_eq!(
        check_live_key("").unwrap_err().kind(),
        LiveKeyErrorKind::Empty
    );
    let long = "k".repeat(129);
    assert_eq!(
        check_live_key(&long).unwrap_err().kind(),
        LiveKeyErrorKind::TooLong
    );
    assert_eq!(
        check_live_key("a/b").unwrap_err().kind(),
        LiveKeyErrorKind::ForbiddenByte
    );
    assert!(check_live_key(&"k".repeat(128)).is_ok());
}
