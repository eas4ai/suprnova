use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn repository_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn entrypoint_path(relative: &str) -> PathBuf {
    if relative.starts_with(".githooks/")
        || relative.starts_with(".config/")
        || relative == "rust-toolchain.toml"
    {
        workspace_path(relative)
    } else {
        repository_path(relative)
    }
}

fn read_entrypoint(relative: &str) -> String {
    fs::read_to_string(entrypoint_path(relative)).expect("entrypoint must be readable")
}

fn run_script(relative: &str, postgres_url: Option<&str>, mysql_url: Option<&str>) -> Output {
    let mut command = Command::new("bash");
    command
        .arg(entrypoint_path(relative))
        .env_remove("MAGNETAR_POSTGRES_TEST_URL")
        .env_remove("MAGNETAR_MYSQL_TEST_URL");
    if let Some(url) = postgres_url {
        command.env("MAGNETAR_POSTGRES_TEST_URL", url);
    }
    if let Some(url) = mysql_url {
        command.env("MAGNETAR_MYSQL_TEST_URL", url);
    }
    command.output().expect("entrypoint must execute")
}

#[cfg(unix)]
fn assert_executable(relative: &str) {
    use std::os::unix::fs::PermissionsExt;

    let path = entrypoint_path(relative);
    let mode = fs::metadata(path)
        .expect("entrypoint metadata must be readable")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "{relative} must be executable");
}

#[cfg(not(unix))]
fn assert_executable(_relative: &str) {}

#[test]
fn verification_entrypoints_exist_and_are_executable() {
    for relative in [
        "scripts/gate.sh",
        "scripts/check-feature-matrix.sh",
        ".githooks/pre-push",
    ] {
        assert!(entrypoint_path(relative).is_file(), "{relative} must exist");
        assert_executable(relative);
    }
}

#[test]
fn gate_contains_required_checks_and_delegates_feature_matrix() {
    let gate = read_entrypoint("scripts/gate.sh");

    for marker in [
        "set -euo pipefail",
        "cargo check --all-targets --all-features",
        "cargo fmt --all -- --check",
        "cargo clippy --all-targets --all-features",
        "cargo nextest run --profile ci --all-features",
        "cargo test --doc --all-features",
        "MAGNETAR_POSTGRES_TEST_URL",
        "MAGNETAR_MYSQL_TEST_URL",
        "cargo nextest run --profile ci --test concurrency --all-features",
        "cargo fmt --manifest-path examples/host/Cargo.toml -- --check",
        "cargo check --manifest-path examples/host/Cargo.toml",
        "cargo clippy --manifest-path examples/host/Cargo.toml",
        "cargo run --manifest-path examples/host/Cargo.toml -- --smoke-credentials",
        "check-feature-matrix.sh",
        "command -v jq",
        "command -v cargo-nextest",
        "Required command not found",
        "exec",
    ] {
        assert!(gate.contains(marker), "gate is missing marker: {marker}");
    }
    let host_commands = [
        "cargo fmt --manifest-path examples/host/Cargo.toml -- --check",
        "cargo check --manifest-path examples/host/Cargo.toml",
        "cargo clippy --manifest-path examples/host/Cargo.toml",
        "cargo run --manifest-path examples/host/Cargo.toml -- --smoke-credentials",
    ];
    let feature_matrix = gate.find("check-feature-matrix.sh").unwrap();
    let mut previous = 0;
    for command in host_commands {
        let position = gate.find(command).unwrap();
        assert!(position > previous && position < feature_matrix);
        previous = position;
    }
    let exec = gate
        .find("exec \"$ROOT_DIR/scripts/check-feature-matrix.sh\"")
        .unwrap();
    assert!(exec > previous, "feature matrix exec must be final");
    assert!(
        gate.trim_end()
            .ends_with("exec \"$ROOT_DIR/scripts/check-feature-matrix.sh\"")
    );
}

#[test]
fn feature_gate_discovers_and_checks_every_feature() {
    let feature_gate = read_entrypoint("scripts/check-feature-matrix.sh");

    for marker in [
        "set -euo pipefail",
        "FAIL_CLOSED_FEATURE_GATE",
        "cargo metadata --no-deps --format-version 1",
        "select(.name == \"suprnova-magnetar\")",
        "FEATURE_NAMES",
        "FEATURES",
        "--no-default-features",
        "--all-features",
        "--features",
        "cargo check --all-targets",
        "cargo tree --no-default-features",
        "DISABLED_PROVIDER_NAMES",
        "torii-core",
        "torii-storage-seaorm",
        "torii-axum",
        "suprnova-core",
        "oauth2-broker-core",
    ] {
        assert!(
            feature_gate.contains(marker),
            "feature gate is missing marker: {marker}"
        );
    }
    assert!(
        !feature_gate.contains("MAGNETAR_FEATURES"),
        "feature discovery must come from Cargo metadata"
    );
    assert!(
        !feature_gate.contains(".packages[0]"),
        "feature discovery must select suprnova-magnetar by package name"
    );
}

#[test]
fn ci_profile_and_toolchain_requirements_are_pinned() {
    let nextest = read_entrypoint(".config/nextest.toml");
    assert!(
        nextest.contains("[profile.ci]\nfail-fast = false"),
        "CI nextest profile must disable fail-fast"
    );

    let toolchain = read_entrypoint("rust-toolchain.toml");
    assert!(
        toolchain.contains("channel = \"1.91.1\""),
        "the Rust toolchain channel must be pinned to 1.91.1"
    );
    assert!(
        toolchain.contains("components = [\"rustfmt\", \"clippy\"]"),
        "the pinned toolchain must install rustfmt and clippy"
    );
}

#[cfg(unix)]
fn run_feature_matrix_with_tree(tree_output: &str) -> (Output, String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let directory = env::temp_dir().join(format!("magnetar-feature-matrix-{unique}"));
    fs::create_dir(&directory).expect("feature matrix test directory must be creatable");

    let cargo_path = directory.join("cargo");
    fs::write(
        &cargo_path,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FEATURE_MATRIX_LOG"
case "${1-}" in
  metadata)
    printf '%s\n' '{"packages":[{"name":"unrelated","features":{"unrelated":[]}},{"name":"suprnova-magnetar","features":{"default":["password"],"password":[],"email-verification":[]}}]}'
    ;;
  tree)
    printf '%s\n' "$FEATURE_MATRIX_TREE"
    ;;
esac
"#,
    )
    .expect("fake cargo must be writable");
    let mut permissions = fs::metadata(&cargo_path)
        .expect("fake cargo metadata must be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo_path, permissions).expect("fake cargo must be executable");

    let log_path = directory.join("invocations.log");
    let existing_path = env::var_os("PATH").unwrap_or_default();
    let path =
        env::join_paths(std::iter::once(directory.clone()).chain(env::split_paths(&existing_path)))
            .expect("test PATH must be representable");
    let output = Command::new("bash")
        .arg(repository_path("scripts/check-feature-matrix.sh"))
        .env("FEATURE_MATRIX_LOG", &log_path)
        .env("FEATURE_MATRIX_TREE", tree_output)
        .env("PATH", path)
        .output()
        .expect("feature matrix entrypoint must execute");
    let invocations = fs::read_to_string(&log_path).expect("feature matrix log must be readable");
    (output, invocations, directory)
}

#[cfg(unix)]
#[test]
fn feature_gate_executes_the_metadata_derived_matrix() {
    let (output, invocations, directory) = run_feature_matrix_with_tree(
        "suprnova-magnetar v0.1.0\n├── runtime-helper v1.0.0\n│   [build-dependencies]\n│   ├── build-helper v1.0.0\n│   [dev-dependencies]\n│   └── dev-helper v1.0.0",
    );
    assert!(
        output.status.success(),
        "feature matrix failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for invocation in [
        "metadata --no-deps --format-version 1",
        "check --all-targets --no-default-features",
        "check --all-targets --all-features",
        "check --all-targets --no-default-features --features password",
        "check --all-targets --no-default-features --features email-verification",
        "tree --no-default-features --quiet",
        "tree --all-features --quiet",
    ] {
        assert!(
            invocations.contains(invocation),
            "feature matrix did not execute: {invocation}"
        );
    }

    fs::remove_dir_all(directory).expect("feature matrix test directory must be removable");
}

#[cfg(unix)]
#[test]
fn feature_gate_rejects_forbidden_provider_dependencies() {
    let (output, _, directory) =
        run_feature_matrix_with_tree("suprnova-magnetar v0.1.0\ntorii-core v1.0.0");
    assert!(
        !output.status.success(),
        "feature matrix must reject forbidden provider dependencies"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("torii-core"),
        "feature matrix failure must identify the forbidden provider package"
    );
    fs::remove_dir_all(directory).expect("feature matrix test directory must be removable");
}

#[cfg(unix)]
fn run_gate_with_metadata(metadata: &str) -> (Output, String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let directory = env::temp_dir().join(format!("magnetar-gate-{unique}"));
    fs::create_dir(&directory).expect("gate test directory must be creatable");

    let cargo_path = directory.join("cargo");
    fs::write(
        &cargo_path,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf 'cargo %s\n' "$*" >> "$GATE_LOG"
case "${1-}" in
  metadata)
    printf '%s\n' "$GATE_METADATA"
    ;;
  tree)
    printf '%s\n' 'suprnova-magnetar v0.1.0'
    ;;
esac
"#,
    )
    .expect("fake cargo must be writable");
    let mut permissions = fs::metadata(&cargo_path)
        .expect("fake cargo metadata must be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo_path, permissions).expect("fake cargo must be executable");

    let nextest_path = directory.join("cargo-nextest");
    fs::write(
        &nextest_path,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf 'nextest %s\n' "$*" >> "$GATE_LOG"
"#,
    )
    .expect("fake cargo-nextest must be writable");
    let mut permissions = fs::metadata(&nextest_path)
        .expect("fake cargo-nextest metadata must be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&nextest_path, permissions).expect("fake cargo-nextest must be executable");

    let log_path = directory.join("invocations.log");
    let existing_path = env::var_os("PATH").unwrap_or_default();
    let path =
        env::join_paths(std::iter::once(directory.clone()).chain(env::split_paths(&existing_path)))
            .expect("test PATH must be representable");
    let output = Command::new("bash")
        .env("GATE_LOG", &log_path)
        .arg(repository_path("scripts/gate.sh"))
        .env("GATE_METADATA", metadata)
        .env("MAGNETAR_POSTGRES_TEST_URL", "postgres://contract-test")
        .env("MAGNETAR_MYSQL_TEST_URL", "mysql://contract-test")
        .env("PATH", path)
        .output()
        .expect("gate entrypoint must execute");
    let invocations = fs::read_to_string(&log_path).expect("gate log must be readable");
    (output, invocations, directory)
}

#[cfg(unix)]
#[test]
fn gate_continues_when_concurrency_target_is_not_configured() {
    let (output, invocations, directory) = run_gate_with_metadata(
        r#"{"packages":[{"name":"suprnova-magnetar","features":{"default":["password"],"password":[],"email-verification":[]},"targets":[{"name":"magnetar","kind":["lib"]}]}]}"#,
    );
    assert!(
        output.status.success(),
        "gate failed without a concurrency target: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("Concurrency test target not yet configured; continuing."),
        "gate must explain why the concurrency target was skipped"
    );
    assert!(
        !invocations.contains("test concurrency"),
        "gate must not invoke the concurrency test target when it is absent"
    );
    fs::remove_dir_all(directory).expect("gate test directory must be removable");
}

#[cfg(unix)]
#[test]
fn gate_runs_configured_concurrency_target() {
    let (output, invocations, directory) = run_gate_with_metadata(
        r#"{"packages":[{"name":"suprnova-magnetar","features":{"default":["password"],"password":[],"email-verification":[]},"targets":[{"name":"magnetar","kind":["lib"]},{"name":"concurrency","kind":["test"]}]}]}"#,
    );
    assert!(
        output.status.success(),
        "gate failed with a concurrency target: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        invocations.contains("nextest run --profile ci --test concurrency --all-features"),
        "gate must invoke the configured concurrency test target"
    );
    fs::remove_dir_all(directory).expect("gate test directory must be removable");
}

#[test]
fn scripts_and_hook_pass_shell_syntax() {
    for relative in [
        "scripts/gate.sh",
        "scripts/check-feature-matrix.sh",
        ".githooks/pre-push",
    ] {
        let status = Command::new("bash")
            .args(["-n", entrypoint_path(relative).to_str().unwrap()])
            .status()
            .expect("shell syntax check must execute");
        assert!(status.success(), "{relative} must pass bash -n");
    }
}

#[test]
fn gate_requires_each_backend_url() {
    for (postgres_url, mysql_url, expected) in [
        (
            None,
            Some("mysql://contract-test"),
            "MAGNETAR_POSTGRES_TEST_URL",
        ),
        (
            Some("postgres://contract-test"),
            None,
            "MAGNETAR_MYSQL_TEST_URL",
        ),
    ] {
        let output = run_script("scripts/gate.sh", postgres_url, mysql_url);
        assert!(
            !output.status.success(),
            "gate must fail without {expected}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "gate failure must identify {expected}"
        );
    }
}

#[test]
fn pre_push_delegates_the_workspace_gate() {
    let hook = read_entrypoint(".githooks/pre-push");
    for marker in ["set -euo pipefail", "exec", "scripts/gate.sh"] {
        assert!(
            hook.contains(marker),
            "pre-push hook is missing marker: {marker}"
        );
    }
}
