//! LDB-004: `$2y$` hashes Laravel wrote verify through `Auth::attempt` and
//! Magnetar, passwords of 72 bytes and longer included; by default the
//! framework writes `$2b$` and Magnetar upgrades to Argon2id; with the
//! setting on, every hash the framework and Magnetar write is one Laravel
//! 13 with `HASH_VERIFY=true` accepts.

use secrecy::SecretString;
use suprnova::hashing::{Argon2Options, Argon2idHasher, BcryptHasher, Hasher};
use suprnova::{Auth, Credentials};

use crate::on_every_engine;
use crate::support::{self, Engine};

/// PHP 8.4 reports `hash` as bcrypt and verifies `password` against it.
fn assert_laravel_accepts(hash: &str, password: &str, what: &str) {
    let (algo, ok) = support::php_password(hash, password);
    assert_eq!(algo, "bcrypt", "{what}: PHP reports {hash} as {algo}");
    assert!(ok, "{what}: PHP's password_verify refused {hash}");
    assert!(hash.starts_with("$2y$"), "{what}: {hash} is not $2y$");
}

/// Every user Laravel 13 registered signs in through `Auth::attempt`, the
/// 72-byte and 100-byte passwords included; a wrong password does not.
async fn laravel_hashes_sign_in(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    support::install_scaffold_auth().await;
    assert!(fixture.users.values().any(|u| u.password.len() == 72));
    assert!(fixture.users.values().any(|u| u.password.len() == 100));
    for (email, user) in &fixture.users {
        let hash = support::stored_hash(&db.conn, email).await;
        assert!(hash.starts_with("$2y$"), "Laravel wrote {hash}");
        let signed_in = support::in_request(Auth::attempt(
            &Credentials::password(email.as_str(), user.password.as_str()),
            false,
        ))
        .await
        .expect("attempt");
        assert_eq!(
            signed_in.map(|u| u.get_auth_identifier()),
            Some(user.id.to_string()),
            "{engine:?}: {email} ({} bytes) did not sign in",
            user.password.len()
        );
        let wrong = support::in_request(Auth::attempt(
            &Credentials::password(email.as_str(), "not the password"),
            false,
        ))
        .await
        .expect("attempt");
        assert!(wrong.is_none(), "{email} signed in with a wrong password");
    }
}

on_every_engine!(laravel_hashes_sign_in =>
    ldb_004_laravel_hashes_sign_in_through_auth_attempt_sqlite,
    ldb_004_laravel_hashes_sign_in_through_auth_attempt_postgres,
    ldb_004_laravel_hashes_sign_in_through_auth_attempt_mysql);

/// By default the framework writes `$2b$`, and Magnetar's verifier upgrades
/// a valid bcrypt sign-in to Argon2id.
#[tokio::test]
#[serial_test::serial]
async fn ldb_004_by_default_hashes_are_2b_and_magnetar_upgrades_to_argon2id() {
    suprnova::LaravelDatabase::follow_environment();
    let hash = suprnova::hashing::hash("secret").expect("hash");
    assert!(hash.starts_with("$2b$"), "the default hash is {hash}");
    assert!(!suprnova::hashing::needs_rehash(&hash));

    let fixture = support::fixture(Engine::Sqlite);
    let laravel_hash = {
        let (db, _) = support::laravel(Engine::Sqlite).await;
        support::stored_hash(&db.conn, "taylor@example.com").await
    };
    let verifier = magnetar::password::PasswordVerifier::new(
        std::sync::Arc::new(magnetar::password::StandardPasswordHashDriver),
        magnetar::password::PasswordHashConfig::default(),
    )
    .expect("a verifier");
    let verdict = verifier
        .verify_attempt(
            Some(&laravel_hash),
            &SecretString::from(fixture.users["taylor@example.com"].password.clone()),
        )
        .expect("verify");
    assert!(verdict.valid);
    match verdict.rehash {
        magnetar::password::RehashOutcome::Upgraded(upgraded) => {
            assert!(upgraded.starts_with("$argon2id$"), "upgraded to {upgraded}")
        }
        other => panic!("a bcrypt sign-in was not upgraded to Argon2id: {other:?}"),
    }
}

/// With the setting on, registration, a password reset and a sign-in that
/// rewrites a `$2b$` or Argon2id hash all leave a `$2y$` hash Laravel 13
/// verifies; passwords of 72 bytes and longer included.
async fn setting_writes_laravels_hashes(engine: Engine) {
    let _shared = support::Shared::on();
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);
    support::install_scaffold_auth().await;

    // Registration, through the scaffold's `User::create`.
    let long = format!("x{}x", "\u{e9}".repeat(49));
    for (email, password) in [
        ("short@example.com", "registered-password"),
        ("long@example.com", long.as_str()),
    ] {
        crate::scaffold::user::User::create("New", email, password)
            .await
            .expect("register");
        assert_laravel_accepts(
            &support::stored_hash(&db.conn, email).await,
            password,
            "registration",
        );
    }

    // A valid sign-in rewrites a `$2b$` and an Argon2id hash.
    for (email, hash) in [
        (
            "short@example.com",
            BcryptHasher::default().hash("rewrite-me").expect("$2b$"),
        ),
        (
            "long@example.com",
            Argon2idHasher::new(Argon2Options::default())
                .expect("an argon2id hasher")
                .hash("rewrite-me")
                .expect("argon2id"),
        ),
    ] {
        db.conn
            .execute_unprepared(&format!(
                "UPDATE users SET password = '{hash}' WHERE email = '{email}'"
            ))
            .await
            .expect("store the hash");
        let signed_in = support::in_request(Auth::attempt(
            &Credentials::password(email, "rewrite-me"),
            false,
        ))
        .await
        .expect("attempt");
        assert!(signed_in.is_some(), "{engine:?}: {email} did not sign in");
        assert_laravel_accepts(
            &support::stored_hash(&db.conn, email).await,
            "rewrite-me",
            "sign-in rewrite",
        );
    }

    // A password reset, through the provider's reset flow.
    // SAFETY: the test is serial within its process.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.test") };
    let mail = suprnova::Mail::fake();
    suprnova::auth_flows::PasswordReset::send_link("taylor@example.com", "https://app.test/reset")
        .await
        .expect("send the reset link");
    let token = mail
        .captured()
        .first()
        .and_then(|m| m.text.clone())
        .and_then(|text| {
            text.lines()
                .find(|line| line.contains("token="))
                .and_then(|line| line.rsplit("token=").next())
                .map(|token| token.trim().to_owned())
        })
        .expect("the reset token");
    suprnova::auth_flows::PasswordReset::complete(&token, "after-the-reset")
        .await
        .expect("complete the reset");
    assert_laravel_accepts(
        &support::stored_hash(&db.conn, "taylor@example.com").await,
        "after-the-reset",
        "password reset",
    );
}

use sea_orm::ConnectionTrait;

on_every_engine!(setting_writes_laravels_hashes =>
    ldb_004_with_the_setting_the_framework_writes_hashes_laravel_accepts_sqlite,
    ldb_004_with_the_setting_the_framework_writes_hashes_laravel_accepts_postgres,
    ldb_004_with_the_setting_the_framework_writes_hashes_laravel_accepts_mysql);

/// Set up Magnetar's default engine on `db`, as `init_magnetar` does for
/// an application, inside the caller's container scope.
async fn init_magnetar(db: &support::Db) {
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    suprnova::rate_limit::bootstrap_default().await;
    suprnova::init_magnetar(suprnova::MagnetarConfig::from_sea_orm(db.conn.clone()))
        .await
        .expect("init_magnetar");
}

/// Put a user with `hash` into Magnetar's user table, as an import of the
/// Laravel user carries the hash.
async fn magnetar_user(db: &support::Db, email: &str, hash: &str) {
    db.conn
        .execute_unprepared(&format!(
            "INSERT INTO app_users (email, name, password_hash, auth_epoch) \
             VALUES ('{email}', 'Imported', '{hash}', 0)"
        ))
        .await
        .expect("insert a Magnetar user");
}

async fn magnetar_hash(db: &support::Db, email: &str) -> String {
    let rows = support::rows(
        &db.conn,
        &format!("SELECT password_hash FROM app_users WHERE email = '{email}'"),
    )
    .await;
    support::text(&rows[0], "password_hash")
}

async fn magnetar_sign_in(email: &str, password: &str) -> bool {
    support::in_request(Auth::password().authenticate(email, password, None, None))
        .await
        .is_ok()
}

/// Every hash Laravel 13 wrote signs in through Magnetar, the 72-byte and
/// 100-byte passwords included, and by default Magnetar then upgrades it
/// to Argon2id.
async fn laravel_hashes_sign_in_through_magnetar(engine: Engine) {
    suprnova::LaravelDatabase::follow_environment();
    let (db, fixture) = support::laravel(engine).await;
    let _bound = support::bind(&db.conn);
    init_magnetar(&db).await;
    for (email, user) in &fixture.users {
        let laravel_hash = support::stored_hash(&db.conn, email).await;
        magnetar_user(&db, email, &laravel_hash).await;
        assert!(
            magnetar_sign_in(email, &user.password).await,
            "{engine:?}: {email} ({} bytes) did not sign in through Magnetar",
            user.password.len()
        );
        assert!(!magnetar_sign_in(email, "not the password").await);
        let upgraded = magnetar_hash(&db, email).await;
        assert!(
            upgraded.starts_with("$argon2id$"),
            "{engine:?}: by default Magnetar upgrades to Argon2id, it stored {upgraded}"
        );
    }
}

#[test]
#[serial_test::serial]
fn ldb_004_laravel_hashes_sign_in_through_magnetar_sqlite() {
    support::alone(
        "passwords::ldb_004_laravel_hashes_sign_in_through_magnetar_sqlite",
        || laravel_hashes_sign_in_through_magnetar(Engine::Sqlite),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway Postgres at PG_TEST_URL"]
fn ldb_004_laravel_hashes_sign_in_through_magnetar_postgres() {
    support::alone(
        "passwords::ldb_004_laravel_hashes_sign_in_through_magnetar_postgres",
        || laravel_hashes_sign_in_through_magnetar(Engine::Postgres),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
fn ldb_004_laravel_hashes_sign_in_through_magnetar_mysql() {
    support::alone(
        "passwords::ldb_004_laravel_hashes_sign_in_through_magnetar_mysql",
        || laravel_hashes_sign_in_through_magnetar(Engine::Mysql),
    );
}

/// With the setting on, Magnetar registers, rewrites a `$2b$` or Argon2id
/// hash on a valid sign-in, and resets a password to a `$2y$` hash Laravel
/// 13 verifies, and upgrades nothing to Argon2id.
async fn magnetar_writes_laravels_hashes(engine: Engine) {
    let _shared = support::Shared::on();
    let (db, _) = support::laravel(engine).await;
    let _bound = support::bind(&db.conn);
    init_magnetar(&db).await;

    let long = format!("x{}x", "\u{e9}".repeat(49));
    for (email, password) in [
        ("short@example.com", "registered-password"),
        ("long@example.com", long.as_str()),
    ] {
        support::in_request(Auth::password().register(email, password))
            .await
            .expect("register through Magnetar");
        assert_laravel_accepts(
            &magnetar_hash(&db, email).await,
            password,
            "Magnetar registration",
        );
    }

    for (email, hash) in [
        (
            "rewrite-2b@example.com",
            BcryptHasher::default().hash("rewrite-me").expect("$2b$"),
        ),
        (
            "rewrite-argon@example.com",
            Argon2idHasher::new(Argon2Options::default())
                .expect("an argon2id hasher")
                .hash("rewrite-me")
                .expect("argon2id"),
        ),
    ] {
        magnetar_user(&db, email, &hash).await;
        assert!(
            magnetar_sign_in(email, "rewrite-me").await,
            "{email} signs in"
        );
        assert_laravel_accepts(
            &magnetar_hash(&db, email).await,
            "rewrite-me",
            "Magnetar sign-in rewrite",
        );
    }

    // A password reset through Magnetar's engine.
    // SAFETY: the test runs alone in its process.
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.test") };
    db.conn
        .execute_unprepared(
            "UPDATE app_users SET email_verified_at = CURRENT_TIMESTAMP \
             WHERE email = 'short@example.com'",
        )
        .await
        .expect("verify the address");
    let mail = suprnova::Mail::fake();
    suprnova::auth_flows::PasswordReset::send_link("short@example.com", "https://app.test/reset")
        .await
        .expect("send the reset link");
    let token = mail
        .captured()
        .first()
        .and_then(|m| m.text.clone())
        .and_then(|text| {
            text.lines()
                .find(|line| line.contains("token="))
                .and_then(|line| line.rsplit("token=").next())
                .map(|token| token.trim().to_owned())
        })
        .expect("the reset token");
    suprnova::auth_flows::PasswordReset::complete(&token, "after-the-reset")
        .await
        .expect("complete the reset");
    assert_laravel_accepts(
        &magnetar_hash(&db, "short@example.com").await,
        "after-the-reset",
        "Magnetar password reset",
    );
}

#[test]
#[serial_test::serial]
fn ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_sqlite() {
    support::alone(
        "passwords::ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_sqlite",
        || magnetar_writes_laravels_hashes(Engine::Sqlite),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway Postgres at PG_TEST_URL"]
fn ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_postgres() {
    support::alone(
        "passwords::ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_postgres",
        || magnetar_writes_laravels_hashes(Engine::Postgres),
    );
}

#[test]
#[serial_test::serial]
#[ignore = "requires a throwaway MySQL at MYSQL_TEST_URL"]
fn ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_mysql() {
    support::alone(
        "passwords::ldb_004_with_the_setting_magnetar_writes_hashes_laravel_accepts_mysql",
        || magnetar_writes_laravels_hashes(Engine::Mysql),
    );
}
