//! Cache normalization must retain the protocol's literal comma entries.
use std::collections::HashMap;
use suprnova::{InertiaRequestExt, InertiaResponse};
struct Visit(HashMap<String, String>);
impl InertiaRequestExt for Visit {
    fn path(&self) -> &str {
        "/cache"
    }
    fn header(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}
#[tokio::test]
async fn cache_011_partial_lists_preserve_whitespace_and_drop_empty_segments() {
    let request = Visit(
        [
            ("X-Inertia", "1"),
            ("X-Inertia-Partial-Component", "Cache"),
            ("X-Inertia-Partial-Data", "a,, b,a"),
            ("X-Inertia-Partial-Except", "a"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into(),
    );
    let response = InertiaResponse::new("Cache")
        .with("a", 1)
        .with("b", 2)
        .with(" b", 3)
        .resolve(&request)
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(page["props"][" b"], 3);
    assert!(page["props"].get("b").is_none());
    assert!(page["props"].get("a").is_none());
}
