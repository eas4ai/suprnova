//! The ninth parity round's pagination behaviour, observed by the
//! `par-laravel-gaps-data` mechanism.

use suprnova::LengthAwarePaginator;

#[test]
fn an_empty_result_reports_one_last_page() {
    let page = LengthAwarePaginator::<u8>::new(vec![], 0, 10, 1);
    let json = serde_json::to_value(&page).expect("serializes");
    assert_eq!(json["last_page"], 1, "Laravel reports one page for an empty result");
}
