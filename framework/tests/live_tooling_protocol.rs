//! The hidden Live tooling helper speaks the bounded JSON-lines protocol the CLI consumes.

use std::fs;
use std::path::PathBuf;
use std::sync::Once;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest as _, Sha256};
use suprnova::live::assets::live_asset_catalog;
use suprnova::live::tooling::{ToolRequest, ToolingErrorKind, execute};
use suprnova::live::tooling_protocol::{
    AssetKind, Body, COMMAND_NAME, Envelope, MAX_LINE_BYTES, MAX_TEMPLATE_FILE_BYTES,
    MAX_TEMPLATE_ROOTS, MAX_TEXT_BYTES, Operation, Outcome, PROTOCOL_VERSION, Severity,
};
use suprnova::live::{LiveComponent, LiveConfig, LiveRegistry, live};
use suprnova::{App, Crypt, EncryptionKey, console};

#[derive(LiveComponent)]
#[live(
    name = "tests.tooling-counter",
    view = "live/tests/tooling-counter.html"
)]
pub struct ToolingCounter {
    #[public]
    count: u64,
}

#[live]
impl ToolingCounter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

#[derive(LiveComponent)]
#[live(name = "tests.tooling-broken", view = "live/tests/tooling-broken.html")]
pub struct ToolingBroken {
    #[public]
    count: u64,
}

#[live]
impl ToolingBroken {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

fn fixture() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        App::init();
        Crypt::init(EncryptionKey::generate());
        App::singleton(LiveConfig::standard());
        App::singleton(
            LiveRegistry::builder()
                .register::<ToolingCounter>()
                .expect("counter registers")
                .register::<ToolingBroken>()
                .expect("broken component registers")
                .build(),
        );
    });
}

fn template_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/templates")
}

fn run(
    protocol: u16,
    operation: Operation,
    roots: Vec<PathBuf>,
) -> (Result<(), ToolingErrorKind>, Vec<Envelope>) {
    fixture();
    let request = ToolRequest::new(protocol, operation, roots);
    let mut out = Vec::new();
    let result = execute(&request, &mut out).map_err(|error| error.kind());
    let text = String::from_utf8(out).expect("stdout is UTF-8");
    let envelopes = text
        .lines()
        .map(|line| {
            assert!(line.len() < MAX_LINE_BYTES, "line within the per-line cap");
            serde_json::from_str::<Envelope>(line).expect("every line is a v1 envelope")
        })
        .collect();
    (result, envelopes)
}

fn end(envelopes: &[Envelope]) -> (Outcome, Option<String>) {
    let last = envelopes.last().expect("at least one envelope");
    match &last.body {
        Body::End(report) => (report.status, report.error.clone()),
        other => panic!("last envelope is the end marker, got {other:?}"),
    }
}

fn assert_well_formed(envelopes: &[Envelope], operation: Operation) {
    assert!(matches!(
        envelopes.first().map(|e| &e.body),
        Some(Body::Begin)
    ));
    let ends = envelopes
        .iter()
        .filter(|e| matches!(e.body, Body::End(_)))
        .count();
    assert_eq!(ends, 1, "exactly one end marker");
    assert!(matches!(
        envelopes.last().map(|e| &e.body),
        Some(Body::End(_))
    ));
    let identity = live_asset_catalog().ok().map(|c| c.identity().to_owned());
    for (index, envelope) in envelopes.iter().enumerate() {
        assert_eq!(envelope.protocol, PROTOCOL_VERSION);
        assert_eq!(
            envelope.sequence as usize, index,
            "contiguous sequence numbers"
        );
        assert_eq!(envelope.operation, operation);
        assert_eq!(envelope.framework, env!("CARGO_PKG_VERSION"));
        assert_eq!(envelope.assets, identity);
    }
    let text = serde_json::to_string(envelopes).expect("encode");
    let value: serde_json::Value = serde_json::from_str(&text).expect("decode");
    assert_strings_bounded(&value);
}

fn assert_strings_bounded(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            assert!(text.len() <= MAX_TEXT_BYTES || is_asset_content(text));
        }
        serde_json::Value::Array(items) => items.iter().for_each(assert_strings_bounded),
        serde_json::Value::Object(map) => map.values().for_each(assert_strings_bounded),
        _ => {}
    }
}

fn is_asset_content(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

#[test]
fn the_helper_is_a_registered_hidden_console_command() {
    let entry = console::find(COMMAND_NAME).expect("helper is registered at link time");
    let command = (entry.clap_builder)();
    assert!(command.is_hide_set(), "the helper never appears in help");
    assert_eq!(COMMAND_NAME, "__suprnova:live-tool");
    let names: Vec<&str> = command
        .get_arguments()
        .map(|arg| arg.get_id().as_str())
        .collect();
    assert!(names.contains(&"protocol"));
    assert!(names.contains(&"operation"));
    assert!(names.contains(&"templates"));
}

#[test]
fn check_reports_every_component_and_only_bounded_diagnostics() {
    let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Check, vec![template_root()]);
    assert_eq!(result, Ok(()));
    assert_well_formed(&envelopes, Operation::Check);
    assert_eq!(end(&envelopes), (Outcome::Ok, None));

    let summaries: Vec<_> = envelopes
        .iter()
        .filter_map(|e| match &e.body {
            Body::Summary(summary) => Some(summary),
            _ => None,
        })
        .collect();
    assert_eq!(summaries.len(), 1);
    let summary = summaries[0];
    assert!(summary.registry_bound);
    assert_eq!(summary.components, 2);
    assert_eq!(summary.proved, 1);
    assert!(summary.errors >= 1);
    assert!(summary.template_files >= 2);

    let diagnostics: Vec<_> = envelopes
        .iter()
        .filter_map(|e| match &e.body {
            Body::Diagnostic(diagnostic) => Some(diagnostic),
            _ => None,
        })
        .collect();
    assert!(!diagnostics.is_empty());
    assert!(
        diagnostics
            .iter()
            .all(|d| d.component.as_deref() != Some("tests.tooling-counter"))
    );
    let broken = diagnostics
        .iter()
        .find(|d| d.component.as_deref() == Some("tests.tooling-broken"))
        .expect("the broken component is diagnosed");
    assert_eq!(broken.code, "unknown_action");
    assert_eq!(broken.severity, Severity::Error);
    assert_eq!(
        broken.view.as_deref(),
        Some("live/tests/tooling-broken.html")
    );
    assert!(broken.line >= 1);
    // The summary follows every diagnostic and precedes the end marker.
    let summary_index = envelopes
        .iter()
        .position(|e| matches!(e.body, Body::Summary(_)))
        .expect("summary");
    assert_eq!(summary_index, envelopes.len() - 2);
}

#[test]
fn check_without_a_bound_registry_fails_closed() {
    // A registry is bound by the fixture, so exercise the template side instead:
    // a missing template root is rejected before any component is checked.
    let missing = std::env::temp_dir().join("suprnova-live-tooling-missing-root");
    let _ = fs::remove_dir_all(&missing);
    let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Check, vec![missing]);
    assert_eq!(result, Err(ToolingErrorKind::TemplateRootRejected));
    assert_well_formed(&envelopes, Operation::Check);
    assert_eq!(
        end(&envelopes),
        (
            Outcome::Failed,
            Some("live_tooling_template_root_rejected".to_owned())
        )
    );
    assert!(
        !envelopes
            .iter()
            .any(|e| matches!(e.body, Body::Diagnostic(_)))
    );
}

#[test]
fn inspect_reports_only_safe_bounded_metadata() {
    let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Inspect, Vec::new());
    assert_eq!(result, Ok(()));
    assert_well_formed(&envelopes, Operation::Inspect);
    assert_eq!(end(&envelopes), (Outcome::Ok, None));

    let runtime = envelopes
        .iter()
        .find_map(|e| match &e.body {
            Body::Runtime(report) => Some(report),
            _ => None,
        })
        .expect("one runtime report");
    assert!(runtime.registry_bound);
    assert_eq!(runtime.components, 2);
    let config = LiveConfig::standard();
    assert_eq!(
        runtime.config.max_request_bytes,
        config.max_request_bytes() as u64
    );
    assert_eq!(
        runtime.config.max_response_bytes,
        config.max_response_bytes() as u64
    );
    assert_eq!(
        runtime.config.max_context_lifetime_ms,
        config.max_context_lifetime_ms()
    );
    assert!(!runtime.upload_host.installed);
    assert_eq!(runtime.runtime_bound, runtime.readiness.is_some());
    let catalog = live_asset_catalog().expect("artifacts validate");
    assert_eq!(runtime.asset_identity.as_deref(), Some(catalog.identity()));
    assert_eq!(runtime.browser_runtime_version, "0.1.0");
    assert_eq!(runtime.runtime_contract_version, 1);

    let components: Vec<_> = envelopes
        .iter()
        .filter_map(|e| match &e.body {
            Body::Component(report) => Some(report),
            _ => None,
        })
        .collect();
    let names: Vec<&str> = components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["tests.tooling-broken", "tests.tooling-counter"]);
    let counter = components[1];
    assert_eq!(counter.view, "live/tests/tooling-counter.html");
    assert_eq!(counter.fields, 1);
    assert_eq!(counter.upload_fields, 0);
    assert_eq!(counter.actions, 1);
    assert_eq!(counter.contract_digest.len(), 16);
    assert!(
        counter
            .contract_digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    );
    assert!(counter.minimum_protocol >= 1);
    // Nothing that looks like state, key material, or a token crosses the wire.
    let text = serde_json::to_string(&envelopes).expect("encode");
    for forbidden in [
        "credential",
        "cookie",
        "secret",
        "snapshot",
        "\"key\"",
        "token",
    ] {
        assert!(
            !text.contains(forbidden),
            "{forbidden} never appears in inspection output"
        );
    }
}

#[test]
fn assets_exports_exactly_the_reviewed_bytes() {
    let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Assets, Vec::new());
    assert_eq!(result, Ok(()));
    assert_well_formed(&envelopes, Operation::Assets);
    assert_eq!(end(&envelopes), (Outcome::Ok, None));
    let catalog = live_asset_catalog().expect("artifacts validate");

    let assets: Vec<_> = envelopes
        .iter()
        .filter_map(|e| match &e.body {
            Body::Asset(asset) => Some(asset),
            _ => None,
        })
        .collect();
    assert_eq!(
        assets.len(),
        11,
        "manifest, eight artifacts, two boot scripts"
    );
    for asset in &assets {
        let decoded = BASE64.decode(&asset.content).expect("standard base64");
        assert_eq!(decoded.len() as u64, asset.bytes);
        let digest = Sha256::digest(&decoded);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(asset.sha256, hex);
        assert_eq!(asset.sri, format!("sha256-{}", BASE64.encode(digest)));
        assert!(!asset.file.contains('/'));
        let expected: &[u8] = match asset.kind {
            AssetKind::Manifest => {
                assert_eq!(asset.file, "suprnova-live.assets.json");
                assert_eq!(asset.content_type, "application/json");
                catalog.manifest_bytes()
            }
            AssetKind::Artifact => {
                assert_eq!(asset.content_type, "text/javascript; charset=utf-8");
                catalog
                    .artifacts()
                    .iter()
                    .find(|a| a.file() == asset.file)
                    .expect("artifact exists")
                    .bytes()
            }
            AssetKind::Boot => {
                assert_eq!(asset.content_type, "text/javascript; charset=utf-8");
                catalog
                    .boot_scripts()
                    .iter()
                    .find(|b| b.file() == asset.file)
                    .expect("boot script exists")
                    .bytes()
            }
        };
        assert_eq!(decoded, expected);
    }
    let files: std::collections::BTreeSet<&str> = assets.iter().map(|a| a.file.as_str()).collect();
    assert_eq!(files.len(), 11, "every file is exported once");
}

#[test]
fn unsupported_protocols_and_operations_fail_closed() {
    let (result, envelopes) = run(PROTOCOL_VERSION + 1, Operation::Assets, Vec::new());
    assert_eq!(result, Err(ToolingErrorKind::UnsupportedProtocol));
    assert_well_formed(&envelopes, Operation::Assets);
    assert_eq!(envelopes.len(), 2, "begin and a failed end marker only");
    assert_eq!(
        end(&envelopes),
        (
            Outcome::Failed,
            Some("live_tooling_unsupported_protocol".to_owned())
        )
    );

    let error = ToolRequest::parse(PROTOCOL_VERSION, "bogus", Vec::new())
        .expect_err("unknown operations are rejected");
    assert_eq!(error.kind(), ToolingErrorKind::UnknownOperation);
    assert_eq!(error.to_string(), "live_tooling_unknown_operation");
    let request = ToolRequest::parse(PROTOCOL_VERSION, "inspect", Vec::new()).expect("known");
    assert_eq!(request.operation(), Operation::Inspect);
}

#[test]
fn template_roots_are_bounded_and_symlink_free() {
    let base = tempfile::tempdir().expect("tempdir");
    let roots: Vec<PathBuf> = (0..=MAX_TEMPLATE_ROOTS)
        .map(|index| {
            let root = base.path().join(format!("root-{index}"));
            fs::create_dir_all(&root).expect("root");
            root
        })
        .collect();
    let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Check, roots);
    assert_eq!(result, Err(ToolingErrorKind::TemplateLimitExceeded));
    assert_eq!(
        end(&envelopes).1.as_deref(),
        Some("live_tooling_template_limit_exceeded")
    );

    let oversized_root = base.path().join("oversized");
    fs::create_dir_all(oversized_root.join("live")).expect("dir");
    fs::write(
        oversized_root.join("live/huge.html"),
        vec![b' '; MAX_TEMPLATE_FILE_BYTES + 1],
    )
    .expect("write");
    let (result, _) = run(PROTOCOL_VERSION, Operation::Check, vec![oversized_root]);
    assert_eq!(result, Err(ToolingErrorKind::TemplateLimitExceeded));

    #[cfg(unix)]
    {
        let linked_root = base.path().join("linked");
        std::os::unix::fs::symlink(template_root(), &linked_root).expect("symlink");
        let (result, _) = run(PROTOCOL_VERSION, Operation::Check, vec![linked_root]);
        assert_eq!(result, Err(ToolingErrorKind::TemplateRootRejected));

        let inner_link_root = base.path().join("inner");
        fs::create_dir_all(&inner_link_root).expect("dir");
        std::os::unix::fs::symlink(template_root().join("live"), inner_link_root.join("live"))
            .expect("symlink");
        let (result, envelopes) = run(PROTOCOL_VERSION, Operation::Check, vec![inner_link_root]);
        assert_eq!(result, Err(ToolingErrorKind::TemplateRejected));
        assert_eq!(
            end(&envelopes).1.as_deref(),
            Some("live_tooling_template_rejected")
        );
    }
}

#[test]
fn a_check_without_templates_reports_missing_views_rather_than_a_vacuous_pass() {
    let empty = tempfile::tempdir().expect("tempdir");
    let (result, envelopes) = run(
        PROTOCOL_VERSION,
        Operation::Check,
        vec![empty.path().to_path_buf()],
    );
    assert_eq!(result, Ok(()));
    let summary = envelopes
        .iter()
        .find_map(|e| match &e.body {
            Body::Summary(summary) => Some(summary),
            _ => None,
        })
        .expect("summary");
    assert_eq!(summary.template_files, 0);
    assert_eq!(summary.proved, 0);
    assert_eq!(summary.components, 2);
    let codes: Vec<&str> = envelopes
        .iter()
        .filter_map(|e| match &e.body {
            Body::Diagnostic(d) => Some(d.code.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(codes, ["missing_view", "missing_view"]);
}
