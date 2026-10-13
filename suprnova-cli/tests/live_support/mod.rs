//! Shared fixtures for the Live CLI tests: envelope builders for the helper
//! protocol and a fake application console that replays a scripted stream.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest as _, Sha256};

pub const IDENTITY: &str = "suprnova-live-0.1.0-0123456789abcdef";
pub const FRAMEWORK: &str = "1.3.7";
pub const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

pub fn envelope(sequence: u32, operation: &str, body: &str) -> String {
    envelope_with(sequence, operation, FRAMEWORK, Some(IDENTITY), body)
}

pub fn envelope_with(
    sequence: u32,
    operation: &str,
    framework: &str,
    assets: Option<&str>,
    body: &str,
) -> String {
    let assets = match assets {
        Some(identity) => format!("\"{identity}\""),
        None => "null".to_owned(),
    };
    format!(
        "{{\"protocol\":2,\"sequence\":{sequence},\"operation\":\"{operation}\",\"framework\":\"{framework}\",\"assets\":{assets},\"body\":{body}}}\n"
    )
}

pub fn begin(operation: &str) -> String {
    envelope(0, operation, "{\"kind\":\"begin\"}")
}

pub fn end_ok(sequence: u32, operation: &str) -> String {
    envelope(
        sequence,
        operation,
        "{\"kind\":\"end\",\"payload\":{\"status\":\"ok\",\"error\":null}}",
    )
}

pub fn end_failed(sequence: u32, operation: &str, error: &str) -> String {
    envelope(
        sequence,
        operation,
        &format!(
            "{{\"kind\":\"end\",\"payload\":{{\"status\":\"failed\",\"error\":\"{error}\"}}}}"
        ),
    )
}

pub fn diagnostic(
    sequence: u32,
    component: &str,
    view: &str,
    code: &str,
    severity: &str,
    line: u32,
    column: u32,
) -> String {
    envelope(
        sequence,
        "check",
        &format!(
            "{{\"kind\":\"diagnostic\",\"payload\":{{\"component\":\"{component}\",\"view\":\"{view}\",\"code\":\"{code}\",\"severity\":\"{severity}\",\"line\":{line},\"column\":{column}}}}}"
        ),
    )
}

pub fn summary(
    sequence: u32,
    components: u32,
    proved: u32,
    errors: u32,
    unproved: u32,
    template_files: u32,
) -> String {
    envelope(
        sequence,
        "check",
        &format!(
            "{{\"kind\":\"summary\",\"payload\":{{\"registry_bound\":true,\"components\":{components},\"proved\":{proved},\"errors\":{errors},\"unproved\":{unproved},\"template_files\":{template_files}}}}}"
        ),
    )
}

/// The limits the protocol 2 runtime report carries.
pub const LIMITS_JSON: &str = "[{\"setting\":\"LIVE_MAX_REQUEST_BYTES\",\"value\":16777216,\"unit\":\"bytes\"},{\"setting\":\"LIVE_MORPH_MAX_KEYS\",\"value\":1000000,\"unit\":\"keyed elements\"},{\"setting\":\"LIVE_UPLOAD_MAX_FILE_BYTES\",\"value\":1073741824,\"unit\":\"bytes\"}]";

pub fn runtime(sequence: u32) -> String {
    envelope(
        sequence,
        "inspect",
        &format!(
            "{{\"kind\":\"runtime\",\"payload\":{{\"registry_bound\":true,\"components\":1,\"config\":{{\"limits\":{LIMITS_JSON}}},\"upload_host\":{{\"installed\":false,\"finalizer\":false,\"direct_provider\":false,\"scanner\":false,\"application_validator\":false}},\"runtime_bound\":true,\"readiness\":{{\"clock\":true,\"random\":true,\"key_ring\":true,\"ledger\":true,\"promotion\":true,\"execution\":true,\"context_validator\":true,\"host_ports\":true,\"upload_ports\":true,\"upload_services\":true,\"mount_catalog\":true,\"response_and_cancellation\":true,\"subscription_ports\":true,\"async_state\":true}},\"asset_identity\":\"{IDENTITY}\",\"browser_runtime_version\":\"0.1.0\",\"runtime_contract_version\":1,\"protocol_versions\":[1,2]}}}}"
        ),
    )
}

pub fn component(sequence: u32, name: &str, view: &str) -> String {
    envelope(
        sequence,
        "inspect",
        &format!(
            "{{\"kind\":\"component\",\"payload\":{{\"name\":\"{name}\",\"view\":\"{view}\",\"component_version\":1,\"state_schema_version\":1,\"action_schema_version\":1,\"checker_contract_version\":1,\"minimum_protocol\":1,\"fields\":1,\"upload_fields\":0,\"actions\":1,\"events\":0,\"effects\":0,\"subscriptions\":0,\"refresh_on_promote\":false,\"contract_digest\":\"0123456789abcdef\"}}}}"
        ),
    )
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn sri(bytes: &[u8]) -> String {
    format!("sha256-{}", BASE64.encode(Sha256::digest(bytes)))
}

pub fn asset_body(kind: &str, file: &str, content_type: &str, bytes: &[u8]) -> String {
    format!(
        "{{\"kind\":\"asset\",\"payload\":{{\"kind\":\"{kind}\",\"file\":\"{file}\",\"bytes\":{},\"sha256\":\"{}\",\"sri\":\"{}\",\"content_type\":\"{content_type}\",\"content\":\"{}\"}}}}",
        bytes.len(),
        sha256_hex(bytes),
        sri(bytes),
        BASE64.encode(bytes)
    )
}

pub fn asset(sequence: u32, kind: &str, file: &str, content_type: &str, bytes: &[u8]) -> String {
    envelope(
        sequence,
        "assets",
        &asset_body(kind, file, content_type, bytes),
    )
}

pub const ASSET_FILES: [(&str, &str, &[u8]); 3] = [
    (
        "manifest",
        "suprnova-live.assets.json",
        b"{\"schema_version\":2}\n",
    ),
    (
        "artifact",
        "suprnova-live.esm.js",
        b"export const live = 1;\n",
    ),
    ("boot", "suprnova-live.boot.esm.js", b"boot();\n"),
];

pub fn asset_stream() -> String {
    let mut stream = begin("assets");
    let mut sequence = 1;
    for (kind, file, bytes) in ASSET_FILES {
        let content_type = if kind == "manifest" {
            "application/json"
        } else {
            "text/javascript; charset=utf-8"
        };
        stream.push_str(&asset(sequence, kind, file, content_type, bytes));
        sequence += 1;
    }
    stream.push_str(&end_ok(sequence, "assets"));
    stream
}

pub fn check_stream(diagnostics: &[(&str, &str, &str, &str, u32, u32)], proved: u32) -> String {
    let mut stream = begin("check");
    let mut sequence = 1;
    let mut errors = 0;
    let mut unproved = 0;
    for (component, view, code, severity, line, column) in diagnostics {
        stream.push_str(&diagnostic(
            sequence, component, view, code, severity, *line, *column,
        ));
        sequence += 1;
        if *severity == "error" {
            errors += 1;
        } else {
            unproved += 1;
        }
    }
    stream.push_str(&summary(sequence, 2, proved, errors, unproved, 2));
    stream.push_str(&end_ok(sequence + 1, "check"));
    stream
}

pub fn inspect_stream() -> String {
    let mut stream = begin("inspect");
    stream.push_str(&runtime(1));
    stream.push_str(&component(2, "demo.counter", "live/counter.html"));
    stream.push_str(&end_ok(3, "inspect"));
    stream
}

const FAKE_CONSOLE: &str = r#"//! Fake application console replaying a scripted Live tooling stream.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("__suprnova:live-tool") {
        eprintln!("error: unrecognized subcommand");
        std::process::exit(1);
    }
    // A script per protocol, when the test gives one, answers the protocol
    // the CLI asked for, the way a helper of one framework version would.
    let protocol = args
        .iter()
        .position(|arg| arg == "--protocol")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_default();
    let output = std::env::var(format!("FAKE_LIVE_TOOL_OUTPUT_PROTOCOL_{protocol}"))
        .or_else(|_| std::env::var("FAKE_LIVE_TOOL_OUTPUT"))
        .expect("FAKE_LIVE_TOOL_OUTPUT");
    let text = std::fs::read(output).expect("scripted output");
    use std::io::Write as _;
    std::io::stdout().write_all(&text).expect("stdout");
    std::io::stdout().flush().expect("flush");
    eprintln!("fake console: replayed {} bytes", text.len());
    let code: i32 = std::env::var("FAKE_LIVE_TOOL_EXIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    std::process::exit(code);
}
"#;

/// A fake application project with a `console` binary, shared by every test in
/// the process; it is built once by the first `cargo run` and reused after.
pub fn fake_project() -> &'static Path {
    static PROJECT: OnceLock<PathBuf> = OnceLock::new();
    PROJECT
        .get_or_init(|| {
            // Under this package's own target directory, not the system temp
            // dir: the project outlives every test in the process, so it is
            // kept, and a kept directory in `/tmp` is one more leak per run.
            // `cargo clean` removes these.
            let dir = tempfile::Builder::new()
                .prefix("suprnova-live-fake-app-")
                .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
                .expect("tempdir");
            let root = dir.keep();
            fs::write(
                root.join("Cargo.toml"),
                "[package]\nname = \"fake_app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"console\"\npath = \"src/bin/console.rs\"\n\n[workspace]\n",
            )
            .expect("manifest");
            fs::create_dir_all(root.join("src/bin")).expect("src/bin");
            fs::write(root.join("src/bin/console.rs"), FAKE_CONSOLE).expect("console");
            fs::write(root.join("src/lib.rs"), "//! fake\n").expect("lib");
            fs::create_dir_all(root.join("templates/live")).expect("templates");
            fs::write(root.join("templates/live/counter.html"), "<p>x</p>\n").expect("view");
            root
        })
        .as_path()
}

/// Runs the CLI inside the fake project with a scripted helper stream.
pub fn run_cli(args: &[&str], script: &str, exit: i32) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let project = fake_project();
    let index = COUNTER.fetch_add(1, Ordering::SeqCst);
    let script_path = project.join(format!(".fake-script-{}-{index}.jsonl", std::process::id()));
    fs::write(&script_path, script).expect("script");
    let output = Command::new(BIN)
        .args(args)
        .current_dir(project)
        .env("FAKE_LIVE_TOOL_OUTPUT", &script_path)
        .env("FAKE_LIVE_TOOL_EXIT", exit.to_string())
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("suprnova binary spawnable");
    let _ = fs::remove_file(&script_path);
    output
}

/// Runs the CLI with one scripted helper stream per protocol it may ask for,
/// so a test plays a helper from another framework version.
pub fn run_cli_by_protocol(args: &[&str], scripts: &[(u16, &str)]) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let project = fake_project();
    let index = COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut command = Command::new(BIN);
    command
        .args(args)
        .current_dir(project)
        .env("FAKE_LIVE_TOOL_EXIT", "0")
        .env_remove("FAKE_LIVE_TOOL_OUTPUT")
        .env_remove("CARGO_TARGET_DIR");
    let mut paths = Vec::new();
    for (protocol, script) in scripts {
        let path = project.join(format!(
            ".fake-script-{}-v{index}-p{protocol}.jsonl",
            std::process::id()
        ));
        fs::write(&path, script).expect("script");
        command.env(format!("FAKE_LIVE_TOOL_OUTPUT_PROTOCOL_{protocol}"), &path);
        paths.push(path);
    }
    let output = command.output().expect("suprnova binary spawnable");
    for path in paths {
        let _ = fs::remove_file(path);
    }
    output
}

/// A whole stream in protocol 1, as a helper from before protocol 2 writes it.
pub fn in_protocol_one(stream: &str) -> String {
    stream.replace("\"protocol\":2", "\"protocol\":1")
}

/// The inspect stream a protocol 1 helper writes: three named limits.
pub fn inspect_stream_protocol_one() -> String {
    in_protocol_one(&inspect_stream()).replace(
        &format!("\"config\":{{\"limits\":{}}}", LIMITS_JSON),
        "\"config\":{\"max_request_bytes\":1048576,\"max_response_bytes\":1048576,\"max_context_lifetime_ms\":30000}",
    )
}

/// What a protocol 1 helper answers when asked for protocol 2.
pub fn refusal_protocol_one(operation: &str) -> String {
    in_protocol_one(&format!(
        "{}{}",
        begin(operation),
        end_failed(1, operation, "live_tooling_unsupported_protocol")
    ))
}

pub fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
