use super::drivers::Capture;
use serial_test::serial;
use std::sync::Arc;
use suprnova::{App, Context, Exceptions, FrameworkError, Reporter, Reporting};

/// An ordinary field name proves model visibility is applied before scrubbing.
#[suprnova::model(table = "reporting_private_profiles", timestamps = false, hidden = ["ssn"])]
pub struct PrivateProfile {
    /// The public identity remains useful in diagnostics.
    pub id: i64,
    /// Model visibility excludes this field even though its name is not a secret keyword.
    pub ssn: String,
    /// Safe context survives serialization and scrubbing.
    pub label: String,
}

#[tokio::test]
#[serial]
async fn report_004_model_hidden_fields_and_secret_string_diagnostics_stay_private() {
    Exceptions::reset();
    let events = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(events.clone());
    Reporting::bootstrap().expect("reporting boot");
    let profile = PrivateProfile {
        id: 42,
        ssn: "private-social-number".into(),
        label: "public diagnostic".into(),
        ..Default::default()
    };
    let credential = suprnova::SecretString::from("private-secret-string".to_owned());
    Context::scope(Default::default(), async {
        Context::add("profile", &profile);
        // SecretString deliberately has no Serialize implementation. Its Debug
        // representation is safe when a diagnostic formats the wrapper.
        Context::add("credential_diagnostic", format!("{credential:?}"));
        suprnova::report(&FrameworkError::internal("serialization privacy")).expect("report");
    })
    .await;
    let events = events.0.lock().expect("events");
    assert_eq!(events.len(), 1);
    let payload = events[0].payload.to_string();
    assert!(!payload.contains("private-social-number"));
    assert!(!payload.contains("private-secret-string"));
    assert!(payload.contains("public diagnostic"));
    assert!(payload.contains("42"));
}
