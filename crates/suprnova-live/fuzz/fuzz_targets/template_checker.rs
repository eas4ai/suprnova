#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::checker::{CheckerLimits, TemplateCatalog, TemplateChecker};
use suprnova_live::identity::{ComponentName, ViewName};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};

fuzz_target!(|data: &[u8]| {
    if data.len() > 4_096 {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(component) = ComponentName::parse("fuzz.component") else {
        return;
    };
    let Ok(view) = ViewName::parse("fuzz/component.html") else {
        return;
    };
    let Ok(versions) = ContractVersions::new(1, 1, 1, 1, 1) else {
        return;
    };
    let Ok(metadata) = ComponentMetadata::new(
        component.clone(),
        view.clone(),
        versions,
        vec![],
        vec![],
    ) else {
        return;
    };
    let Ok(builder) = ComponentRegistryBuilder::new().register(ComponentDescriptor::new(metadata))
    else {
        return;
    };
    let registry = builder.build();
    let Ok(catalog) = TemplateCatalog::new(vec![(view, source.to_owned())]) else {
        return;
    };
    let checker = TemplateChecker::new(&registry, &catalog, CheckerLimits::default());
    let _ = checker.check_component(&component);
});
