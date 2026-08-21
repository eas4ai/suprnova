//! Binding timing, URL, session, and canonical field-metadata contracts.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{ComponentName, ModelField, ViewName};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, FieldMetadata, MetadataErrorKind,
};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::state::{
    BindingTiming, ModelCodec, SessionError, SessionField, SessionIntent, SessionIntentKind,
    SessionIntents, SessionPort, SessionValue, TimingErrorKind, UrlBinding, UrlBindingMode,
    UrlBindingSet, UrlErrorKind,
};

fn field(name: &str, category: FieldCategory) -> FieldMetadata {
    FieldMetadata::new(
        ModelField::parse(name).expect("field identity"),
        category,
        StateCodec::Json,
        true,
    )
}

#[test]
fn timing_policies_are_closed_and_debounce_is_bounded() {
    assert_eq!(BindingTiming::default(), BindingTiming::Submit);
    for timing in [
        BindingTiming::Immediate,
        BindingTiming::Change,
        BindingTiming::Blur,
        BindingTiming::Submit,
    ] {
        assert_eq!(timing.debounce_millis(), None);
    }

    let debounce = BindingTiming::debounce(250).expect("bounded debounce");
    assert_eq!(debounce.debounce_millis(), Some(250));
    assert_eq!(
        BindingTiming::debounce(0)
            .expect_err("zero debounce")
            .kind(),
        TimingErrorKind::InvalidDebounce
    );
    assert_eq!(
        BindingTiming::debounce(60_001)
            .expect_err("unbounded debounce")
            .kind(),
        TimingErrorKind::InvalidDebounce
    );
}

#[test]
fn url_bindings_are_scalar_typed_and_preserve_navigation_semantics() {
    let reflected = UrlBinding::new(
        "page",
        FieldCategory::Model,
        ModelCodec::U64,
        UrlBindingMode::Reflect,
        true,
    )
    .expect("reflected URL binding");
    assert_eq!(reflected.query_key(), "page");
    assert_eq!(reflected.mode(), UrlBindingMode::Reflect);
    assert!(reflected.omit_default());
    assert_eq!(
        reflected
            .encode(&42_u64, &InputLimits::default())
            .expect("URL value encodes"),
        "42"
    );
    assert_eq!(
        reflected
            .decode::<u64>("42", &InputLimits::default())
            .expect("URL value decodes"),
        42
    );
    assert!(
        reflected
            .decode::<u64>("not-an-integer", &InputLimits::default())
            .is_err()
    );
    assert_eq!(
        reflected
            .encode_if_changed(&42_u64, &42_u64, &InputLimits::default())
            .expect("default comparison"),
        None
    );

    let navigated = UrlBinding::new(
        "category",
        FieldCategory::Public,
        ModelCodec::String,
        UrlBindingMode::Navigate,
        false,
    )
    .expect("real-route URL binding");
    assert_eq!(navigated.mode(), UrlBindingMode::Navigate);
}

#[test]
fn url_metadata_rejects_sensitive_categories_complex_values_and_duplicate_keys() {
    for category in [
        FieldCategory::Locked,
        FieldCategory::ServerOnly,
        FieldCategory::Session,
        FieldCategory::Computed,
        FieldCategory::Transient,
        FieldCategory::Secret,
    ] {
        let error = UrlBinding::new(
            "unsafe",
            category,
            ModelCodec::String,
            UrlBindingMode::Reflect,
            false,
        )
        .expect_err("category is not URL-shareable");
        assert_eq!(error.kind(), UrlErrorKind::ForbiddenCategory);
    }

    let complex = UrlBinding::new(
        "filters",
        FieldCategory::Model,
        ModelCodec::map(ModelCodec::String),
        UrlBindingMode::Reflect,
        false,
    )
    .expect_err("complex URL codec");
    assert_eq!(complex.kind(), UrlErrorKind::UnsupportedCodec);

    let first = UrlBinding::new(
        "q",
        FieldCategory::Model,
        ModelCodec::String,
        UrlBindingMode::Reflect,
        false,
    )
    .expect("first query binding");
    let second = UrlBinding::new(
        "q",
        FieldCategory::Public,
        ModelCodec::String,
        UrlBindingMode::Navigate,
        false,
    )
    .expect("second query binding");
    let error = UrlBindingSet::new(vec![
        (ModelField::parse("query").expect("field identity"), first),
        (
            ModelField::parse("category").expect("field identity"),
            second,
        ),
    ])
    .expect_err("duplicate query key");
    assert_eq!(error.kind(), UrlErrorKind::DuplicateQueryKey);
}

#[test]
fn model_and_url_metadata_are_part_of_the_component_contract_digest() {
    let plain = field("query", FieldCategory::Model)
        .with_model_binding(ModelCodec::String, BindingTiming::Submit)
        .expect("plain model metadata");
    let bound = field("query", FieldCategory::Model)
        .with_model_binding(ModelCodec::String, BindingTiming::Blur)
        .expect("model metadata")
        .with_url_binding(
            UrlBinding::new(
                "q",
                FieldCategory::Model,
                ModelCodec::String,
                UrlBindingMode::Reflect,
                true,
            )
            .expect("URL metadata"),
        )
        .expect("field URL metadata");

    assert_eq!(bound.model_codec(), Some(&ModelCodec::String));
    assert_eq!(bound.binding_timing(), Some(BindingTiming::Blur));
    assert_eq!(bound.url_binding().expect("URL binding").query_key(), "q");
    assert_ne!(
        metadata(plain).contract_digest(),
        metadata(bound).contract_digest()
    );

    let invalid_timing = field("invalid", FieldCategory::Model)
        .with_model_binding(ModelCodec::String, BindingTiming::Debounce(0))
        .expect_err("unchecked zero debounce cannot enter metadata");
    assert_eq!(
        invalid_timing.kind(),
        suprnova_live::metadata::MetadataErrorKind::InvalidBindingMetadata
    );
}

#[test]
fn component_metadata_rejects_duplicate_url_query_keys() {
    let query = field("query", FieldCategory::Model)
        .with_model_binding(ModelCodec::String, BindingTiming::Submit)
        .expect("query model metadata")
        .with_url_binding(
            UrlBinding::new(
                "q",
                FieldCategory::Model,
                ModelCodec::String,
                UrlBindingMode::Reflect,
                false,
            )
            .expect("query URL binding"),
        )
        .expect("query metadata");
    let category = field("category", FieldCategory::Public)
        .with_url_binding(
            UrlBinding::new(
                "q",
                FieldCategory::Public,
                ModelCodec::String,
                UrlBindingMode::Navigate,
                false,
            )
            .expect("category URL binding"),
        )
        .expect("category metadata");

    let error = ComponentMetadata::new(
        ComponentName::parse("search").expect("component identity"),
        ViewName::parse("live/search.html").expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
        vec![query, category],
        vec![],
    )
    .expect_err("duplicate URL query key");

    assert_eq!(error.kind(), MetadataErrorKind::DuplicateUrlQueryKey);
}

#[test]
fn component_metadata_requires_registered_model_and_session_contracts() {
    for (name, category) in [
        ("query", FieldCategory::Model),
        ("upload", FieldCategory::Transient),
        ("theme", FieldCategory::Session),
    ] {
        let error = ComponentMetadata::new(
            ComponentName::parse("preferences").expect("component identity"),
            ViewName::parse("live/preferences.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
            vec![field(name, category)],
            vec![],
        )
        .expect_err("binding contract must be registered");

        assert_eq!(error.kind(), MetadataErrorKind::InvalidBindingMetadata);
    }
}

fn metadata(field: FieldMetadata) -> ComponentMetadata {
    ComponentMetadata::new(
        ComponentName::parse("search").expect("component identity"),
        ViewName::parse("live/search.html").expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
        vec![field],
        vec![],
    )
    .expect("component metadata")
}

#[derive(Default)]
struct MemorySession {
    values: Mutex<BTreeMap<String, CanonicalValue>>,
}

#[async_trait]
impl SessionPort for MemorySession {
    async fn read(&self, field: &SessionField) -> Result<Option<SessionValue>, SessionError> {
        self.values
            .lock()
            .expect("session lock")
            .get(field.name().as_str())
            .cloned()
            .map(|value| SessionValue::from_canonical(field, value, &InputLimits::default()))
            .transpose()
    }

    async fn apply(&self, intent: &SessionIntent) -> Result<(), SessionError> {
        let mut values = self.values.lock().expect("session lock");
        match intent.kind() {
            SessionIntentKind::Set => {
                values.insert(
                    intent.field().name().as_str().to_owned(),
                    intent
                        .value()
                        .expect("set intent value")
                        .canonical()
                        .clone(),
                );
            }
            SessionIntentKind::Remove => {
                values.remove(intent.field().name().as_str());
            }
        }
        Ok(())
    }
}

#[tokio::test]
async fn session_state_uses_registered_metadata_and_redacted_bounded_intents() {
    let session_field = field("theme", FieldCategory::Session)
        .with_session_binding(ModelCodec::String)
        .expect("registered session metadata");
    let registered = SessionField::from_metadata(&session_field).expect("session field");
    assert!(SessionField::from_metadata(&field("query", FieldCategory::Model)).is_err());

    let set = SessionIntent::set(
        registered.clone(),
        &"midnight".to_owned(),
        &InputLimits::default(),
    )
    .expect("bounded session intent");
    assert!(!format!("{set:?}").contains("midnight"));

    let mut intents = SessionIntents::new(1).expect("bounded intent collector");
    intents.push(set.clone()).expect("first intent");
    assert!(
        intents
            .push(SessionIntent::remove(registered.clone()))
            .is_err()
    );

    let port = MemorySession::default();
    port.apply(&set).await.expect("session write");
    let loaded = port
        .read(&registered)
        .await
        .expect("session read")
        .expect("stored session value");
    assert_eq!(
        loaded
            .decode::<String>(&registered, &InputLimits::default())
            .expect("typed session value"),
        "midnight"
    );
    assert!(!format!("{loaded:?}").contains("midnight"));

    port.apply(&SessionIntent::remove(registered.clone()))
        .await
        .expect("session removal");
    assert!(
        port.read(&registered)
            .await
            .expect("session read")
            .is_none()
    );
}
