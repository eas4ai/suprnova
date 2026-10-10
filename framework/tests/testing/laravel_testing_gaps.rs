//! The testing members of Laravel's surface that Suprnova lacked or read
//! differently (PAR-180): a `204` assertion that passes only for an empty
//! `204`, as Laravel's `assertNoContent` does.

use suprnova::testing::TestResponse;

#[test]
fn a_200_with_an_empty_body_is_not_no_content() {
    let response = TestResponse::new(200, Vec::<(String, String)>::new(), Vec::<u8>::new());

    assert_eq!(
        response.status(),
        204,
        "Laravel's assertNoContent fails a 200: only an empty 204 is no content"
    );
}
