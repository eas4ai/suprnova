#![cfg(feature = "testing")]

use std::any::Any;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;

use suprnova::auth::AuthConfig;
use suprnova::auth_flows::PasswordReset;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::container::testing::TestContainer;
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, AuthManager, Authenticatable, CanResetPassword, EloquentUserProvider, MustVerifyEmail,
    UserProvider, model,
};

#[model(table = "users", fillable = ["email", "password"])]
pub struct TestUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl Authenticatable for TestUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for TestUser {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}

impl CanResetPassword for TestUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }

    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_owned();
    }
}

fn token_from_fake(fake: &suprnova::mail::MailFake) -> String {
    let captured = fake.captured();
    let text = captured
        .first()
        .expect("at least one reset mail")
        .text
        .as_deref()
        .expect("reset mail text body");
    let link = text
        .lines()
        .find(|line| line.contains("token="))
        .expect("reset link");
    link.rsplit("token=")
        .next()
        .expect("token value")
        .trim()
        .to_owned()
}

#[tokio::test]
async fn verified_provider_user_resets_without_a_magnetar_engine() {
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.test");
    }

    let database = TestDatabase::sqlite_memory()
        .await
        .expect("sqlite database");
    let connection = database.conn();
    connection
        .execute_unprepared(
            "CREATE TABLE users (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                email TEXT NOT NULL, \
                password TEXT NOT NULL, \
                email_verified_at TEXT\
             )",
        )
        .await
        .expect("create users table");
    connection
        .execute(&create_auth_flow_tokens_table())
        .await
        .expect("create auth-flow token table");

    let old_hash = suprnova::hash("old-password").expect("old password hash");
    connection
        .execute_unprepared(&format!(
            "INSERT INTO users (email, password, email_verified_at) VALUES \
             ('verified@example.test', '{old_hash}', CURRENT_TIMESTAMP), \
             ('unverified@example.test', '{old_hash}', NULL)"
        ))
        .await
        .expect("seed provider users");

    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<TestUser>::new()))
        .expect("register provider");
    suprnova::rate_limit::bootstrap_default().await;

    let mail = suprnova::Mail::fake();
    PasswordReset::send_link("verified@example.test", "https://app.test/reset-password")
        .await
        .expect("verified provider reset link");
    mail.assert_sent_to("verified@example.test");
    let token = token_from_fake(&mail);

    let absent_mail = suprnova::Mail::fake();
    PasswordReset::send_link("missing@example.test", "https://app.test/reset-password")
        .await
        .expect("missing user remains anti-enumerating");
    assert_eq!(absent_mail.count(), 0);

    let unverified_mail = suprnova::Mail::fake();
    PasswordReset::send_link("unverified@example.test", "https://app.test/reset-password")
        .await
        .expect("unverified user remains anti-enumerating");
    assert_eq!(unverified_mail.count(), 0);

    assert!(PasswordReset::check(&token).await.expect("check token"));
    let user_id = PasswordReset::complete(&token, "new-password")
        .await
        .expect("complete provider reset");
    assert_eq!(user_id, "1");

    let user = EloquentUserProvider::<TestUser>::new()
        .retrieve_by_id(&user_id)
        .await
        .expect("reload provider user")
        .expect("provider user exists");
    let stored_hash = user.get_auth_password().expect("stored password hash");
    assert!(suprnova::hashing::verify("new-password", stored_hash).expect("verify new password"));
    assert!(!suprnova::hashing::verify("old-password", stored_hash).expect("reject old password"));
    assert!(
        !PasswordReset::check(&token)
            .await
            .expect("check consumed token")
    );
}
