#![cfg(feature = "testing")]

//! Public Suprnova authentication facade tests backed by Magnetar.

#[path = "common/magnetar_auth.rs"]
mod magnetar_auth;

use serial_test::serial;
use suprnova::Auth;

static SETUP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn setup() {
    SETUP
        .get_or_init(|| async { magnetar_auth::install().await })
        .await;
}

#[tokio::test]
#[serial]
async fn password_register_and_authenticate_round_trip() {
    setup().await;
    let user = Auth::password()
        .register("parity@example.test", "correct-password")
        .await
        .expect("register through Magnetar");
    let (authenticated, session) = Auth::password()
        .authenticate(
            "PARITY@example.test",
            "correct-password",
            Some("integration-test".to_owned()),
            Some("127.0.0.1".to_owned()),
        )
        .await
        .expect("authenticate through Magnetar");
    assert_eq!(authenticated.id, user.id);
    assert!(session.token.is_some());
}

#[tokio::test]
#[serial]
async fn wrong_password_fails_authentication() {
    setup().await;
    Auth::password()
        .register("wrong-password@example.test", "correct-password")
        .await
        .expect("register user");
    let error = Auth::password()
        .authenticate(
            "wrong-password@example.test",
            "incorrect-password",
            None,
            None,
        )
        .await
        .expect_err("incorrect password must fail");
    assert_eq!(error.status_code(), 401);
}

#[tokio::test]
#[serial]
async fn magic_link_is_single_use_and_issues_a_session() {
    setup().await;
    let token = Auth::magic_link()
        .send(
            "magic-parity@example.test",
            "https://example.test/auth/magic",
        )
        .await
        .expect("mint magic link");
    let (user, session) = Auth::magic_link()
        .consume(&token)
        .await
        .expect("consume magic link");
    assert_eq!(user.email, "magic-parity@example.test");
    assert!(session.token.is_some());
    assert!(Auth::magic_link().consume(&token).await.is_err());
}

#[tokio::test]
#[serial]
async fn direct_user_lookup_uses_the_installed_engine() {
    setup().await;
    let user = Auth::password()
        .register("lookup@example.test", "lookup-password")
        .await
        .expect("register user");
    let found = suprnova::magnetar_integration::find_user_by_id(user.id.as_str())
        .await
        .expect("lookup succeeds")
        .expect("user exists");
    assert_eq!(found.id, user.id);
}
