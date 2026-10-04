#![cfg(feature = "testing")]

//! Lockout of the two-factor proof paths with a Magnetar engine installed.
//!
//! Second-factor failures have a counter of their own, keyed by the
//! enrollment's user id; these tests drive it through the public facade.

use crate::magnetar_auth;

use once_cell::sync::Lazy;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use serial_test::serial;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

use suprnova::auth_flows::two_factor::migration::Migration as TwoFactorMigration;
use suprnova::auth_flows::two_factor::migration_attempts::Migration as TwoFactorAttemptsMigration;
use suprnova::auth_flows::two_factor::migration_replay::Migration as TwoFactorReplayMigration;
use suprnova::auth_flows::{TwoFactor, TwoFactorUser};
use suprnova::container::App;
use suprnova::database::DbConnection;

use suprnova::{Crypt, EncryptionKey};

/// One tokio runtime across every test in this file.
static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

/// One-time setup for framework storage and Magnetar authentication.
static SETUP: Lazy<()> = Lazy::new(|| {
    static CRYPT_INIT: OnceLock<()> = OnceLock::new();
    CRYPT_INIT.get_or_init(|| {
        Crypt::init(EncryptionKey::generate());
    });

    RT.block_on(async {
        let conn = Database::connect("sqlite:file::memory:?cache=shared")
            .await
            .expect("sqlite in-memory connection");

        // Install in the container so DB::connection() resolves.
        App::singleton(DbConnection::from_raw(conn.clone()));

        // 2FA tables.
        struct TfMigrator;
        impl MigratorTrait for TfMigrator {
            fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
                vec![
                    Box::new(TwoFactorMigration),
                    Box::new(TwoFactorReplayMigration),
                    Box::new(TwoFactorAttemptsMigration),
                ]
            }
        }
        TfMigrator::up(&conn, None)
            .await
            .expect("two_factor migrations");

        magnetar_auth::install().await;
    });
});

struct FakeUser {
    id: String,
    email: String,
}

impl TwoFactorUser for FakeUser {
    fn user_id(&self) -> &str {
        &self.id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

fn totp_code_for(otpauth_url: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    let url = url::Url::parse(otpauth_url).expect("otpauth url");
    let secret = url
        .query_pairs()
        .find(|(k, _)| k == "secret")
        .map(|(_, v)| v.into_owned())
        .expect("secret query param");
    let secret_bytes = Secret::Encoded(secret).to_bytes().expect("decode secret");
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        None,
        "label".into(),
    )
    .expect("totp")
    .generate_current()
    .expect("generate")
}

async fn enrolled(id: &str) -> (FakeUser, suprnova::auth_flows::EnrollmentResponse) {
    let user = FakeUser {
        id: format!("{id}-uid"),
        email: format!("{id}@example.com"),
    };
    let resp = TwoFactor::enroll(&user).await.expect("enroll");
    TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
        .await
        .expect("confirm");
    (user, resp)
}

#[test]
#[serial]
fn failed_2fa_verifies_lock_the_second_factor() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, resp) = enrolled("victim-bf-2fa").await;

        // Five wrong codes reach the threshold; each is evaluated.
        for _ in 0..5 {
            assert!(!TwoFactor::verify(&user, "000000").await.unwrap());
        }
        let error = TwoFactor::verify(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect_err("the sixth attempt is refused");
        assert_eq!(error.status_code(), 429);
    });
}

#[test]
#[serial]
fn failed_recovery_code_consumes_lock_the_second_factor() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, resp) = enrolled("victim-rec-bf").await;

        for _ in 0..5 {
            let consumed = TwoFactor::consume_recovery_code(&user, "no-such-code-zzz")
                .await
                .unwrap();
            assert!(!consumed);
        }
        let error = TwoFactor::consume_recovery_code(&user, &resp.recovery_codes[0])
            .await
            .expect_err("the sixth attempt is refused");
        assert_eq!(error.status_code(), 429);
    });
}

#[test]
#[serial]
fn successful_2fa_verify_resets_failed_attempts() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, resp) = enrolled("success-resets").await;

        // Four failures, one short of the lock, then the right code.
        for _ in 0..4 {
            assert!(!TwoFactor::verify(&user, "000000").await.unwrap());
        }
        assert!(
            TwoFactor::verify(&user, &totp_code_for(&resp.otpauth_url))
                .await
                .unwrap()
        );

        // The success cleared them: four more wrong codes are evaluated
        // rather than refused.
        for _ in 0..4 {
            assert!(!TwoFactor::verify(&user, "000000").await.unwrap());
        }
    });
}

/// Five wrong codes through the direct `verify` primitive lock it, and the
/// lock then holds against the right code: `verify` refuses with 429
/// instead of evaluating it. The manual gates login on `verify`, so without
/// this the TOTP space is guessable at request rate by anyone who knows the
/// password.
#[test]
#[serial]
fn verify_refuses_a_valid_code_once_the_account_is_locked() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, resp) = enrolled("locked-verify").await;

        for _ in 0..5 {
            assert!(!TwoFactor::verify(&user, "000000").await.unwrap());
        }
        let live = totp_code_for(&resp.otpauth_url);
        let error = TwoFactor::verify(&user, &live)
            .await
            .expect_err("a locked account must not accept even the right code");
        assert_eq!(error.status_code(), 429);

        // After an admin unlock the same code still verifies: the refused
        // attempt never claimed its timestep.
        assert!(
            TwoFactor::unlock(&user).await.unwrap(),
            "the user was locked"
        );
        assert!(TwoFactor::verify(&user, &live).await.unwrap());
    });
}

/// The recovery-code primitive shares the lock: once wrong recovery codes
/// lock the account, a valid code is refused with 429 and stays unconsumed.
#[test]
#[serial]
fn consume_recovery_code_refuses_a_valid_code_once_the_account_is_locked() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, resp) = enrolled("locked-recovery").await;

        for _ in 0..5 {
            assert!(
                !TwoFactor::consume_recovery_code(&user, "000000-000000")
                    .await
                    .unwrap()
            );
        }
        let valid = resp.recovery_codes[0].clone();
        let error = TwoFactor::consume_recovery_code(&user, &valid)
            .await
            .expect_err("a locked account must not accept even a valid recovery code");
        assert_eq!(error.status_code(), 429);

        assert!(TwoFactor::unlock(&user).await.unwrap());
        assert!(
            TwoFactor::consume_recovery_code(&user, &valid)
                .await
                .unwrap(),
            "the refused attempt must not have consumed the code"
        );
    });
}

/// A wrong code at the threshold is refused before evaluation as well, so
/// the counter cannot be pushed past the lock by parallel guesses that each
/// read "unlocked": every attempt is reserved before its code is read.
#[test]
#[serial]
fn verify_admits_no_more_guesses_than_the_threshold() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let (user, _resp) = enrolled("threshold-verify").await;

        let guesses = (0..8).map(|_| TwoFactor::verify(&user, "000000"));
        let outcomes = futures::future::join_all(guesses).await;
        let evaluated = outcomes.iter().filter(|o| matches!(o, Ok(false))).count();
        let refused = outcomes
            .iter()
            .filter(|o| matches!(o, Err(e) if e.status_code() == 429))
            .count();
        assert_eq!(
            evaluated, 5,
            "exactly the threshold of guesses is evaluated"
        );
        assert_eq!(refused, 3, "every guess past the threshold is refused");
    });
}
