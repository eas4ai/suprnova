//! Every public model serialization uses the same instance policy.

use crate::query_fixture::Fixture;
use serde_json::{Value, json};
use suprnova::{Collection, Model, accessor, attrs, model};

/// A secret accessor must stay hidden along with ordinary fields.
#[model(table = "gap_hidden", timestamps = false, hidden = ["secret", "private_label"],
    appends = ["private_label"], accessors = ["full_name"])]
pub struct Hidden {
    pub id: i64,
    pub email: String,
    pub secret: String,
}

impl Hidden {
    /// Provide a runtime append without adding it to the default output.
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("User {}", self.id)
    }

    /// Panic if a hidden accessor is evaluated before the visibility filter.
    #[accessor]
    pub fn private_label(&self) -> String {
        panic!("hidden accessor must not run")
    }
}

/// Visible lists also gate default and runtime appended names.
#[model(table = "gap_visible", timestamps = false, visible = ["id"],
    appends = ["label"], accessors = ["full_name"])]
pub struct Visible {
    pub id: i64,
    pub email: String,
}

impl Visible {
    /// An omitted appended name tests the allowlist.
    #[accessor]
    pub fn label(&self) -> String {
        "label".into()
    }

    /// The runtime dispatcher can append this name after it becomes visible.
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("User {}", self.id)
    }
}

/// An ordinary model also receives instance overrides without declared lists.
#[model(table = "gap_plain", timestamps = false)]
pub struct Plain {
    pub id: i64,
    pub email: String,
}

/// A serde rename and skip option remain part of the model's output contract.
#[model(table = "gap_renamed", timestamps = false, hidden = ["displayName"])]
#[serde(crate = "suprnova::serde")]
pub struct Renamed {
    pub id: i64,
    #[serde(rename = "displayName")]
    pub name: String,
    #[serde(skip_serializing)]
    pub internal: String,
}

/// A common application type name must not collide with generated serde helpers.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Attributes {
    pub enabled: bool,
}

/// A structured cast exercises a field whose type is named Attributes.
#[model(table = "gap_application_attributes", timestamps = false,
    casts = { attributes = suprnova::AsObject<Attributes> })]
pub struct WithAttributes {
    pub id: i64,
    pub attributes: Attributes,
}

/// A fully skipped output requires no borrowed lifetime in the generated view.
#[model(table = "gap_empty_output", timestamps = false)]
pub struct EmptyOutput {
    #[serde(skip_serializing)]
    pub id: i64,
}

trait LabelValue {
    fn label(&self) -> &str;
}

impl LabelValue for String {
    fn label(&self) -> &str {
        self
    }
}

fn serialize_label<T: LabelValue, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(value.label())
}

/// Generic serde callbacks must receive the field's original type.
#[model(table = "gap_generic_serializer", timestamps = false)]
pub struct GenericSerializer {
    #[serde(skip_serializing)]
    pub id: i64,
    #[serde(serialize_with = "serialize_label")]
    pub label: String,
}

/// A failed accessor serializer gives the shared policy a recoverable error path.
pub struct FailingValue;

impl serde::Serialize for FailingValue {
    fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("accessor serialization failed"))
    }
}

/// A default append can fail or be suppressed before its serializer runs.
#[model(table = "gap_failed_append", timestamps = false, appends = ["broken"])]
pub struct FailedAppend {
    pub id: i64,
}

impl FailedAppend {
    /// Return a deliberately fallible value to verify serde error propagation.
    #[accessor]
    pub fn broken(&self) -> FailingValue {
        FailingValue
    }
}

fn assert_all<M: serde::Serialize>(model: &M, expected: Value) {
    assert_eq!(serde_json::to_value(model).unwrap(), expected);
    assert_eq!(
        serde_json::from_str::<Value>(&serde_json::to_string(model).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        serde_json::to_value(vec![model]).unwrap(),
        json!([expected])
    );
}

fn hidden() -> Hidden {
    Hidden {
        id: 3,
        email: "a@example.test".into(),
        secret: "password".into(),
        ..Default::default()
    }
}

#[test]
fn declared_hidden_applies_to_serde_arrays_json_collections_and_appends() {
    let model = hidden();
    let expected = json!({"id": 3, "email": "a@example.test"});
    assert_all(&model, expected.clone());
    assert_eq!(model.to_array(), expected);
    assert_eq!(
        serde_json::from_str::<Value>(&model.to_json()).unwrap(),
        expected
    );
    let collection = Collection::from(vec![model]);
    assert_eq!(collection.to_array(), json!([expected]));
    assert_eq!(
        serde_json::to_value(&collection).unwrap(),
        json!([expected])
    );
}

#[test]
fn runtime_hiding_revealing_and_conditions_persist_only_on_the_instance() {
    let mut model = hidden();
    let untouched = model.clone();
    model.make_hidden_if(false, "email");
    assert!(model.to_array().get("email").is_some());
    model.make_visible_if(false, "secret");
    assert!(model.to_array().get("secret").is_none());
    model.make_hidden_if(true, ["email", "missing", "email"]);
    assert_all(&model, json!({"id": 3}));
    model.make_visible_if(true, "secret");
    assert_all(&model, json!({"id": 3, "secret": "password"}));
    model.make_visible(["email", "missing"]);
    assert_eq!(
        model.to_array(),
        json!({"id": 3, "email": "a@example.test", "secret": "password"})
    );
    assert_eq!(
        untouched.to_array(),
        json!({"id": 3, "email": "a@example.test"})
    );
    let mut copy = model.clone();
    copy.make_hidden("secret");
    assert!(model.to_array().get("secret").is_some());
    assert!(copy.to_array().get("secret").is_none());
}

#[test]
fn visible_lists_extend_for_revealed_fields_and_appends() {
    let mut model = Visible {
        id: 7,
        email: "b@example.test".into(),
        ..Default::default()
    };
    assert_all(&model, json!({"id": 7}));
    model.append("full_name").unwrap();
    assert_eq!(model.to_array(), json!({"id": 7}));
    model.make_visible(["email", "label", "full_name"]);
    let expected =
        json!({"id": 7, "email": "b@example.test", "label": "label", "full_name": "User 7"});
    assert_eq!(model.to_array(), expected);
    assert_all(&model, expected);
    model.make_hidden(["label", "full_name"]);
    assert_all(&model, json!({"id": 7, "email": "b@example.test"}));
    model.make_visible("full_name");
    assert_eq!(model.to_array()["full_name"], "User 7");
}

#[test]
fn runtime_appends_are_idempotent_filtered_and_reject_unknown_names() {
    let mut model = hidden();
    assert!(model.to_array().get("full_name").is_none());
    model
        .append("full_name")
        .unwrap()
        .append("full_name")
        .unwrap();
    assert_all(
        &model,
        json!({"id": 3, "email": "a@example.test", "full_name": "User 3"}),
    );
    model.make_hidden("full_name");
    assert!(model.to_array().get("full_name").is_none());
    model.make_visible("full_name");
    assert_eq!(model.to_array()["full_name"], "User 3");
    let before = model.to_array();
    assert!(model.append("unknown").is_err());
    assert_eq!(model.to_array(), before);
}

#[test]
fn models_without_declared_lists_support_runtime_visibility() {
    let mut model = Plain {
        id: 9,
        email: "plain@example.test".into(),
        ..Default::default()
    };
    model.make_hidden("email");
    assert_all(&model, json!({"id": 9}));
    model.make_visible("email");
    assert_all(&model, json!({"id": 9, "email": "plain@example.test"}));
}

#[test]
fn serde_field_options_and_accessor_errors_survive_the_shared_policy() {
    let mut renamed = Renamed {
        id: 1,
        name: "Alice".into(),
        internal: "private".into(),
        ..Default::default()
    };
    assert_all(&renamed, json!({"id": 1}));
    renamed.make_visible("displayName");
    assert_all(&renamed, json!({"id": 1, "displayName": "Alice"}));
    let custom = WithAttributes {
        id: 4,
        attributes: Attributes { enabled: true },
        ..Default::default()
    };
    assert_all(&custom, json!({"id": 4, "attributes": {"enabled": true}}));
    assert_all(&EmptyOutput::default(), json!({}));
    let custom = GenericSerializer {
        label: "typed callback".into(),
        ..Default::default()
    };
    assert_all(&custom, json!({"label": "typed callback"}));
    let mut failed = FailedAppend {
        id: 2,
        ..Default::default()
    };
    assert!(
        serde_json::to_value(&failed)
            .unwrap_err()
            .to_string()
            .contains("accessor serialization failed")
    );
    assert_eq!(failed.to_array(), Value::Null);
    assert_eq!(failed.to_json(), "null");
    failed.make_hidden("broken");
    assert_all(&failed, json!({"id": 2}));
}

#[tokio::test]
async fn filtered_serialization_does_not_change_persistence_or_raw_originals() {
    let fixture = Fixture::sqlite().await;
    fixture.exec("CREATE TABLE gap_hidden (id INTEGER PRIMARY KEY, email TEXT NOT NULL, secret TEXT NOT NULL)").await;
    let mut model = Hidden::create(attrs! { email: "first", secret: "password" })
        .await
        .unwrap();
    let replica: Hidden = model.replicate_into().await.unwrap();
    assert_eq!(replica.secret, "password");
    assert_eq!(replica.email, "first");
    model.append("full_name").unwrap();
    model.make_hidden("email");
    model.email = "second".into();
    model.secret = "changed".into();
    model.save().await.unwrap();
    let stored = Hidden::find(model.id).await.unwrap().unwrap();
    assert_eq!(stored.email, "second");
    assert_eq!(stored.secret, "changed");
    assert_eq!(
        model.get_raw_originals().unwrap().get("secret"),
        Some(&json!("changed"))
    );
    assert_eq!(
        model.to_array(),
        json!({"id": model.id, "full_name": format!("User {}", model.id)})
    );
}
