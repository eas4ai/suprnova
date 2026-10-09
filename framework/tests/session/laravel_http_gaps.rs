//! The tenth parity round's session behaviour, observed by the
//! `par-laravel-gaps-http` mechanism.

use suprnova::session::{
    get_csrf_token, new_session_slot_for_test, regenerate_session_id, session_scope_for_test,
};

#[tokio::test]
async fn regenerating_the_session_issues_a_new_csrf_token() {
    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async {
        let before = get_csrf_token().expect("a session is in scope");
        regenerate_session_id();
        let after = get_csrf_token().expect("a session is in scope");
        assert_ne!(
            before, after,
            "Laravel's Session::regenerate issues a new CSRF token"
        );
    })
    .await;
}
