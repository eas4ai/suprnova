use std::sync::{Arc, Mutex};

use http_body_util::BodyExt;
use suprnova::live::{
    AcceptedOutcomeKind, LiveComponent, LiveConfig, LiveConfigErrorKind, LiveOutcomeAccepted,
    LiveRegistry, LiveRuntime, live,
    testing::{
        LiveTestOperation, LiveTestRuntimeProvider, LiveValidationHarness, inspect_runtime,
        prepare_live_request_with_fixed_clock_for_test, project_live_response_for_test,
        register_live_mount_for_test, report_live_outcome_for_test, same_runtime_instance,
        validate_runtime_provider_omission_for_test,
    },
};
use suprnova::{
    App, Application, EventFacade, FrameworkError, Request, Router, Server,
    events::{assert_dispatched, assert_dispatched_times},
};
use validator::Validate;

#[derive(LiveComponent)]
#[live(name = "tests.boot-component", view = "live/tests/boot-component.html")]
pub struct BootComponent {
    #[allow(
        dead_code,
        reason = "the boot fixture needs one generated state field but never executes an action"
    )]
    count: u64,
}

#[live]
impl BootComponent {}

#[derive(LiveComponent, Validate)]
#[live(
    name = "tests.validated-component",
    view = "live/tests/validated-component.html"
)]
pub struct ValidatedComponent {
    #[validate(email)]
    email: String,
    #[validate(url)]
    website: String,
    component_validations: u64,
    argument_validations: u64,
}

#[live]
impl ValidatedComponent {
    #[action(validate = "whole")]
    pub fn save(&mut self) {}

    #[action(validate = "arguments")]
    pub fn save_email(&mut self, email: String) {
        self.email = email;
    }

    #[validate]
    pub fn validate_component(&mut self) -> Result<(), validator::ValidationErrors> {
        self.component_validations += 1;
        Validate::validate(self)
    }

    #[validate(action = "save_email")]
    pub fn validate_save_email(
        &mut self,
        email: String,
    ) -> Result<(), validator::ValidationErrors> {
        self.argument_validations += 1;
        ActionEmail { email }.validate()
    }
}

#[derive(Validate)]
struct ActionEmail {
    #[validate(email)]
    email: String,
}

#[derive(serde::Deserialize, serde::Serialize, Validate)]
struct NestedEmail {
    #[validate(email)]
    email: String,
}

#[derive(LiveComponent, Validate)]
#[live(
    name = "tests.list-validation",
    view = "live/tests/list-validation.html"
)]
pub struct ListValidatedComponent {
    #[validate(nested)]
    items: Vec<NestedEmail>,
}

#[live]
impl ListValidatedComponent {
    #[action(validate = "whole")]
    pub fn save(&mut self) {}

    #[validate]
    pub fn validate_component(&self) -> Result<(), validator::ValidationErrors> {
        Validate::validate(self)
    }
}

#[test]
fn application_retains_infallible_routes_and_adds_a_fallible_boundary() {
    let _compatible = Application::new().routes(Router::new);
    let _fallible = Application::new().try_routes(|| Ok(Router::new()));
}

#[test]
fn context_lifetime_is_bounded_at_configuration_time() {
    let zero = LiveConfig::builder()
        .max_context_lifetime_ms(0)
        .build()
        .expect_err("a zero context lifetime must fail closed");
    assert_eq!(zero.kind(), LiveConfigErrorKind::InvalidContextLifetime);

    let unbounded = LiveConfig::builder()
        .max_context_lifetime_ms(300_001)
        .build()
        .expect_err("a context lifetime above the engine ceiling must fail closed");
    assert_eq!(
        unbounded.kind(),
        LiveConfigErrorKind::InvalidContextLifetime
    );
}

#[test]
fn runtime_is_bound_before_fallible_routes_and_reused_on_reentry() {
    // This test is the only environment-mutating test in this binary. Each
    // integration test file is a separate process, so the APP_KEY OnceLock and
    // App container are isolated from the existing boot suites.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::remove_var("APP_KEY");
        std::env::remove_var("APP_KEY_PREVIOUS");
        std::env::remove_var("APP_PREVIOUS_KEYS");
    }

    App::init();
    let config = LiveConfig::builder()
        .max_request_bytes(256 * 1024)
        .max_response_bytes(128 * 1024)
        .max_context_lifetime_ms(15_000)
        .build()
        .expect("test Live configuration");
    App::singleton(config);
    App::singleton(
        LiveRegistry::builder()
            .register::<BootComponent>()
            .expect("boot component registration")
            .build(),
    );

    let mut prebuilt_router = Router::new();
    register_live_mount_for_test::<BootComponent>(&mut prebuilt_router, "/catalog", "root")
        .expect("declare prebuilt Live mount");
    let _prebuilt = Server::from_config(prebuilt_router)
        .expect("prebuilt router follows the same Live preparation lifecycle");

    let observed = Arc::new(Mutex::new(None::<LiveRuntime>));
    let observed_in_routes = Arc::clone(&observed);
    let _server = Server::try_from_config_with_routes(move || {
        let runtime: LiveRuntime = App::resolve().map_err(|error| {
            FrameworkError::internal(format!(
                "Live runtime was not bound before route construction: {error}"
            ))
        })?;
        *observed_in_routes.lock().expect("observation lock") = Some(runtime);
        Ok(Router::new())
    })
    .expect("ordered server preparation");

    let during_routes = observed
        .lock()
        .expect("observation lock")
        .clone()
        .expect("route closure observed a runtime");
    let after_routes: LiveRuntime = App::resolve().expect("runtime remains container-bound");
    assert!(same_runtime_instance(&during_routes, &after_routes));
    assert_eq!(after_routes.config(), config);
    assert_eq!(after_routes.registry_len(), 1);
    assert_eq!(format!("{after_routes:?}"), "<LiveRuntime:redacted>");

    let runtime_report = inspect_runtime(&after_routes);
    assert!(runtime_report.is_complete());
    assert!(runtime_report.has_clock());
    assert!(runtime_report.has_random_source());
    assert!(runtime_report.has_key_ring());
    assert!(runtime_report.has_instance_ledger());
    assert!(runtime_report.has_seed_promotion_service());
    assert!(runtime_report.has_execution_kernel());
    assert!(runtime_report.has_context_validator());
    assert!(runtime_report.has_host_ports());
    assert!(runtime_report.has_mount_catalog());
    assert_eq!(
        format!("{runtime_report:?}"),
        "<LiveRuntimeReport:redacted>"
    );

    for provider in LiveTestRuntimeProvider::ALL {
        let error = validate_runtime_provider_omission_for_test(&after_routes, provider)
            .expect_err("every missing runtime provider must fail assembly");
        assert!(
            error.to_string().contains(provider.name()),
            "the boot error must name the missing provider without exposing state"
        );
    }

    let expires_at = prepare_live_request_with_fixed_clock_for_test(
        &after_routes,
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
        10_000,
    )
    .expect("fixed-clock provider override prepares a request");
    assert_eq!(expires_at, 25_000);

    let second = Server::try_from_config_with_routes(|| {
        let rebound: LiveRuntime = App::resolve().map_err(|error| {
            FrameworkError::internal(format!("Live runtime missing on reentry: {error}"))
        })?;
        if !same_runtime_instance(&after_routes, &rebound) {
            return Err(FrameworkError::internal(
                "server reentry replaced the immutable Live runtime",
            ));
        }
        Ok(Router::new())
    });
    assert!(
        second.is_ok(),
        "a second prepared server reuses the runtime"
    );

    let failed = Server::try_from_config_with_routes(|| {
        Err(FrameworkError::internal("live route catalog rejected"))
    });
    let error = match failed {
        Ok(_) => panic!("fallible route construction must return its error"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("live route catalog rejected"));
}

#[test]
fn host_adapter_sources_remain_framework_private_and_engine_trait_owned() {
    let sources = [
        include_str!("../src/live/ports/authorization.rs"),
        include_str!("../src/live/ports/transaction.rs"),
        include_str!("../src/live/ports/validation.rs"),
        include_str!("../src/live/ports/events.rs"),
        include_str!("../src/live/ports/telemetry.rs"),
        include_str!("../src/live/ports/cancellation.rs"),
        include_str!("../src/live/ports/response.rs"),
    ];

    for source in sources {
        assert!(
            !source.contains("pub trait ") && !source.contains("pub enum "),
            "framework adapters must not introduce parallel public contracts"
        );
        assert!(
            source.contains("impl "),
            "each adapter source must implement an engine-owned contract"
        );
    }
}

#[tokio::test]
async fn endpoint_response_intent_projects_without_losing_status_headers_or_body() {
    let response = project_live_response_for_test(
        409,
        [
            ("content-type", "application/vnd.suprnova-live+json"),
            ("x-live-recovery", "fresh-render"),
        ],
        b"{\"kind\":\"refresh_required\"}",
    )
    .expect("project endpoint response");
    assert_eq!(response.status_code(), 409);
    assert_eq!(
        response.header_value("content-type"),
        Some("application/vnd.suprnova-live+json")
    );
    assert_eq!(
        response.header_value("x-live-recovery"),
        Some("fresh-render")
    );
    let body = response
        .into_hyper()
        .into_body()
        .collect()
        .await
        .expect("collect projected body")
        .to_bytes();
    assert_eq!(body.as_ref(), b"{\"kind\":\"refresh_required\"}");
}

#[tokio::test]
async fn accepted_outcome_reporter_dispatches_one_engine_typed_event() {
    let _fake = EventFacade::fake();
    report_live_outcome_for_test(7, AcceptedOutcomeKind::NoRender)
        .await
        .expect("post-acceptance reporting");
    assert_dispatched::<LiveOutcomeAccepted>(|event| {
        event.revision() == 7 && event.outcome() == AcceptedOutcomeKind::NoRender
    });
    assert_dispatched_times::<LiveOutcomeAccepted>(1);
}

#[tokio::test]
async fn selected_validation_uses_the_sealed_component_registration_and_fails_closed() {
    let harness = LiveValidationHarness::new().expect("sealed validation fixture");
    let issues = harness
        .validate("tests.validation")
        .await
        .expect("registered component validation callback");
    assert_eq!(
        issues,
        vec![(
            "profile.email".to_owned(),
            "validation.email_invalid".to_owned(),
        )]
    );

    let error = harness
        .validate("tests.unregistered")
        .await
        .expect_err("unknown component validation must fail closed");
    assert!(error.to_string().contains("validation provider rejected"));
}

#[tokio::test]
async fn macro_registration_seals_the_real_typed_validation_hook() {
    let registry = LiveRegistry::builder()
        .register::<ValidatedComponent>()
        .expect("macro component with a typed validation hook registers")
        .build();
    let harness = LiveValidationHarness::from_registry(registry);
    let mut component = ValidatedComponent {
        email: "not-an-email".to_owned(),
        website: "https://example.test".to_owned(),
        component_validations: 0,
        argument_validations: 0,
    };
    let issues = harness
        .validate_target("tests.validated-component", "save", &mut component)
        .await
        .expect("generated typed validation callback");
    assert_eq!(
        issues,
        vec![("email".to_owned(), "validation.email".to_owned())]
    );
    assert_eq!(component.component_validations, 1);
    assert_eq!(component.argument_validations, 0);
}

#[tokio::test]
async fn generated_validation_honors_argument_selection_and_typed_action_values() {
    let registry = LiveRegistry::builder()
        .register::<ValidatedComponent>()
        .expect("macro component with component and argument hooks registers")
        .build();
    let harness = LiveValidationHarness::from_registry(registry);
    let mut component = ValidatedComponent {
        email: "valid@example.test".to_owned(),
        website: "https://example.test".to_owned(),
        component_validations: 0,
        argument_validations: 0,
    };
    let issues = harness
        .validate_string_action_target(
            "tests.validated-component",
            "save_email",
            [("email", "not-an-email")],
            &mut component,
        )
        .await
        .expect("generated typed action-argument validation callback");
    assert_eq!(
        issues,
        vec![("email".to_owned(), "validation.email".to_owned())]
    );
    assert_eq!(component.component_validations, 0);
    assert_eq!(component.argument_validations, 1);
}

#[tokio::test]
async fn generated_selected_validation_returns_only_declared_model_paths() {
    let registry = LiveRegistry::builder()
        .register::<ValidatedComponent>()
        .expect("macro component with a component hook registers")
        .build();
    let harness = LiveValidationHarness::from_registry(registry);
    let mut component = ValidatedComponent {
        email: "not-an-email".to_owned(),
        website: "not-a-url".to_owned(),
        component_validations: 0,
        argument_validations: 0,
    };
    let issues = harness
        .validate_selected_target(
            "tests.validated-component",
            "save",
            ["email"],
            &mut component,
        )
        .await
        .expect("selected generated component validation");
    assert_eq!(
        issues,
        vec![("email".to_owned(), "validation.email".to_owned())]
    );
    assert_eq!(component.component_validations, 1);
    assert_eq!(component.argument_validations, 0);
}

#[tokio::test]
async fn nested_list_validation_collapses_and_deduplicates_stable_model_paths() {
    let registry = LiveRegistry::builder()
        .register::<ListValidatedComponent>()
        .expect("nested-list validation component registers")
        .build();
    let harness = LiveValidationHarness::from_registry(registry);
    let mut component = ListValidatedComponent {
        items: (0..3)
            .map(|_| NestedEmail {
                email: "not-an-email".to_owned(),
            })
            .collect(),
    };
    let issues = harness
        .validate_target_with_issue_limit("tests.list-validation", "save", 1, &mut component)
        .await
        .expect("deduplicated list validation must fit the one-issue engine budget");
    assert_eq!(
        issues,
        vec![("items.email".to_owned(), "validation.email".to_owned())]
    );
}

#[test]
fn server_new_prepares_live_before_attempting_to_bind_a_socket() {
    let output = std::process::Command::new(std::env::current_exe().expect("current test binary"))
        .args([
            "--ignored",
            "--exact",
            "server_new_child_rejects_invalid_live_mount_before_socket_binding",
        ])
        .output()
        .expect("run isolated Server::new lifecycle proof");
    assert!(
        output.status.success(),
        "isolated Server::new proof failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "executed in a child process by the Server::new lifecycle proof"]
fn server_new_child_rejects_invalid_live_mount_before_socket_binding() {
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::remove_var("APP_KEY");
        std::env::remove_var("APP_KEY_PREVIOUS");
        std::env::remove_var("APP_PREVIOUS_KEYS");
    }
    let mut router = Router::new();
    register_live_mount_for_test::<BootComponent>(&mut router, "/catalog", "root")
        .expect("declare Live mount without registering its component");
    let error = tokio::runtime::Runtime::new()
        .expect("test runtime")
        .block_on(Server::new(router).host("not-an-ip").run())
        .expect_err("an unknown Live component must fail before listening");
    assert!(
        error
            .to_string()
            .contains("Live mount catalog was rejected")
    );
    assert!(!error.to_string().contains("invalid server host"));
}
