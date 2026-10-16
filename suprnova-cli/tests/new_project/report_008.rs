use super::{BIN, scaffold};
use std::process::Command;

#[test]
fn report_008_compose_offers_persistent_loopback_rustrak_with_a_fresh_secret() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut secrets = Vec::new();
    for name in ["tracker-one", "tracker-two"] {
        let project = scaffold(tmp.path(), name, "api");
        let output = Command::new(BIN)
            .args(["docker:compose", "--with-rustrak"])
            .current_dir(&project)
            .output()
            .expect("spawn compose generator");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let compose =
            std::fs::read_to_string(project.join("docker-compose.yml")).expect("generated compose");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(project.join("docker-compose.yml"))
                .expect("compose permissions")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "the generated tracker session key is private");
        }
        for expected in [
            "rustrak/rustrak-server:v0.16.4",
            "127.0.0.1:${RUSTRAK_PORT:-8080}:8080",
            "rustrak_data:/data",
            "  rustrak_data:",
            "PUBLIC_URL: http://127.0.0.1:${RUSTRAK_PORT:-8080}",
            "RUSTRAK_TELEMETRY: \"off\"",
        ] {
            assert!(compose.contains(expected), "missing {expected}");
        }
        let secret = compose
            .lines()
            .find_map(|line| line.trim().strip_prefix("SESSION_SECRET_KEY: "))
            .expect("generated session key")
            .trim_matches('"');
        assert!(
            secret.len() >= 64,
            "Rustrak requires a key of at least 64 characters"
        );
        assert!(
            !secret.contains('{'),
            "key must be generated before the file is written"
        );
        secrets.push(secret.to_string());
        assert!(!compose.contains("mailpit:"));
        assert!(!compose.contains("minio:"));

        let repeated = Command::new(BIN)
            .args(["docker:compose", "--with-rustrak"])
            .current_dir(&project)
            .output()
            .expect("repeat compose generator");
        assert!(repeated.status.success());
        let unchanged =
            std::fs::read_to_string(project.join("docker-compose.yml")).expect("existing compose");
        assert!(
            compose == unchanged,
            "existing service settings must be preserved"
        );
    }
    assert_ne!(secrets[0], secrets[1]);
}

#[test]
fn report_008_rustrak_is_optional_and_existing_service_flags_still_work() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(tmp.path(), "mail-only", "api");
    let output = Command::new(BIN)
        .args(["docker:compose", "--with-mailpit"])
        .current_dir(&project)
        .output()
        .expect("spawn compose generator");
    assert!(output.status.success());
    let compose =
        std::fs::read_to_string(project.join("docker-compose.yml")).expect("generated compose");
    assert!(compose.contains("mailpit:"));
    assert!(!compose.contains("rustrak"));
}

#[test]
fn report_008_environment_templates_document_reporting_without_turning_it_on() {
    for template in [
        include_str!("../../src/templates/files/root/env.tpl"),
        include_str!("../../src/templates/files/root/env.example.tpl"),
        include_str!("../../src/templates/files/api/.env.tpl"),
    ] {
        for setting in [
            "# REPORTING_DRIVER=sentry",
            "# REPORTING_DSN=",
            "# REPORTING_RELEASE=",
            "# REPORTING_SEND_PII=false",
            "# REPORTING_LOGS=off",
            "# REPORTING_TRACES_SAMPLE_RATE=0",
            "# REPORTING_PUBLIC_DSN=",
        ] {
            assert!(
                template.lines().any(|line| line == setting),
                "missing {setting}"
            );
        }
    }
}
