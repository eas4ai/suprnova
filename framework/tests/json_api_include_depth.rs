//! The `?include=` depth cap. On a cyclic relationship graph,
//! `?include=author.posts.author.posts…` is attacker-controlled fan-out
//! bounded only by the query string's length, so `IncludeTree` truncates
//! every path to `resources::max_relationship_depth` while it parses.
//!
//! The cap is process-global, so every test here is `#[serial]` **and**
//! sets the depth it expects on its first line: serial gives mutual
//! exclusion, not ordering, and a panicking test would otherwise leave a
//! stale cap behind for whichever test ran next.

use serde_json::Value;
use suprnova::{
    DEFAULT_MAX_RELATIONSHIP_DEPTH, Data, RequestIncludeSet, Resource, Validate,
    current_max_relationship_depth, max_relationship_depth, scope_include_set,
};

#[derive(Debug, Clone, Data, Validate)]
#[json_resource("authors")]
pub struct AuthorResource {
    pub id: i64,
    pub name: String,

    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}

#[derive(Debug, Clone, Data, Validate)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,
}

/// Build `post -> author -> post -> author -> …`, `links` author/post
/// pairs deep. Every resource gets a distinct id so `IncludedSink`'s
/// `(type, id)` dedup can never hide a level from the assertions.
fn nested_post(links: i64) -> PostResource {
    let mut post = PostResource {
        id: links,
        title: format!("post {links}"),
        author: None,
    };
    for step in (1..=links).rev() {
        post = PostResource {
            id: step - 1,
            title: format!("post {}", step - 1),
            author: Some(AuthorResource {
                id: step,
                name: format!("author {step}"),
                posts: vec![post],
            }),
        };
    }
    post
}

/// Every `(type, id)` pair in the rendered `included` array.
fn included_pairs(body: &Value) -> Vec<(String, String)> {
    body["included"]
        .as_array()
        .expect("included array")
        .iter()
        .map(|v| {
            (
                v["type"].as_str().unwrap_or_default().to_string(),
                v["id"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
#[serial_test::serial]
async fn a_cyclic_include_deeper_than_the_default_is_cut_at_five() {
    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);
    assert_eq!(current_max_relationship_depth(), 5);

    // Six segments over a cyclic posts <-> authors graph.
    let post = nested_post(3);
    let set = RequestIncludeSet::from_query("include=author.posts.author.posts.author.posts");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(post).render().await.expect("render");
        assert_eq!(
            resp.status_code(),
            200,
            "a truncated path is served, not rejected"
        );
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;

    let pairs = included_pairs(&body);
    assert_eq!(
        pairs.len(),
        5,
        "six segments must be cut to five: {pairs:?}"
    );
    assert!(
        pairs.contains(&("authors".to_string(), "3".to_string())),
        "the fifth segment must still be included: {pairs:?}"
    );
    assert!(
        !pairs.contains(&("posts".to_string(), "3".to_string())),
        "the sixth segment must be gone: {pairs:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_explicit_cap_of_two_cuts_at_two() {
    max_relationship_depth(2);
    assert_eq!(current_max_relationship_depth(), 2);

    let post = nested_post(3);
    let set = RequestIncludeSet::from_query("include=author.posts.author.posts.author.posts");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(post).render().await.expect("render");
        assert_eq!(resp.status_code(), 200);
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;

    let pairs = included_pairs(&body);
    assert_eq!(
        pairs,
        vec![
            ("authors".to_string(), "1".to_string()),
            ("posts".to_string(), "1".to_string()),
        ],
        "an explicit cap of 2 keeps exactly the first two segments"
    );

    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);
}

#[tokio::test]
#[serial_test::serial]
async fn truncation_never_widens_the_allowlist() {
    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);

    // `secrets` is on nobody's allowlist, and it sits at segment 3 -
    // inside the cap - so the request is still rejected with the full
    // dotted path.
    let post = nested_post(3);
    let set = RequestIncludeSet::from_query("include=author.posts.secrets");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(post).render().await.expect("render");
        assert_eq!(
            resp.status_code(),
            400,
            "an unknown include inside the cap is still a 400"
        );
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;

    let detail = body["errors"][0]["detail"].as_str().expect("detail string");
    assert!(
        detail.contains("author.posts.secrets"),
        "the full rejected path must survive: {detail}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_segment_past_the_cap_is_dropped_before_it_is_validated() {
    max_relationship_depth(2);

    // Same unknown segment as above, now the third of three: the cap
    // removes it before the allowlist ever sees it, so the request
    // succeeds with the two segments that survived. Nothing outside the
    // allowlist is emitted either way - the client just gets less than it
    // asked for, which is what Laravel does too.
    let post = nested_post(3);
    let set = RequestIncludeSet::from_query("include=author.posts.secrets");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(post).render().await.expect("render");
        assert_eq!(resp.status_code(), 200);
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;

    assert_eq!(included_pairs(&body).len(), 2);

    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);
}

#[tokio::test]
#[serial_test::serial]
async fn a_cap_of_zero_turns_every_include_off() {
    max_relationship_depth(0);
    assert_eq!(current_max_relationship_depth(), 0);

    let post = nested_post(1);
    let set = RequestIncludeSet::from_query("include=author");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(post).render().await.expect("render");
        assert_eq!(resp.status_code(), 200);
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;

    assert!(
        body.get("included").is_none(),
        "depth 0 leaves nothing to include: {body}"
    );

    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);
}

#[tokio::test]
#[serial_test::serial]
async fn the_most_recent_setter_call_wins() {
    max_relationship_depth(1);
    assert_eq!(current_max_relationship_depth(), 1);

    let set = RequestIncludeSet::from_query("include=author.posts.author.posts.author.posts");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(nested_post(3))
            .render()
            .await
            .expect("render");
        assert_eq!(resp.status_code(), 200);
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;
    assert_eq!(
        included_pairs(&body).len(),
        1,
        "the first call sets the cap"
    );

    // A second call replaces the first rather than accumulating with it.
    max_relationship_depth(3);
    assert_eq!(current_max_relationship_depth(), 3);

    let set = RequestIncludeSet::from_query("include=author.posts.author.posts.author.posts");
    let body = scope_include_set(set, async move {
        let resp = Resource::single(nested_post(3))
            .render()
            .await
            .expect("render");
        assert_eq!(resp.status_code(), 200);
        serde_json::from_slice::<Value>(resp.body()).expect("json body")
    })
    .await;
    assert_eq!(
        included_pairs(&body).len(),
        3,
        "the second call replaces the first"
    );

    max_relationship_depth(DEFAULT_MAX_RELATIONSHIP_DEPTH);
}
