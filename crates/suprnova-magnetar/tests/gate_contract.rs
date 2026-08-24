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

const LIVE_DATABASE_IGNORE_REASON: &str = "requires T2 live Postgres/MySQL database";

#[derive(Debug, PartialEq, Eq)]
struct LiveDatabaseTest {
    relative: String,
    name: String,
    has_ignore: bool,
    ignore_reason: Option<String>,
}

fn mask_non_code(source: &str) -> Vec<u8> {
    fn blank(masked: &mut [u8], start: usize, end: usize) {
        for byte in &mut masked[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }

    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            let end = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset);
            blank(&mut masked, index, end);
            index = end;
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            let mut depth = 1;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            blank(&mut masked, start, index);
            continue;
        }

        let raw_start = if bytes[index] == b'r' {
            Some(index)
        } else if bytes[index..].starts_with(b"br") {
            Some(index + 1)
        } else {
            None
        };
        if let Some(raw_start) = raw_start {
            let mut quote = raw_start + 1;
            while quote < bytes.len() && bytes[quote] == b'#' {
                quote += 1;
            }
            if quote < bytes.len() && bytes[quote] == b'"' {
                let hashes = quote - raw_start - 1;
                let mut end = quote + 1;
                while end < bytes.len() {
                    if bytes[end] == b'"'
                        && bytes
                            .get(end + 1..end + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                    {
                        end += 1 + hashes;
                        break;
                    }
                    end += 1;
                }
                blank(&mut masked, index, end);
                index = end;
                continue;
            }
        }

        if bytes[index] == b'"' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            blank(&mut masked, start, index);
            continue;
        }

        index += 1;
    }
    masked
}

fn live_database_tests_from_source(relative: &str, source: &str) -> Vec<LiveDatabaseTest> {
    let masked = mask_non_code(source);
    let code = std::str::from_utf8(&masked).expect("masked Rust source must remain UTF-8");
    let mut inventory = Vec::new();
    let mut cursor = 0;

    while let Some(offset) = code[cursor..].find("async fn ") {
        let function_start = cursor + offset;
        let name_start = function_start + "async fn ".len();
        let name_end = code[name_start..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .map_or(code.len(), |offset| name_start + offset);
        let name = &source[name_start..name_end];
        let Some(opening_offset) = code[name_end..].find('{') else {
            break;
        };
        let opening = name_end + opening_offset;
        let mut depth = 0;
        let mut closing = None;
        for (offset, byte) in masked[opening..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        closing = Some(opening + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let closing = closing.unwrap_or_else(|| panic!("{relative}::{name} has an unclosed body"));
        let declaration_start = source[..function_start]
            .rfind("\n\n")
            .map_or(0, |position| position + 2);
        let attributes = &source[declaration_start..function_start];
        let body = &source[opening..=closing];
        if attributes.contains("#[tokio::test]")
            && (body.contains("MAGNETAR_POSTGRES_TEST_URL")
                || body.contains("MAGNETAR_MYSQL_TEST_URL"))
        {
            let ignore_reason = attributes
                .lines()
                .find_map(|line| line.trim().strip_prefix("#[ignore = \""))
                .and_then(|reason| reason.strip_suffix("\"]"))
                .map(str::to_owned);
            inventory.push(LiveDatabaseTest {
                relative: relative.to_owned(),
                name: name.to_owned(),
                has_ignore: ignore_reason.is_some()
                    || attributes
                        .lines()
                        .any(|line| line.trim().starts_with("#[ignore]")),
                ignore_reason,
            });
        }
        cursor = closing + 1;
    }

    inventory
}

fn rust_source_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
    {
        let path = entry
            .expect("source directory entry must be readable")
            .path();
        if path.is_dir() {
            rust_source_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn live_database_tests_from_repository() -> Vec<LiveDatabaseTest> {
    let manifest_dir = repository_path("");
    let mut inventory = Vec::new();
    for root in ["tests", "src"] {
        let mut files = Vec::new();
        rust_source_files(&repository_path(root), &mut files);
        files.sort();
        for path in files {
            let relative = path
                .strip_prefix(&manifest_dir)
                .expect("source file must be under the crate root")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            inventory.extend(live_database_tests_from_source(&relative, &source));
        }
    }
    inventory
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

const LIVE_TEST_INVOCATIONS: [&str; 10] = [
    "run_live_test test default_schema_backends postgres_default_schema_is_replay_safe",
    "run_live_test test default_schema_backends postgres_api_import_advances_the_default_user_sequence",
    "run_live_test test default_schema_backends mysql_default_schema_is_replay_safe",
    "run_live_test test foundation_gate postgres_backend_is_reachable",
    "run_live_test test foundation_gate mysql_backend_is_reachable",
    "run_live_test test storage_tokens configured_postgres_target_is_required",
    "run_live_test test storage_tokens configured_mysql_target_is_required",
    "run_live_test test token_broker_concurrency two_pod_convergence_postgres",
    "run_live_test test token_broker_concurrency two_pod_convergence_mysql",
    "run_live_test lib _ migration::mysql_swap_tests::plan_bound_coordinator_revalidates_imports_swaps_cleans_and_releases_barrier",
];

const LIVE_TEST_LIVE_COMMANDS: [&str; 10] = [
    "cargo test --test default_schema_backends --all-features postgres_default_schema_is_replay_safe -- --ignored --exact",
    "cargo test --test default_schema_backends --all-features postgres_api_import_advances_the_default_user_sequence -- --ignored --exact",
    "cargo test --test default_schema_backends --all-features mysql_default_schema_is_replay_safe -- --ignored --exact",
    "cargo test --test foundation_gate --all-features postgres_backend_is_reachable -- --ignored --exact",
    "cargo test --test foundation_gate --all-features mysql_backend_is_reachable -- --ignored --exact",
    "cargo test --test storage_tokens --all-features configured_postgres_target_is_required -- --ignored --exact",
    "cargo test --test storage_tokens --all-features configured_mysql_target_is_required -- --ignored --exact",
    "cargo test --test token_broker_concurrency --all-features two_pod_convergence_postgres -- --ignored --exact",
    "cargo test --test token_broker_concurrency --all-features two_pod_convergence_mysql -- --ignored --exact",
    "cargo test --lib --all-features migration::mysql_swap_tests::plan_bound_coordinator_revalidates_imports_swaps_cleans_and_releases_barrier -- --ignored --exact",
];

const LIVE_DATABASE_QUALIFICATION_TESTS: [(&str, &str); 10] = [
    (
        "tests/default_schema_backends.rs",
        "postgres_default_schema_is_replay_safe",
    ),
    (
        "tests/default_schema_backends.rs",
        "postgres_api_import_advances_the_default_user_sequence",
    ),
    (
        "tests/default_schema_backends.rs",
        "mysql_default_schema_is_replay_safe",
    ),
    ("tests/foundation_gate.rs", "postgres_backend_is_reachable"),
    ("tests/foundation_gate.rs", "mysql_backend_is_reachable"),
    (
        "tests/storage_tokens.rs",
        "configured_postgres_target_is_required",
    ),
    (
        "tests/storage_tokens.rs",
        "configured_mysql_target_is_required",
    ),
    (
        "tests/token_broker_concurrency.rs",
        "two_pod_convergence_postgres",
    ),
    (
        "tests/token_broker_concurrency.rs",
        "two_pod_convergence_mysql",
    ),
    (
        "src/migration/mysql_swap_tests.rs",
        "plan_bound_coordinator_revalidates_imports_swaps_cleans_and_releases_barrier",
    ),
];

fn parse_live_test_invocations(script: &str) -> Vec<String> {
    script
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("run_live_test "))
        .map(|invocation| format!("run_live_test {invocation}"))
        .collect()
}

fn live_invocation_registers_test(invocation: &str, test_name: &str) -> bool {
    invocation
        .split_ascii_whitespace()
        .next_back()
        .is_some_and(|registered| {
            registered == test_name || registered.ends_with(&format!("::{test_name}"))
        })
}

fn assert_exact_live_test_invocations(script: &str) {
    let invocations = parse_live_test_invocations(script);
    assert_eq!(
        invocations.len(),
        LIVE_TEST_INVOCATIONS.len(),
        "live tests must be exactly 10"
    );
    for (index, expected) in LIVE_TEST_INVOCATIONS.iter().enumerate() {
        assert_eq!(
            &invocations[index], expected,
            "unexpected live invocation order at index {index}"
        );
    }
}

fn assert_exact_live_test_commands(invocations: &str) {
    let commands = invocations
        .lines()
        .filter(|invocation| invocation.starts_with("cargo test "))
        .collect::<Vec<_>>();
    assert_eq!(
        commands.as_slice(),
        LIVE_TEST_LIVE_COMMANDS.as_slice(),
        "live gate Cargo test commands must exactly match registry"
    );
}

#[cfg(unix)]
fn run_live_gate_with_metadata(metadata: &str) -> (Output, String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let directory = env::temp_dir().join(format!("magnetar-live-gate-{unique}"));
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
    printf '%s\n' 'suprnova-magnetar v1.2.4'
    printf '%s\n' 'sea-orm v2.0.2'
    printf '%s\n' 'sea-query v1.0.2'
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
        .env("GATE_LOG", &log_path)
        .args([
            repository_path("scripts/gate.sh").to_str().unwrap(),
            "--live",
        ])
        .env("GATE_METADATA", metadata)
        .env("MAGNETAR_POSTGRES_TEST_URL", "postgres://contract")
        .env("MAGNETAR_MYSQL_TEST_URL", "mysql://contract")
        .env("PATH", path)
        .output()
        .expect("live gate entrypoint must execute");
    let invocations = fs::read_to_string(&log_path).expect("gate log must be readable");
    (output, invocations, directory)
}

#[cfg(not(unix))]
fn run_live_gate_with_metadata(_metadata: &str) -> (Output, String, PathBuf) {
    unreachable!("live gate contract is unix-only");
}

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
fn every_live_database_test_is_ignored_and_registered() {
    let gate = read_entrypoint("scripts/gate.sh");
    let inventory = live_database_tests_from_repository();
    let expected = LIVE_DATABASE_QUALIFICATION_TESTS
        .into_iter()
        .map(|(relative, name)| LiveDatabaseTest {
            relative: relative.to_owned(),
            name: name.to_owned(),
            has_ignore: true,
            ignore_reason: Some(LIVE_DATABASE_IGNORE_REASON.to_owned()),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        inventory, expected,
        "source-discovered live database test inventory must remain the expected ten"
    );

    let (output, invocations, directory) = run_live_gate_with_metadata(
        r#"{"packages":[{"name":"suprnova-magnetar","features":{"default":["password"],"password":[],"email-verification":[]},"targets":[{"name":"magnetar","kind":["lib"]}]}]}"#,
    );
    assert!(
        output.status.success(),
        "live gate command must run with required live URLs configured: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_exact_live_test_commands(&invocations);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let registrations = parse_live_test_invocations(&gate);
    for test in inventory {
        assert!(
            test.has_ignore,
            "{}::{} must remain #[ignore]",
            test.relative, test.name
        );
        let registration = registrations
            .iter()
            .find(|invocation| live_invocation_registers_test(invocation, &test.name))
            .unwrap_or_else(|| {
                panic!(
                    "{}::{} must be registered by --live",
                    test.relative, test.name
                )
            });
        let expected_log = format!(
            "Running live test: {}",
            registration
                .strip_prefix("run_live_test ")
                .expect("live registration must use run_live_test")
        );
        assert!(
            stdout.lines().any(|line| line == expected_log),
            "{}::{} must emit the stable log line `{expected_log}`",
            test.relative,
            test.name
        );
    }
    fs::remove_dir_all(directory).expect("gate test directory must be removable");
}

#[test]
fn live_test_command_registry_is_exact() {
    let gate = read_entrypoint("scripts/gate.sh");
    assert_exact_live_test_invocations(&gate);
}

#[test]
fn source_scanner_detects_an_extra_unregistered_url_requiring_test() {
    let source = r#"
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn registered_live_test() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL").expect("required");
    connect(url).await;
}

#[tokio::test]
async fn extra_unregistered_live_test() {
    let url = std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("required");
    connect(url).await;
}
"#;

    let inventory = live_database_tests_from_source("tests/synthetic.rs", source);
    assert_eq!(
        inventory
            .iter()
            .map(|test| test.name.as_str())
            .collect::<Vec<_>>(),
        ["registered_live_test", "extra_unregistered_live_test"]
    );
    let extra = &inventory[1];
    assert!(
        !extra.has_ignore,
        "the extra test must be detected as non-ignored"
    );
    assert!(
        LIVE_TEST_INVOCATIONS
            .iter()
            .all(|invocation| !live_invocation_registers_test(invocation, &extra.name)),
        "the extra test must be detected as unregistered"
    );
}

#[test]
#[should_panic(expected = "live gate Cargo test commands must exactly match registry")]
fn live_command_execution_contract_rejects_wrong_target_kind() {
    let mutated = LIVE_TEST_LIVE_COMMANDS.join("\n").replacen(
        "cargo test --test default_schema_backends",
        "cargo test --lib",
        1,
    );
    assert_exact_live_test_commands(&mutated);
}

#[test]
fn live_database_contract_is_mutation_detectable() {
    let gate = read_entrypoint("scripts/gate.sh");
    let mutated = gate.replace(
        "run_live_test test token_broker_concurrency two_pod_convergence_mysql\n",
        "",
    );
    assert_ne!(
        parse_live_test_invocations(&mutated),
        LIVE_TEST_INVOCATIONS,
        "omitted live invocation should break the live contract"
    );
}

#[test]
fn gate_contains_required_checks_and_delegates_feature_matrix_with_live_env_gating() {
    let gate = read_entrypoint("scripts/gate.sh");
    let (default_gate, live_gate) = gate
        .split_once("if [[ \"${1-}\" == \"--live\" ]]")
        .expect("gate script should define a live mode guard");

    for marker in [
        "set -euo pipefail",
        "cargo check --all-targets --all-features",
        "cargo fmt --all -- --check",
        "cargo clippy --all-targets --all-features",
        "cargo nextest run --profile ci --all-features",
        "cargo test --doc --all-features",
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
    for marker in ["MAGNETAR_POSTGRES_TEST_URL", "MAGNETAR_MYSQL_TEST_URL"] {
        assert!(
            !default_gate.contains(marker),
            "the default gate must not require a live database: {marker}"
        );
        assert!(
            live_gate.contains(marker),
            "the live gate path must require live database URLs: {marker}"
        );
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
        toolchain.contains("channel = \"1.94.0\""),
        "the Rust toolchain channel must be pinned to 1.94.0"
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
    let (output, invocations, directory) = run_feature_matrix_with_tree(concat!(
        "suprnova-magnetar v1.2.4\n",
        "sea-orm v2.0.2\n",
        "sea-query v1.0.2\n",
        "sqlx v0.9.0\n",
        "sqlx-core v0.9.0\n",
        "sqlx-sqlite v0.9.0\n",
    ));
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
fn feature_gate_rejects_old_database_major_lines() {
    let tree = concat!(
        "suprnova-magnetar v1.2.4\n",
        "sea-orm v2.0.2\nsea-query v1.0.2\nsqlx v0.9.0\n",
        "sqlx-core v0.9.0\nsqlx-sqlite v0.9.0\n",
        "sea-orm v1.1.20\nsea-query v0.32.7\nsqlx-core v0.8.6\n",
    );
    let (output, _, directory) = run_feature_matrix_with_tree(tree);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsupported database dependency"));
    fs::remove_dir_all(directory).unwrap();
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
    printf '%s\n' 'suprnova-magnetar v1.2.4'
    printf '%s\n' 'sea-orm v2.0.2'
    printf '%s\n' 'sea-query v1.0.2'
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
fn pre_push_delegates_the_workspace_gate() {
    let hook = read_entrypoint(".githooks/pre-push");
    for marker in ["set -euo pipefail", "exec", "scripts/gate.sh"] {
        assert!(
            hook.contains(marker),
            "pre-push hook is missing marker: {marker}"
        );
    }
}
