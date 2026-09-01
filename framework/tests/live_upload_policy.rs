use suprnova::live::{
    LiveComponent, LiveRegistry, UploadPolicy, UploadReplacement, UploadScan, UploadType, live,
};
use suprnova_live::checker::{CheckerLimits, DiagnosticCode, TemplateCatalog, TemplateChecker};
use suprnova_live::identity::{ComponentName, ViewName};
use suprnova_live::registry::ComponentRegistryBuilder;

fn valid_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

fn invalid_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(0)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .finalize_action("save_avatar")
        .build()
}

fn missing_action_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .finalize_action("missing_action")
        .build()
}

#[derive(LiveComponent)]
#[live(
    name = "tests.valid-upload-policy",
    view = "live/tests/upload-policy.html"
)]
pub struct ValidUploadPolicyComponent {
    #[model]
    #[upload(policy = valid_policy)]
    avatar: String,
}

#[live]
impl ValidUploadPolicyComponent {
    #[action]
    pub fn save_avatar(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "tests.invalid-upload-policy",
    view = "live/tests/upload-policy.html"
)]
pub struct InvalidUploadPolicyComponent {
    #[model]
    #[upload(policy = invalid_policy)]
    avatar: String,
}

#[live]
impl InvalidUploadPolicyComponent {
    #[action]
    pub fn save_avatar(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "tests.missing-upload-finalize-action",
    view = "live/tests/upload-policy.html"
)]
pub struct MissingUploadFinalizeAction {
    #[model]
    #[upload(policy = missing_action_policy)]
    avatar: String,
}

#[live]
impl MissingUploadFinalizeAction {}

#[derive(LiveComponent)]
#[live(
    name = "tests.template-upload-without-policy",
    view = "live/tests/upload-without-policy.html",
    checker_contract_version = 2
)]
pub struct TemplateUploadWithoutPolicy {
    #[model]
    avatar: String,
}

#[live]
impl TemplateUploadWithoutPolicy {}

#[test]
fn generated_upload_policy_is_validated_and_bound_to_a_registered_finalize_action() {
    let registry = LiveRegistry::builder()
        .register::<ValidUploadPolicyComponent>()
        .expect("valid generated upload policy")
        .build();
    assert_eq!(registry.len(), 1);

    assert!(
        LiveRegistry::builder()
            .register::<InvalidUploadPolicyComponent>()
            .is_err(),
        "an invalid application builder result must fail before registry publication"
    );
    assert!(
        LiveRegistry::builder()
            .register::<MissingUploadFinalizeAction>()
            .is_err(),
        "the generated policy finalize action must exist in the registered action table"
    );
}

#[test]
fn checked_template_upload_requires_generated_field_policy() {
    let descriptor = <TemplateUploadWithoutPolicy as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("component metadata without upload policy is otherwise valid");
    let component =
        ComponentName::parse("tests.template-upload-without-policy").expect("component identity");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("register macro descriptor")
        .build();
    let view = ViewName::parse("live/tests/upload-without-policy.html").expect("view identity");
    let catalog = TemplateCatalog::new(vec![(
        view,
        include_str!("templates/live/tests/upload-without-policy.html"),
    )])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&component);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::ForbiddenModel),
        "the checked template must reject live:upload for a model without generated policy"
    );
}
