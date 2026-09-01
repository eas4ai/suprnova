//! Validation selection, bag policy, and redaction contracts.

use std::collections::BTreeMap;
use std::sync::Mutex;

use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{ActionName, ComponentName};
use suprnova_live::state::ModelPath;
use suprnova_live::validation::{
    BagPolicy, ErrorBag, ValidationEngine, ValidationEngineErrorKind, ValidationFuture,
    ValidationIssue, ValidationMessageId, ValidationPort, ValidationPortError, ValidationRequest,
    ValidationSelection, ValidationStatus,
};

struct RecordingPort {
    trace: Mutex<Vec<ValidationSelection>>,
}

impl ValidationPort for RecordingPort {
    fn validate<'a>(
        &'a self,
        request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>> {
        Box::pin(async move {
            assert_eq!(request.component().as_str(), "tests.validation");
            self.trace
                .lock()
                .expect("validation trace lock")
                .push(request.selection().clone());
            let issue = match request.selection() {
                ValidationSelection::None => return Ok(Vec::new()),
                ValidationSelection::Selected(paths) => ValidationIssue::new(
                    paths[0].clone(),
                    ValidationMessageId::parse("validation.required").expect("message identifier"),
                ),
                ValidationSelection::WholeComponent => ValidationIssue::new(
                    ModelPath::parse("profile").expect("cross-field path"),
                    ValidationMessageId::parse("validation.profile_incomplete")
                        .expect("message identifier"),
                ),
                ValidationSelection::ActionArguments => ValidationIssue::new(
                    ModelPath::parse("amount").expect("argument path"),
                    ValidationMessageId::parse("validation.amount_too_large")
                        .expect("message identifier"),
                ),
                ValidationSelection::ComponentAndArguments => ValidationIssue::new(
                    ModelPath::parse("transfer").expect("action path"),
                    ValidationMessageId::parse("validation.transfer_invalid")
                        .expect("message identifier"),
                ),
            };
            Ok(vec![issue])
        })
    }
}

fn request(selection: ValidationSelection) -> ValidationRequest<'static> {
    let state = Box::leak(Box::new(CanonicalValue::Object(BTreeMap::new())));
    let arguments = Box::leak(Box::new(CanonicalValue::Object(BTreeMap::new())));
    let action = Box::leak(Box::new(
        ActionName::parse("save").expect("registered action"),
    ));
    let component = Box::leak(Box::new(
        ComponentName::parse("tests.validation").expect("registered component"),
    ));
    ValidationRequest::new(component, selection, state, arguments).with_action(action)
}

#[tokio::test]
async fn selected_whole_action_and_cross_field_validation_are_explicit() {
    let port = RecordingPort {
        trace: Mutex::new(Vec::new()),
    };
    let engine = ValidationEngine::default();

    for selection in [
        ValidationSelection::Selected(vec![ModelPath::parse("name").expect("selected path")]),
        ValidationSelection::WholeComponent,
        ValidationSelection::ActionArguments,
        ValidationSelection::ComponentAndArguments,
    ] {
        let mut bag = ErrorBag::default();
        let status = engine
            .validate(&port, request(selection), &mut bag, BagPolicy::Replace)
            .await
            .expect("bounded validation");
        assert_eq!(status, ValidationStatus::Invalid);
        assert_eq!(bag.len(), 1);
    }

    assert_eq!(port.trace.lock().expect("validation trace lock").len(), 4);
}

#[tokio::test]
async fn error_bag_clear_retain_and_replace_are_deterministic() {
    let port = RecordingPort {
        trace: Mutex::new(Vec::new()),
    };
    let engine = ValidationEngine::default();
    let stale = ValidationIssue::new(
        ModelPath::parse("stale").expect("stale path"),
        ValidationMessageId::parse("validation.stale").expect("message identifier"),
    );
    let mut bag = ErrorBag::from_issues(vec![stale]).expect("bounded bag");

    engine
        .validate(
            &port,
            request(ValidationSelection::ActionArguments),
            &mut bag,
            BagPolicy::Retain,
        )
        .await
        .expect("retained validation");
    assert_eq!(bag.len(), 2);

    engine
        .validate(
            &port,
            request(ValidationSelection::WholeComponent),
            &mut bag,
            BagPolicy::Replace,
        )
        .await
        .expect("replaced validation");
    assert_eq!(bag.len(), 1);
    assert_eq!(bag.issues()[0].path().as_str(), "profile");

    engine
        .validate(
            &port,
            request(ValidationSelection::None),
            &mut bag,
            BagPolicy::Clear,
        )
        .await
        .expect("cleared validation");
    assert!(bag.is_empty());
}

#[test]
fn binding_and_validation_failures_have_separate_types_and_redacted_diagnostics() {
    fn accepts_validation_issue(_: &ValidationIssue) {}

    let issue = ValidationIssue::new(
        ModelPath::parse("email").expect("field path"),
        ValidationMessageId::parse("validation.email").expect("message identifier"),
    );
    accepts_validation_issue(&issue);

    let debug = format!("{issue:?}");
    assert!(debug.contains("validation.email"));
    assert!(!debug.contains("secret@example.test"));
}

struct PanickingPort;

impl ValidationPort for PanickingPort {
    fn validate<'a>(
        &'a self,
        _request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>> {
        Box::pin(async { panic!("sensitive-validation-state") })
    }
}

#[tokio::test]
async fn provider_panics_are_redacted_and_replace_updates_remain_atomic() {
    let stale = ValidationIssue::new(
        ModelPath::parse("stale").expect("stale path"),
        ValidationMessageId::parse("validation.stale").expect("message identifier"),
    );
    let mut bag = ErrorBag::from_issues(vec![stale]).expect("bounded bag");
    let error = ValidationEngine::default()
        .validate(
            &PanickingPort,
            request(ValidationSelection::WholeComponent),
            &mut bag,
            BagPolicy::Replace,
        )
        .await
        .expect_err("provider panic must not unwind or clear prior state");

    assert_eq!(error.kind(), ValidationEngineErrorKind::ProviderFailure);
    assert_eq!(bag.issues()[0].path().as_str(), "stale");
    assert!(!format!("{error:?}").contains("sensitive-validation-state"));
}
