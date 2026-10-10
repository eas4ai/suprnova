//! The CSRF field and the fallible token helpers (PAR-163): the hidden
//! input carries `autocomplete="off"` so a browser never restores a stale
//! token into it, and the `try_*` siblings name the missing session where
//! Laravel's `csrf_token` throws.

use suprnova::session::{new_session_slot_for_test, session_scope_for_test};
use suprnova::{csrf_field, csrf_token, try_csrf_field, try_csrf_token};

#[tokio::test]
async fn csrf_field_inside_a_session_carries_the_token_and_autocomplete_off() {
    let slot = new_session_slot_for_test();
    session_scope_for_test(slot, async move {
        let token = csrf_token().expect("a session scope holds a token");
        let field = csrf_field();
        assert_eq!(
            field,
            format!(r#"<input type="hidden" name="_token" value="{token}" autocomplete="off">"#)
        );
        assert_eq!(try_csrf_field().expect("a session is installed"), field);
        assert_eq!(try_csrf_token().expect("a session is installed"), token);
    })
    .await;
}

#[tokio::test]
async fn csrf_field_and_token_keep_their_answers_outside_a_session() {
    assert_eq!(csrf_field(), "");
    assert!(csrf_token().is_none());
}

#[tokio::test]
async fn try_csrf_helpers_outside_a_session_name_the_missing_session() {
    let token_err = try_csrf_token().expect_err("no session scope is installed");
    assert!(
        token_err.message().contains("session"),
        "the error names the missing session: {token_err}"
    );
    let field_err = try_csrf_field().expect_err("no session scope is installed");
    assert!(
        field_err.message().contains("session"),
        "the error names the missing session: {field_err}"
    );
}
