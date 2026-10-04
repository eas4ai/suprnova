#![cfg(feature = "testing")]

//! Cross-facade lockout test for failed two-factor and recovery-code attempts.

use crate::magnetar_auth;

use once_cell::sync::Lazy;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use serial_test::serial;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

use suprnova::auth_flows::two_factor::migration::Migration as TwoFactorMigration;
use suprnova::auth_flows::two_factor::migration_replay::Migration as TwoFactorReplayMigration;
use suprnova::auth_flows::{BruteForce, TwoFactor, TwoFactorUser};
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

#[test]
#[serial]
fn failed_2fa_verifies_lock_the_account() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        // Register through magnetar so the user row exists for the
        // brute-force email lookups.
        suprnova::Auth::password()
            .register("victim-bf-2fa@example.com", "longpassword123")
            .await
            .expect("register")
            .created()
            .expect("registration creates a new account");

        let user = FakeUser {
            id: "victim-bf-2fa-uid".into(),
            email: "victim-bf-2fa@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");

        // Precondition.
        assert!(
            !BruteForce::is_locked(user.email()).await.unwrap(),
            "freshly-enrolled account must not be locked"
        );

        // 5 wrong codes = default BruteForceProtectionConfig threshold.
        // Each failed verify records a brute-force attempt; the 5th
        // crosses the lockout.
        for _ in 0..5 {
            let _ = TwoFactor::verify(&user, "000000").await.unwrap();
        }

        assert!(
            BruteForce::is_locked(user.email()).await.unwrap(),
            "5 failed 2FA verifies must lock the account via BruteForce"
        );
    });
}

#[test]
#[serial]
fn failed_recovery_code_consumes_lock_the_account() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        suprnova::Auth::password()
            .register("victim-rec-bf@example.com", "longpassword123")
            .await
            .expect("register")
            .created()
            .expect("registration creates a new account");

        let user = FakeUser {
            id: "victim-rec-bf-uid".into(),
            email: "victim-rec-bf@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");

        // Clear any failed counter from the previous test (#[serial]
        // gives ordering but the static magnetar instance persists across
        // tests in this binary - a residual counter from a prior file
        // would let this test pass for the wrong reason).
        BruteForce::reset_attempts(user.email()).await.unwrap();
        BruteForce::unlock_account(user.email()).await.unwrap();
        assert!(!BruteForce::is_locked(user.email()).await.unwrap());

        for _ in 0..5 {
            let consumed = TwoFactor::consume_recovery_code(&user, "no-such-code-zzz")
                .await
                .unwrap();
            assert!(!consumed);
        }

        assert!(
            BruteForce::is_locked(user.email()).await.unwrap(),
            "5 failed recovery-code consumes must lock the account via BruteForce"
        );
    });
}

#[test]
#[serial]
fn successful_2fa_verify_resets_failed_attempts() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        suprnova::Auth::password()
            .register("success-resets@example.com", "longpassword123")
            .await
            .expect("register")
            .created()
            .expect("registration creates a new account");

        let user = FakeUser {
            id: "success-resets-uid".into(),
            email: "success-resets@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");

        // Pile up some failures - but stop one short of lockout.
        BruteForce::reset_attempts(user.email()).await.unwrap();
        BruteForce::unlock_account(user.email()).await.unwrap();
        for _ in 0..4 {
            let _ = TwoFactor::verify(&user, "000000").await.unwrap();
        }
        let status_pre = BruteForce::get_lockout_status(user.email()).await.unwrap();
        assert!(status_pre.failed_attempts >= 4);
        assert!(!status_pre.is_locked);

        // A successful verify clears the counter.
        let live = totp_code_for(&resp.otpauth_url);
        assert!(TwoFactor::verify(&user, &live).await.unwrap());

        let status_post = BruteForce::get_lockout_status(user.email()).await.unwrap();
        assert_eq!(
            status_post.failed_attempts, 0,
            "successful verify must reset the failed-attempt counter"
        );
    });
}

/// Five wrong codes through the direct `verify` primitive lock the account,
/// and the lock then holds against the right code: `verify` refuses with 429
/// instead of evaluating it, and the refusal does not reset the counter. The
/// manual gates login on `verify`, so without this the TOTP space is
/// guessable at request rate by anyone who knows the password.
#[test]
#[serial]
fn verify_refuses_a_valid_code_once_the_account_is_locked() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user = FakeUser {
            id: "locked-verify-uid".into(),
            email: "locked-verify@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");
        BruteForce::reset_attempts(user.email()).await.unwrap();

        for _ in 0..5 {
            assert!(!TwoFactor::verify(&user, "000000").await.unwrap());
        }
        assert!(BruteForce::is_locked(user.email()).await.unwrap());

        let live = totp_code_for(&resp.otpauth_url);
        let error = TwoFactor::verify(&user, &live)
            .await
            .expect_err("a locked account must not accept even the right code");
        assert_eq!(error.status_code(), 429);
        assert!(
            BruteForce::is_locked(user.email()).await.unwrap(),
            "the refused attempt must leave the lock in place"
        );

        // After an unlock the same code still verifies: the locked attempt
        // never claimed its timestep.
        BruteForce::unlock_account(user.email()).await.unwrap();
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
        let user = FakeUser {
            id: "locked-recovery-uid".into(),
            email: "locked-recovery@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");
        BruteForce::reset_attempts(user.email()).await.unwrap();

        for _ in 0..5 {
            assert!(
                !TwoFactor::consume_recovery_code(&user, "000000-000000")
                    .await
                    .unwrap()
            );
        }
        assert!(BruteForce::is_locked(user.email()).await.unwrap());

        let valid = resp.recovery_codes[0].clone();
        let error = TwoFactor::consume_recovery_code(&user, &valid)
            .await
            .expect_err("a locked account must not accept even a valid recovery code");
        assert_eq!(error.status_code(), 429);
        assert!(BruteForce::is_locked(user.email()).await.unwrap());

        BruteForce::unlock_account(user.email()).await.unwrap();
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
        let user = FakeUser {
            id: "threshold-verify-uid".into(),
            email: "threshold-verify@example.com".into(),
        };
        let resp = TwoFactor::enroll(&user).await.expect("enroll");
        TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
            .await
            .expect("confirm");
        BruteForce::reset_attempts(user.email()).await.unwrap();

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
