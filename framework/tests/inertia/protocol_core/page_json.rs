//! PAR-053: the page JSON the first visit writes into its `<script>`, and
//! how the page encodes its values.

use serde_json::{Value, json};
use suprnova::InertiaResponse;

use super::support::{MockReq, body_of, no_manifest, page_of};

#[tokio::test]
async fn inp_the_first_visit_script_escapes_angle_brackets() {
    // jsdom: a prop holding `<!--<script>` put the tokenizer into the
    // double-escaped script state, so the real `</script>` no longer closed
    // the element and the mount element became script text.
    let resp = InertiaResponse::new("Home")
        .with("note", "<!--<script>alert(1)</script>-->")
        .resolve(&MockReq::new("/"))
        .await
        .unwrap();
    let html = body_of(resp).await;
    let open = r#"<script type="application/json" data-page="app">"#;
    let start = html.find(open).expect("the page script") + open.len();
    let end = start + html[start..].find("</script>").expect("a closing tag");
    let script = &html[start..end];
    assert!(
        !script.contains('<') && !script.contains('>'),
        "no angle bracket may reach the script text; got {script}"
    );
    assert!(
        script.contains(r"\u003c!--\u003cscript\u003e"),
        "got {script}"
    );
    assert!(
        script.contains(r"\u003c\/script\u003e"),
        "`/` stays escaped too"
    );
    let page: Value = serde_json::from_str(script).expect("the script text is the page JSON");
    assert_eq!(page["props"]["note"], "<!--<script>alert(1)</script>-->");
    assert!(
        html[end..].starts_with("</script>\n<div id=\"app\"></div>"),
        "the mount element follows the script"
    );
}

// ---- big integers ----

/// Props and flash holding integers on both sides of JavaScript's safe
/// range, at several depths.
fn with_integers(response: InertiaResponse) -> InertiaResponse {
    response
        .with("id", 9_007_199_254_740_993_u64)
        .with("negative", -9_007_199_254_740_993_i64)
        .with("safe", 9_007_199_254_740_991_u64)
        .with("negative_safe", -9_007_199_254_740_991_i64)
        .with("ratio", 1.5)
        .with("largest", u64::MAX)
        .with(
            "nested",
            json!({"list": [9_007_199_254_740_993_u64, "x"], "deep": {"n": 12}}),
        )
        .flash("created", json!({"id": 9_007_199_254_740_993_u64}))
}

fn big(digits: &str) -> Value {
    json!({ "$bigint": digits })
}

#[tokio::test]
async fn inp_preserve_big_integers_wraps_every_unsafe_integer_in_props_and_flash() {
    let config = no_manifest().preserve_big_integers(true);
    let resp = with_integers(InertiaResponse::new("Ids").with_config(config))
        .resolve(&MockReq::new("/").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;
    let props = &page["props"];
    assert_eq!(props["id"], big("9007199254740993"), "got {page}");
    assert_eq!(props["negative"], big("-9007199254740993"));
    assert_eq!(props["safe"], json!(9_007_199_254_740_991_u64));
    assert_eq!(props["negative_safe"], json!(-9_007_199_254_740_991_i64));
    assert_eq!(props["ratio"], json!(1.5), "a float is not an integer");
    assert_eq!(props["largest"], big("18446744073709551615"));
    assert_eq!(
        props["nested"],
        json!({"list": [big("9007199254740993"), "x"], "deep": {"n": 12}})
    );
    assert_eq!(page["flash"]["created"]["id"], big("9007199254740993"));
    assert_eq!(page["preserveBigIntegers"], true);
}

#[tokio::test]
async fn inp_the_first_visit_carries_the_big_integer_markers_too() {
    let resp = with_integers(
        InertiaResponse::new("Ids")
            .with_config(no_manifest())
            .preserve_big_integers(true),
    )
    .resolve(&MockReq::new("/"))
    .await
    .unwrap();
    let html = body_of(resp).await;
    assert!(
        html.contains(r#""id":{"$bigint":"9007199254740993"}"#),
        "got {html}"
    );
    assert!(html.contains(r#""preserveBigIntegers":true"#));
}

#[tokio::test]
async fn inp_without_preserve_big_integers_nothing_is_wrapped() {
    let resp = with_integers(InertiaResponse::new("Ids").with_config(no_manifest()))
        .resolve(&MockReq::new("/").inertia())
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["id"], json!(9_007_199_254_740_993_u64));
    assert_eq!(page["props"]["largest"], json!(u64::MAX));
    assert_eq!(
        page["flash"]["created"]["id"],
        json!(9_007_199_254_740_993_u64)
    );
    assert!(page.get("preserveBigIntegers").is_none(), "got {page}");
}

#[tokio::test]
async fn inp_the_response_setting_beats_the_config() {
    let off = with_integers(
        InertiaResponse::new("Ids")
            .with_config(no_manifest().preserve_big_integers(true))
            .preserve_big_integers(false),
    )
    .resolve(&MockReq::new("/").inertia())
    .await
    .unwrap();
    let off = page_of(off).await;
    assert_eq!(off["props"]["id"], json!(9_007_199_254_740_993_u64));
    assert!(off.get("preserveBigIntegers").is_none());

    let on = with_integers(
        InertiaResponse::new("Ids")
            .with_config(no_manifest())
            .preserve_big_integers(true),
    )
    .resolve(&MockReq::new("/").inertia())
    .await
    .unwrap();
    let on = page_of(on).await;
    assert_eq!(on["props"]["id"], big("9007199254740993"));
    assert_eq!(on["preserveBigIntegers"], true);
}
