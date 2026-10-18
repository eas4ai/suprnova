//! Shared readers keep island and shell assertions consistent without
//! importing database fixtures into tests that only inspect Live markup.

use base64::Engine as _;

/// Finds one island's opening tag so assertions exclude the surrounding shell.
#[must_use]
pub fn island_tag<'h>(html: &'h str, key: &str) -> &'h str {
    let needle = format!("data-suprnova-live-document-key=\"{key}\"");
    let position = html
        .find(&needle)
        .unwrap_or_else(|| panic!("no island with document key {key}"));
    let start = html[..position].rfind('<').expect("island tag start");
    let end = html[position..].find('>').expect("island tag end") + position + 1;
    &html[start..end]
}

/// Reads an emitted attribute so assertions compare its exact markup value.
#[must_use]
pub fn attribute<'h>(tag: &'h str, name: &str) -> &'h str {
    let prefix = format!("{name}=\"");
    let start = tag
        .find(&prefix)
        .map(|index| index + prefix.len())
        .unwrap_or_else(|| panic!("missing attribute {name} in {tag}"));
    let tail = &tag[start..];
    let end = tail.find('"').expect("unterminated attribute");
    &tail[..end]
}

/// Decodes the signed envelope so tests can inspect the island's mounted scope.
/// Its `body.scope` value distinguishes the authority of different requests.
#[must_use]
pub fn decoded_snapshot(tag: &str) -> serde_json::Value {
    let encoded = attribute(tag, "data-suprnova-live-snapshot");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("decode emitted Live snapshot");
    serde_json::from_slice(&bytes).expect("parse emitted Live snapshot")
}
