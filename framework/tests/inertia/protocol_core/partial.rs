//! PAR-047: partial reloads read `X-Inertia-Partial-Data` and
//! `X-Inertia-Partial-Except` as Laravel's `PropsResolver` does.

use suprnova::InertiaResponse;

use super::support::{MockReq, page_of};

#[tokio::test]
async fn inp_partial_data_drops_empty_segments_and_does_not_trim() {
    // `explode(',')` then `array_filter`: "a,, b" names "a" and " b".
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "a,, b");
    let resp = InertiaResponse::new("P")
        .with("a", 1)
        .with("b", 2)
        .with(" b", 3)
        .resolve(&req)
        .await
        .unwrap();
    let props = page_of(resp).await["props"].clone();
    let props = props.as_object().unwrap();
    assert!(
        props.contains_key("a"),
        "the empty segment must not drop `a`"
    );
    assert!(
        props.contains_key(" b"),
        "the entry is ` b`, untrimmed; got {props:?}"
    );
    assert!(
        !props.contains_key("b"),
        "a trimmed `b` must not be selected; got {props:?}"
    );
}

#[tokio::test]
async fn inp_an_empty_partial_data_header_counts_as_absent() {
    // Laravel's `parseHeader` returns null for an empty header, so the
    // reload is filtered by `except` alone. Suprnova dropped every prop.
    for empty in ["", ",,"] {
        let req = MockReq::new("/p")
            .partial("P")
            .header("X-Inertia-Partial-Data", empty)
            .header("X-Inertia-Partial-Except", empty);
        let resp = InertiaResponse::new("P")
            .with("a", 1)
            .with("b", 2)
            .resolve(&req)
            .await
            .unwrap();
        let page = page_of(resp).await;
        assert_eq!(page["props"]["a"], 1, "header {empty:?}: got {page}");
        assert_eq!(page["props"]["b"], 2, "header {empty:?}: got {page}");
    }
}
