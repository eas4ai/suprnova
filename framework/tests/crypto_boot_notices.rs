//! First-boot key notices must remain visible even when Crypt is initialized
//! before the application's tracing subscriber exists.

use std::process::{Command, Output};

use suprnova::{EncryptionKey, Router, Server};

const CHILD_MODE: &str = "SUPRNOVA_CRYPTO_NOTICE_CHILD";

#[test]
fn crypto_notice_child() {
    if std::env::var(CHILD_MODE).is_err() {
        return;
    }
    suprnova::boot::initialize_crypt_or_exit();
    Server::from_config(Router::new()).expect("child crypto boot");
}

fn run_child(mode: &str) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", "crypto_notice_child", "--nocapture"])
        .env(CHILD_MODE, mode)
        .env_remove("APP_KEY")
        .env_remove("APP_KEY_PREVIOUS")
        .env_remove("APP_PREVIOUS_KEYS");

    match mode {
        "transient" => {
            command.env("APP_ENV", "local");
        }
        "previous" => {
            command
                .env("APP_ENV", "production")
                .env("APP_KEY", EncryptionKey::generate().to_base64())
                .env("APP_KEY_PREVIOUS", EncryptionKey::generate().to_base64());
        }
        other => panic!("unknown child mode: {other}"),
    }

    command.output().expect("spawn crypto-notice child")
}

#[test]
fn transient_key_notice_is_visible_without_a_tracing_subscriber() {
    let output = run_child("transient");
    assert!(output.status.success(), "child failed: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("APP_KEY is not set - generated a transient development key"),
        "transient-key notice must be written somewhere operators can see; stderr:\n{stderr}"
    );
    assert_eq!(
        stderr
            .matches("APP_KEY is not set - generated a transient development key")
            .count(),
        1,
        "the repeated Server validation must not duplicate the first-install notice"
    );
}

#[test]
fn previous_key_notice_is_visible_without_a_tracing_subscriber() {
    let output = run_child("previous");
    assert!(output.status.success(), "child failed: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("APP_KEY_PREVIOUS active"),
        "previous-key notice must be written somewhere operators can see; stderr:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("APP_KEY_PREVIOUS active").count(),
        1,
        "the repeated Server validation must not duplicate the first-install notice"
    );
}
