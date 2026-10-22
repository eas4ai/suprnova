use suprnova::live::{
    LiveComponent, LiveRegistry, UploadPolicy, UploadPolicyBuilder, UploadPolicyError,
    UploadReplacement, UploadScan, UploadType, live,
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
        include_str!("../templates/live/tests/upload-without-policy.html"),
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

/// A policy that breaks no rule, to break one rule of at a time.
fn whole() -> UploadPolicyBuilder {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .finalize_action("save_avatar")
}

#[test]
fn a_policy_that_breaks_no_rule_is_valid() {
    assert_eq!(valid_policy().validate(), Ok(()));
    assert_eq!(whole().build().validate(), Ok(()));
    assert_eq!(
        whole()
            .accept_application("application/pdf", &["pdf", ".fdf"])
            .dimensions(4096, 4096, 16_000_000)
            .build()
            .validate(),
        Ok(())
    );
}

#[test]
fn a_policy_says_which_rule_it_breaks() {
    let cases: Vec<(&str, UploadPolicy, UploadPolicyError)> = vec![
        (
            "no maximum_files",
            UploadPolicy::builder()
                .maximum_file_bytes(1024)
                .accept(UploadType::Png)
                .finalize_action("save_avatar")
                .build(),
            UploadPolicyError::MaximumFiles,
        ),
        (
            "maximum_files of 0",
            invalid_policy(),
            UploadPolicyError::MaximumFiles,
        ),
        (
            "no maximum_file_bytes",
            UploadPolicy::builder()
                .maximum_files(1)
                .accept(UploadType::Png)
                .finalize_action("save_avatar")
                .build(),
            UploadPolicyError::MaximumFileBytes,
        ),
        (
            "maximum_file_bytes of 0",
            whole().maximum_file_bytes(0).build(),
            UploadPolicyError::MaximumFileBytes,
        ),
        (
            "a media type in upper case",
            whole()
                .accept_application("Application/PDF", &["pdf"])
                .build(),
            UploadPolicyError::AcceptedType {
                media_type: "Application/PDF".to_owned(),
            },
        ),
        (
            "an extension in upper case",
            whole()
                .accept_application("application/pdf", &["PDF"])
                .build(),
            UploadPolicyError::AcceptedType {
                media_type: "application/pdf".to_owned(),
            },
        ),
        (
            "no extension",
            whole().accept_application("application/pdf", &[]).build(),
            UploadPolicyError::AcceptedType {
                media_type: "application/pdf".to_owned(),
            },
        ),
        (
            "one media type twice",
            whole()
                .accept_application("application/pdf", &["pdf"])
                .accept_application("application/pdf", &["fdf"])
                .build(),
            UploadPolicyError::AcceptedTypes,
        ),
        (
            "a width of 0",
            whole().dimensions(0, 4096, 16_000_000).build(),
            UploadPolicyError::Dimensions,
        ),
        (
            "no finalize_action",
            UploadPolicy::builder()
                .maximum_files(1)
                .maximum_file_bytes(1024)
                .accept(UploadType::Png)
                .build(),
            UploadPolicyError::MissingFinalizeAction,
        ),
        (
            "a finalize_action that is no name",
            whole().finalize_action("save avatar!").build(),
            UploadPolicyError::FinalizeAction {
                name: "save avatar!".to_owned(),
            },
        ),
    ];

    for (what, policy, expected) in cases {
        let refused = policy.validate().expect_err(what);
        assert_eq!(refused, expected, "{what}");
        let said = refused.to_string();
        assert!(said.starts_with("upload policy: "), "{what}: {said}");
        assert!(
            !said.contains("  "),
            "{what}: one line, one space: {said:?}"
        );
    }
}

/// The first rule that is broken is the one that is named, so a policy
/// with two mistakes is corrected one at a time and ends.
#[test]
fn the_first_rule_that_is_broken_is_the_one_that_is_named() {
    let two = UploadPolicy::builder()
        .accept_application("Application/PDF", &["pdf"])
        .build();
    assert_eq!(two.validate(), Err(UploadPolicyError::MaximumFiles));
}

/// The registration answers with the closed error of the engine, as it
/// did, and the policy by itself says why.
#[test]
fn the_registration_keeps_its_closed_error() {
    let refused = LiveRegistry::builder()
        .register::<InvalidUploadPolicyComponent>()
        .err()
        .map(|error| error.to_string());
    assert!(
        refused
            .as_deref()
            .is_some_and(|said| !said.contains("maximum_files")),
        "the error of the registration names no rule: {refused:?}"
    );
    assert_eq!(
        invalid_policy().validate(),
        Err(UploadPolicyError::MaximumFiles)
    );
}
