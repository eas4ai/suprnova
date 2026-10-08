//! PAR-046: the violating example for the `par-inertia-protocol`
//! mechanism's fail receipt. A header value PHP casts to true counts as an
//! Inertia visit, so `X-Inertia: 1` gets the JSON page, not the document.

use std::collections::HashMap;

use suprnova::{InertiaRequestExt, InertiaResponse};

struct Req {
    headers: HashMap<String, String>,
}

impl InertiaRequestExt for Req {
    fn path(&self) -> &str {
        "/home"
    }
    fn path_and_query(&self) -> String {
        "/home".to_string()
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

#[tokio::test]
async fn inp_a_truthy_x_inertia_header_counts_as_an_inertia_visit() {
    let req = Req {
        headers: HashMap::from([("X-Inertia".to_string(), "1".to_string())]),
    };
    let resp = InertiaResponse::new("Home")
        .with("count", 1u32)
        .resolve(&req)
        .await
        .expect("resolves");
    let hyper_resp = resp.into_hyper();
    assert_eq!(
        hyper_resp.headers().get("X-Inertia").map(|v| v.to_str().unwrap_or("")),
        Some("true"),
        "X-Inertia: 1 is an Inertia visit, so the response is the JSON page with X-Inertia: true"
    );
}
