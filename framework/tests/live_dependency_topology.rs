use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("framework lives under workspace root")
        .to_path_buf()
}

#[test]
fn framework_owns_the_only_inward_live_dependency() {
    let root = workspace_root();
    let framework_manifest =
        fs::read_to_string(root.join("framework/Cargo.toml")).expect("read framework manifest");
    let engine_manifest = fs::read_to_string(root.join("crates/suprnova-live/Cargo.toml"))
        .expect("read engine manifest");

    let expected = "suprnova-live = { path = \"../crates/suprnova-live\"";
    assert!(framework_manifest.contains(expected));
    assert_eq!(framework_manifest.matches("suprnova-live = {").count(), 1);
    assert!(!engine_manifest.contains("path = \"../../framework\""));
    assert!(!engine_manifest.contains("path = \"../../../framework\""));

    let output = Command::new("cargo")
        .args(["tree", "-p", "suprnova-live", "--edges", "normal"])
        .current_dir(&root)
        .output()
        .expect("run cargo tree");
    assert!(output.status.success(), "cargo tree failed");
    let tree = String::from_utf8(output.stdout).expect("cargo tree is UTF-8");
    assert!(
        !tree.lines().skip(1).any(|line| line.contains("suprnova v")),
        "internal Live engine must not depend on the public framework\n{tree}"
    );
}

#[test]
fn application_sources_and_manuals_name_only_the_public_facade() {
    let root = workspace_root();
    for relative in ["app/src", "manual", "framework/README.md"] {
        let path = root.join(relative);
        if path.is_file() {
            let source = fs::read_to_string(&path).expect("read public source");
            assert!(!source.contains("suprnova_live"), "{}", path.display());
            continue;
        }
        if !path.exists() {
            continue;
        }
        let mut pending = vec![path];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).expect("read public source directory") {
                let entry = entry.expect("read directory entry");
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|extension| {
                    matches!(extension.to_str(), Some("rs" | "md" | "toml"))
                }) {
                    let source = fs::read_to_string(&path).expect("read public source");
                    assert!(!source.contains("suprnova_live"), "{}", path.display());
                }
            }
        }
    }
}

#[test]
fn generated_abi_is_symbol_allowlisted() {
    let source = fs::read_to_string(workspace_root().join("framework/src/live/__private.rs"))
        .expect("read generated ABI facade");
    assert!(!source.contains("pub use suprnova_live::*"));
    for module in [
        "action",
        "component",
        "identity",
        "metadata",
        "registry",
        "snapshot",
        "state",
        "validation",
    ] {
        assert!(
            !source.contains(&format!("pub use suprnova_live::{module};")),
            "generated ABI must not re-export the complete `{module}` engine module"
        );
    }

    let expected = [
        "suprnova_live::action::ActionArgumentField",
        "suprnova_live::action::ActionArgumentSchema",
        "suprnova_live::action::ActionEntry",
        "suprnova_live::action::ActionError",
        "suprnova_live::action::ActionTable",
        "suprnova_live::action::AuthorizationRequirement",
        "suprnova_live::action::AuthorizedAction",
        "suprnova_live::action::IntoActionResult",
        "suprnova_live::action::TransactionPolicy",
        "suprnova_live::async_updates::BoundedEventNames",
        "suprnova_live::async_updates::BoundedTargets",
        "suprnova_live::async_updates::BoundedTopics",
        "suprnova_live::async_updates::EventCyclePolicy",
        "suprnova_live::async_updates::EventOrder",
        "suprnova_live::async_updates::EventSource",
        "suprnova_live::async_updates::EventTarget",
        "suprnova_live::async_updates::ReconnectPolicy",
        "suprnova_live::async_updates::StreamName",
        "suprnova_live::async_updates::SubscriptionMetadata",
        "suprnova_live::async_updates::SubscriptionMode",
        "suprnova_live::async_updates::SubscriptionModes",
        "suprnova_live::async_updates::TopicName",
        "suprnova_live::canonical::CanonicalValue",
        "suprnova_live::child::EligibleChildParametersV2",
        "suprnova_live::child::VerifiedChildParametersV1",
        "suprnova_live::component::ComponentError",
        "suprnova_live::component::ComponentHooks",
        "suprnova_live::component::LiveFuture",
        "suprnova_live::component::MountContext",
        "suprnova_live::component::RenderContext",
        "suprnova_live::component::composition::ChildParameterField",
        "suprnova_live::component::composition::ChildParameterSchema",
        "suprnova_live::component::generated::GeneratedComponentRuntime",
        "suprnova_live::component::generated::GeneratedComponentState",
        "suprnova_live::component::generated::IntoComponentHookResult",
        "suprnova_live::component::generated::IntoComponentResult",
        "suprnova_live::component::generated::component_hooks",
        "suprnova_live::component::generated::decode_field",
        "suprnova_live::component::generated::decode_json_field",
        "suprnova_live::component::generated::decode_model_field",
        "suprnova_live::component::generated::encode_field",
        "suprnova_live::component::generated::render_component_view",
        "suprnova_live::identity::ActionName",
        "suprnova_live::identity::ComponentName",
        "suprnova_live::identity::ModelField",
        "suprnova_live::identity::ViewName",
        "suprnova_live::metadata::ActionMetadata",
        "suprnova_live::metadata::ComponentMetadata",
        "suprnova_live::metadata::ContractVersions",
        "suprnova_live::metadata::EffectMetadata",
        "suprnova_live::metadata::EffectPayloadMetadata",
        "suprnova_live::metadata::EventMetadata",
        "suprnova_live::metadata::EventPayloadMetadata",
        "suprnova_live::metadata::FieldMetadata",
        "suprnova_live::metadata::LiveComponentContract",
        "suprnova_live::metadata::LiveComponentDefinitionMetadata",
        "suprnova_live::metadata::MetadataError",
        "suprnova_live::registry::ComponentDescriptor",
        "suprnova_live::snapshot::state::FieldCategory",
        "suprnova_live::snapshot::state::StateCodec",
        "suprnova_live::snapshot::state::StateExposure",
        "suprnova_live::state::BindingTiming",
        "suprnova_live::state::ModelCodec",
        "suprnova_live::state::ModelPath",
        "suprnova_live::state::ProposalBatch",
        "suprnova_live::state::UrlBinding",
        "suprnova_live::state::UrlBindingMode",
        "suprnova_live::validation::ValidationFuture",
        "suprnova_live::validation::ValidationIssue",
        "suprnova_live::validation::ValidationPort",
        "suprnova_live::validation::ValidationPortError",
        "suprnova_live::validation::ValidationRequest",
        "suprnova_live::validation::ValidationSelection",
        "suprnova_live::view::IslandRender",
    ];
    // Every `pub use suprnova_live::...;` item, with brace groups expanded so a
    // grouped re-export cannot hide symbols from the allowlist.
    let marker = "pub use suprnova_live::";
    let mut actual = Vec::new();
    for (index, _) in source.match_indices(marker) {
        let path = source[index + marker.len()..]
            .split(';')
            .next()
            .expect("use path before the statement terminator")
            .split_whitespace()
            .collect::<String>();
        match path.split_once('{') {
            Some((base, group)) => {
                let group = group.strip_suffix('}').expect("closed use group");
                assert!(
                    !group.contains('{'),
                    "nested use groups cannot be allowlisted: {path}"
                );
                for item in group.split(',').filter(|item| !item.is_empty()) {
                    actual.push(format!("suprnova_live::{base}{item}"));
                }
            }
            None => actual.push(format!("suprnova_live::{path}")),
        }
    }
    actual.sort_unstable();
    let mut expected = expected
        .iter()
        .map(|item| (*item).to_string())
        .collect::<Vec<_>>();
    expected.sort_unstable();
    assert_eq!(actual, expected, "generated ABI allowlist drifted");
}
