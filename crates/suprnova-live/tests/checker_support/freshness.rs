//! The template source of one freshness combination of the directive grammar.
//!
//! The positive and the negative checker suite read the same
//! `freshness_combinations` of `directive-grammar.json`. One mapping from a
//! combination to its template keeps the two suites on the same grammar.

use serde_json::Value;

/// Returns the `<section>` that declares the stream mode and the poll flag
/// of `combination`.
pub(crate) fn freshness_source(combination: &Value) -> String {
    let poll = combination["poll"].as_bool().expect("poll flag");
    let stream = combination["stream"].as_str().expect("stream mode");
    let stream_attribute = match stream {
        "absent" => "",
        "default" => r#" live:stream="orders""#,
        "hybrid" => r#" live:stream.hybrid="orders""#,
        "push-only" => r#" live:stream.push-only="orders""#,
        other => panic!("unexpected stream mode {other}"),
    };
    let poll_attribute = if poll { " live:poll" } else { "" };
    format!("<section{stream_attribute}{poll_attribute}></section>")
}
