//! Fixed-seed properties for browser-facing shared contracts and inert metadata.

mod checker_support;

use bytes::Bytes;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use suprnova_live::checker::{CheckerLimits, TemplateCatalog, TemplateChecker, directive_contract};
use suprnova_live::identity::{ComponentName, IslandSlot};
use suprnova_live::mount::{DocumentMountKey, MountFlags};
use suprnova_live::view::{MountMetadata, MountSnapshotKind};

fn check_source(source: String) -> suprnova_live::checker::CheckReport {
    let catalog = TemplateCatalog::new(vec![(
        checker_support::view(checker_support::ROOT_VIEW),
        source,
    )])
    .expect("generated bounded template catalog");
    let registry = checker_support::registry();
    TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&checker_support::root_name())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        rng_seed: RngSeed::Fixed(0x5a17_e003),
        .. ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_directive_fragments_have_bounded_redacted_checker_results(
        name in prop::collection::vec(any::<char>(), 0..96).prop_map(|chars| chars.into_iter().collect::<String>()),
        value in prop::collection::vec(any::<char>(), 0..256).prop_map(|chars| chars.into_iter().collect::<String>()),
    ) {
        let marker = format!("raw-secret:{value}:end");
        let source = format!("<button live:{name}=\"{marker}\"></button>");
        let report = check_source(source);
        prop_assert!(report.diagnostics().len() <= 64);
        let formatted = format!("{:?}", report.diagnostics());
        prop_assert!(formatted.len() <= 16_384);
        prop_assert!(!formatted.contains(&marker));
    }

    #[test]
    fn shared_directive_lookup_accepts_only_an_exact_reviewed_name(
        candidate in prop::collection::vec(any::<char>(), 0..160).prop_map(|chars| chars.into_iter().collect::<String>()),
    ) {
        let result = directive_contract(&candidate);
        if let Some(contract) = result {
            prop_assert_eq!(contract.name, candidate.as_str());
            prop_assert!(contract.name.len() <= 32);
            prop_assert!(contract.modifiers.len() <= 16);
        }
    }

    #[test]
    fn browser_metadata_inputs_are_bounded_and_snapshot_debug_is_redacted(
        document_key in prop::collection::vec(any::<char>(), 0..192).prop_map(|chars| chars.into_iter().collect::<String>()),
        flags in prop::collection::vec(("[ -~]{0,40}", "[ -~]{0,1100}"), 0..70),
        snapshot in prop::collection::vec(any::<u8>(), 0..4096),
    ) {
        let _ = DocumentMountKey::parse(&document_key);
        let result = MountFlags::new(flags);
        if let Ok(flags) = result {
            prop_assert!(flags.len() <= 64);
        }

        let slot = IslandSlot::parse("property-root").expect("fixed slot");
        let component = ComponentName::parse("tests.root").expect("fixed component");
        let metadata = MountMetadata::new(
            slot,
            component,
            MountSnapshotKind::Instance,
            Bytes::from(snapshot.clone()),
        );
        if let Ok(metadata) = metadata {
            prop_assert_eq!(format!("{metadata:?}"), "<MountMetadata:redacted>");
            prop_assert_eq!(metadata.signed_snapshot(), snapshot.as_slice());
        } else {
            prop_assert!(snapshot.is_empty());
        }
    }
}
