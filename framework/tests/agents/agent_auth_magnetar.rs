//! AGENT-009 where Magnetar owns the default guard's sessions: the claim
//! page compares the user's Magnetar session with the user Magnetar's own
//! store reports, a bound agent's grant records Magnetar's epoch, and a
//! real password reset ends the agent's token as it ends the sessions.
//!
//! Its own process: it installs the process-wide Magnetar engine, which the
//! tests of this binary that run without one must not see.

use sea_orm::{ConnectionTrait, Database};
use serde_json::json;
use suprnova::agents::AgentAuth;
use suprnova::{Auth, MagnetarConfig, init_magnetar};

use crate::agent_auth::agent_world_in;
use crate::oauth::{container, remember_account, sign_in};

const PASSWORD: &str = "correct-horse-battery";

#[tokio::test]
async fn agent_009_claim_under_magnetar_sessions() {
    if crate::own_process_async::delegate(module_path!(), "agent_009_claim_under_magnetar_sessions")
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
    let container = container();
    let db = Database::connect("sqlite::memory:").await.unwrap();
    init_magnetar(MagnetarConfig::from_sea_orm(db.clone()))
        .await
        .expect("install the default Magnetar engine and its tables");
    AgentAuth::migrate(&db).await.unwrap();
    let email = "magnetar-agent@example.test";
    let user = Auth::password()
        .register(email, PASSWORD)
        .await
        .expect("register a Magnetar user")
        .created()
        .expect("a new account");
    let user_id = user.id.to_string();
    // The claim is bound to a verified email, which Magnetar's own store
    // reports.
    db.execute_unprepared(&format!(
        "UPDATE app_users SET email_verified_at = '2026-01-01 00:00:00' WHERE id = {user_id}"
    ))
    .await
    .unwrap();
    remember_account(&user_id);
    let world = agent_world_in(container, db.clone(), |agent| agent);
    let client = &world.client;

    let registration = client
        .post("/agent/identity")
        .json(&json!({ "type": "service_auth", "login_hint": email }))
        .send()
        .await;
    assert_eq!(registration.status(), 200, "{}", registration.body_text());
    let registration = registration.json();
    sign_in(client, &user_id).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    let page = client.get(crate::oauth::path_of(uri)).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let text = page.body_text();
    let attempt_marker = "name=\"claim_attempt_token\" value=\"";
    let start = text.find(attempt_marker).unwrap() + attempt_marker.len();
    let attempt = &text[start..start + text[start..].find('"').unwrap()];
    let done = client
        .post("/agent/claim")
        .form(&[
            ("_token", crate::oauth::form_token(&text).as_str()),
            ("claim_attempt_token", attempt),
            (
                "user_code",
                registration["claim"]["user_code"].as_str().unwrap(),
            ),
        ])
        .send()
        .await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    let tokens = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
            ("claim_token", registration["claim_token"].as_str().unwrap()),
            ("resource", "http://localhost/mcp"),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"].as_str().unwrap().to_owned();
    let created = client
        .mcp()
        .with_token(&token)
        .call(
            "posts.create",
            json!({ "title": "From a Magnetar user's agent" }),
        )
        .await;
    assert_eq!(created.status(), 200, "{:?}", created.response());
    assert_eq!(created.structured().unwrap()["user"], user_id);

    crate::oauth_magnetar::reset_password(email).await;

    let refused = client
        .mcp()
        .with_token(&token)
        .call("posts.show", json!({ "post": "1" }))
        .await;
    assert_eq!(refused.status(), 401, "{:?}", refused.response());
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(
        www.contains("ended with the user's sign-in"),
        "the agent's grant ended with Magnetar's epoch: {www}"
    );
}
