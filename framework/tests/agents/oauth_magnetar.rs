//! AGENT-005 where Magnetar owns the default guard's sessions: the session
//! handed to the authorization server is the user's Magnetar session, and a
//! grant ends with Magnetar's epoch, here moved by a real password reset.
//!
//! Its own process: it installs the process-wide Magnetar engine, which the
//! tests of this binary that run without one must not see.

use sea_orm::Database;
use serde_json::json;
use suprnova::agents::{AgentsConfig, OAuthServer};
use suprnova::auth_flows::PasswordReset;
use suprnova::testing::TestClient;
use suprnova::{Auth, MagnetarConfig, Mail, init_magnetar};

use crate::oauth::{
    connect, container, refresh, refusal_reason, registry, remember_account, router,
};

const PASSWORD: &str = "correct-horse-battery";

#[tokio::test]
async fn agent_005_password_change_ends_the_token_under_magnetar_sessions() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_005_password_change_ends_the_token_under_magnetar_sessions",
    )
    .await
    {
        return;
    }
    let _env = crate::env_lock::lock_env_async().await;
    // SAFETY: this test runs alone in its own process and holds the
    // environment lock; nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.test");
    }
    let _container = container();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    init_magnetar(MagnetarConfig::from_sea_orm(db.clone()))
        .await
        .expect("install the default Magnetar engine and its tables");
    let email = "magnetar-grant@example.test";
    let user = Auth::password()
        .register(email, PASSWORD)
        .await
        .expect("register a Magnetar user")
        .created()
        .expect("a new account");
    let user_id = user.id.to_string();
    remember_account(&user_id);

    // The registry is built after the engine is installed, as an
    // application's boot builds it.
    let client = TestClient::new(
        router(AgentsConfig::new().oauth_server(OAuthServer::new().connection(db.clone()))),
        registry(),
    );
    let tokens = connect(&client, &user_id, "posts:read").await;
    let called = client
        .mcp()
        .with_token(&tokens.access)
        .call("users.profile", json!({}))
        .await;
    assert_eq!(called.status(), 200, "{:?}", called.response());
    assert_eq!(
        called.structured(),
        Some(&json!({ "user": user_id, "browser": false }))
    );

    // A real password change: the reset link the user is mailed, then the
    // reset it completes, which moves Magnetar's epoch.
    let mail = Mail::fake();
    let _queue = suprnova::Queue::fake();
    PasswordReset::send_link(email, "https://app.test/reset")
        .await
        .expect("send the reset link");
    let captured = mail.captured();
    assert_eq!(captured.len(), 1, "one reset mail");
    let text = captured[0].text.clone().unwrap_or_default();
    let link = text
        .lines()
        .find(|line| line.contains("token="))
        .expect("the reset link")
        .trim()
        .to_owned();
    let token = url::Url::parse(&link)
        .expect("an absolute link")
        .query_pairs()
        .find(|(name, _)| name == "token")
        .map(|(_, value)| value.into_owned())
        .expect("the reset token");
    PasswordReset::complete(&token, "a-brand-new-password")
        .await
        .expect("complete the reset");

    let reason = refusal_reason(&client, &tokens.access).await;
    assert!(
        reason.contains("ended with the user's sign-in"),
        "the grant ended with Magnetar's epoch: {reason}"
    );
    assert_eq!(
        refresh(&client, &tokens.client_id, &tokens.refresh)
            .await
            .status(),
        400
    );
}
