#![cfg(feature = "testing")]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use secrecy::{ExposeSecret, SecretString};
use suprnova::Mail;
use suprnova::auth_flows::PasswordReset;
use suprnova::magnetar_integration::engine::{
    HostPasswordResetIssued, HostSignInDecision, MagnetarPasswordAuthEngine,
};
use suprnova::magnetar_integration::{LockoutStatus, User};

/// What `user_by_id` answers for the reset mail's greeting.
#[derive(Default)]
enum UserLookup {
    #[default]
    Missing,
    Named(&'static str),
    Fails,
}

#[derive(Default)]
struct ResetEngine {
    issued_for: Mutex<Vec<String>>,
    checked: Mutex<Vec<String>>,
    completed: Mutex<Vec<(String, String)>>,
    lookup: Mutex<UserLookup>,
}

impl ResetEngine {
    fn unavailable() -> magnetar::Error {
        magnetar::Error::Internal {
            message: "unused reset test operation".to_owned(),
        }
    }
}

#[async_trait]
impl MagnetarPasswordAuthEngine for ResetEngine {
    async fn password_sign_in(
        &self,
        _input: magnetar::plugins::password::PasswordAttempt,
    ) -> magnetar::Result<(User, HostSignInDecision)> {
        Err(Self::unavailable())
    }

    async fn password_register(
        &self,
        _input: magnetar::plugins::password::RegisterInput,
    ) -> magnetar::Result<suprnova::Registration> {
        Err(Self::unavailable())
    }

    async fn issue_password_reset(
        &self,
        email: &str,
    ) -> magnetar::Result<Option<HostPasswordResetIssued>> {
        self.issued_for.lock().unwrap().push(email.to_owned());
        Ok(Some(HostPasswordResetIssued {
            user_id: "42".to_owned(),
            email: email.to_owned(),
            token: magnetar::storage::IssuedToken {
                plaintext: SecretString::from("magnetar-reset-token".to_owned()),
                token_id: "reset-row".to_owned(),
                expires_at: Utc::now() + Duration::minutes(15),
            },
        }))
    }

    async fn check_password_reset(&self, token: SecretString) -> magnetar::Result<bool> {
        self.checked
            .lock()
            .unwrap()
            .push(token.expose_secret().to_owned());
        Ok(token.expose_secret() == "magnetar-reset-token")
    }

    async fn complete_password_reset(
        &self,
        token: SecretString,
        password: SecretString,
    ) -> magnetar::Result<magnetar::plugins::password_management::PasswordResetFlowOutcome> {
        self.completed.lock().unwrap().push((
            token.expose_secret().to_owned(),
            password.expose_secret().to_owned(),
        ));
        Ok(
            magnetar::plugins::password_management::PasswordResetFlowOutcome {
                user_id: "42".to_owned(),
                auth_epoch: 7,
                revoked_sessions: 2,
                remember_rows_revoked: 3,
                lockout_cleared: Ok(true),
            },
        )
    }

    async fn bearer_user_id(&self, _token: &str) -> magnetar::Result<Option<String>> {
        Ok(None)
    }

    async fn issue_remember(
        &self,
        _user_id: &str,
        _lifetime: chrono::Duration,
    ) -> magnetar::Result<magnetar::sessions::RememberCredential> {
        Err(Self::unavailable())
    }

    async fn remember_sign_in(
        &self,
        _credential: magnetar::sessions::RememberCredential,
        _metadata: magnetar::sessions::SessionMetadata,
        _replacement_lifetime: chrono::Duration,
    ) -> magnetar::Result<suprnova::magnetar_integration::engine::MagnetarRememberSignIn> {
        Err(Self::unavailable())
    }

    async fn resolve_web_binding(
        &self,
        _binding: &magnetar::sessions::WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        Err(Self::unavailable())
    }

    async fn revoke_remember(&self, _user_id: &str) -> magnetar::Result<u64> {
        Ok(0)
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        match *self.lookup.lock().unwrap() {
            UserLookup::Missing => Ok(None),
            UserLookup::Named(name) => Ok(Some(
                User::builder()
                    .id(suprnova::UserId::new(&format!("usr_{user_id}")))
                    .email(format!("{user_id}@example.test"))
                    .name(Some(name.to_owned()))
                    .build()
                    .map_err(|error| magnetar::Error::Internal {
                        message: error.to_string(),
                    })?,
            )),
            UserLookup::Fails => Err(magnetar::Error::Internal {
                message: "scripted user lookup failure".to_owned(),
            }),
        }
    }

    async fn revoke_session(&self, _session_id: &str) -> magnetar::Result<bool> {
        Ok(false)
    }

    async fn revoke_all_sessions(&self, _user_id: &str) -> magnetar::Result<u64> {
        Ok(0)
    }

    async fn list_sessions(
        &self,
        _user_id: &str,
    ) -> magnetar::Result<Vec<magnetar::sessions::SessionSummary>> {
        Ok(Vec::new())
    }

    async fn record_failed_attempt(
        &self,
        _email: &str,
        _ip_address: Option<&str>,
    ) -> magnetar::Result<LockoutStatus> {
        Err(Self::unavailable())
    }

    async fn lockout_status(&self, _email: &str) -> magnetar::Result<LockoutStatus> {
        Err(Self::unavailable())
    }

    async fn reset_attempts(&self, _email: &str) -> magnetar::Result<()> {
        Ok(())
    }

    async fn unlock_account(&self, _email: &str) -> magnetar::Result<bool> {
        Ok(false)
    }

    async fn magic_link_send(&self, _email: &str) -> magnetar::Result<String> {
        Err(Self::unavailable())
    }

    async fn magic_link_consume(
        &self,
        _token: &str,
        _metadata: magnetar::sessions::SessionMetadata,
    ) -> magnetar::Result<HostSignInDecision> {
        Err(Self::unavailable())
    }
}

#[tokio::test]
async fn password_reset_facade_delegates_issue_check_and_completion_to_magnetar() {
    // Its own process: it installs a process-wide Magnetar password engine,
    // which the tests of this binary that run without an engine must not see.
    if crate::own_process_async::delegate(
        module_path!(),
        "password_reset_facade_delegates_issue_check_and_completion_to_magnetar",
    )
    .await
    {
        return;
    }
    let _env = crate::env_lock::lock_env_async().await;
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.test");
    }
    suprnova::rate_limit::bootstrap_default().await;
    let engine = Arc::new(ResetEngine::default());
    suprnova::magnetar_integration::install_magnetar_password_engine_for_test(engine.clone())
        .unwrap();
    let mail = Mail::fake();

    PasswordReset::send_link("victim@example.test", "https://app.test/reset")
        .await
        .unwrap();
    assert_eq!(
        engine.issued_for.lock().unwrap().as_slice(),
        ["victim@example.test"]
    );
    mail.assert_sent_to("victim@example.test");

    assert!(PasswordReset::check("magnetar-reset-token").await.unwrap());
    let outcome =
        PasswordReset::complete_with_outcome("magnetar-reset-token", "replacement password")
            .await
            .unwrap();
    assert_eq!(outcome.user_id, "42");
    assert_eq!(outcome.sessions_revoked.unwrap(), 2);
    assert_eq!(outcome.remember_tokens_revoked.unwrap(), 3);
    assert_eq!(
        engine.completed.lock().unwrap().as_slice(),
        &[(
            "magnetar-reset-token".to_owned(),
            "replacement password".to_owned(),
        )]
    );
}

#[tokio::test]
async fn magnetar_reset_mail_greets_the_user_by_name() {
    // Its own process: it installs a process-wide Magnetar password engine.
    if crate::own_process_async::delegate(
        module_path!(),
        "magnetar_reset_mail_greets_the_user_by_name",
    )
    .await
    {
        return;
    }
    let _env = crate::env_lock::lock_env_async().await;
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.test");
    }
    suprnova::rate_limit::bootstrap_default().await;
    let engine = Arc::new(ResetEngine::default());
    suprnova::magnetar_integration::install_magnetar_password_engine_for_test(engine.clone())
        .unwrap();
    let mail = Mail::fake();

    for (lookup, address, greeting) in [
        (
            UserLookup::Named("Ada Lovelace"),
            "ada@example.test",
            "Hi Ada Lovelace,",
        ),
        (UserLookup::Missing, "nobody@example.test", "Hi there,"),
        (UserLookup::Fails, "failing@example.test", "Hi there,"),
    ] {
        *engine.lookup.lock().unwrap() = lookup;
        PasswordReset::send_link(address, "https://app.test/reset")
            .await
            .unwrap();
        let sent = mail.sent_to(address);
        assert_eq!(sent.len(), 1, "one reset mail to {address}");
        for body in [&sent[0].text, &sent[0].html] {
            let body = body.as_deref().unwrap_or_default();
            assert!(body.contains(greeting), "{address}: {body}");
        }
    }
}
