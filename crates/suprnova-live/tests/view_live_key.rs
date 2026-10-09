//! The `live_key` filter admits exactly the keys the checker and the browser
//! runtime admit, and `live_key_digest` turns any value into such a key.

use askama::Template;
use sha2::{Digest as _, Sha256};
use suprnova_live::view::{LiveKeyErrorKind, check_live_key, live_key_digest};

mod filters {
    pub use suprnova_live::view::filters::{live_key, live_key_digest};
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

/// LIVE-033: the runtime's `SAFE_KEY` refuses a key that starts with `_`, `-`,
/// `.` or `:`, so the filter does too. The runtime's pattern is read from its
/// source, so a change on either side fails here.
#[test]
fn live_033_a_key_starts_with_a_letter_or_a_digit_as_the_runtime_requires() {
    let runtime = include_str!("../browser/src/morph/keys.ts");
    assert!(
        runtime.contains("const SAFE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;"),
        "the runtime's key pattern changed; update check_live_key with it"
    );
    for refused in ["-1", "_draft", ".x", ":flash"] {
        assert_eq!(
            check_live_key(refused).unwrap_err().kind(),
            LiveKeyErrorKind::ForbiddenByte,
            "{refused}"
        );
    }
    for accepted in ["1-a", "a_", "Z.:-_9", "0"] {
        assert!(check_live_key(accepted).is_ok(), "{accepted}");
    }
    let error = KeyedList { items: &["_draft"] }
        .render()
        .expect_err("a leading underscore is refused");
    assert!(
        error.to_string().contains("live_key_forbidden_byte"),
        "{error}"
    );
}

#[derive(Template)]
#[template(
    source = "{% for item in items %}<li live:key=\"{{ item|live_key_digest }}\">{{ item }}</li>{% endfor %}",
    ext = "html"
)]
struct DigestKeyedList<'a> {
    items: &'a [&'a str],
}

/// LIVE-035: any value, an email address, a space, an empty string, or text
/// outside ASCII, renders a stable key the checker and the runtime accept.
#[test]
fn live_035_the_digest_filter_keys_any_value_with_a_stable_accepted_key() {
    let values = ["ada@example.com", "row 2", "", "Zoë", "_draft", "a\"b<c>"];
    let rendered = DigestKeyedList { items: &values }
        .render()
        .expect("every value renders");
    for value in values {
        let key = live_key_digest(value);
        assert_eq!(key.len(), 33, "{key}");
        assert!(key.starts_with('k'), "{key}");
        assert!(check_live_key(&key).is_ok(), "{key}");
        assert_eq!(key, live_key_digest(value), "one value, one key");
        assert!(
            rendered.contains(&format!("live:key=\"{key}\"")),
            "{rendered}"
        );
    }
    let expected: String = Sha256::digest("ada@example.com".as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(live_key_digest("ada@example.com"), format!("k{expected}"));
    assert_ne!(live_key_digest("row 2"), live_key_digest("row 3"));
}
