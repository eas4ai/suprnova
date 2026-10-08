//! PAR-053: the page JSON the first visit writes into its `<script>`, and
//! how the page encodes its values.

use serde_json::Value;
use suprnova::InertiaResponse;

use super::support::{MockReq, body_of};

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
