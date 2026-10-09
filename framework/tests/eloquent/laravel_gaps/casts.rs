//! Enum collections and originals preserve the database representation.

use crate::query_fixture::Fixture;
use serde_json::{Value, json};
use suprnova::{AsBool, AsEnumCollection, Cast, IntoDynCast, Model, model};

/// Serde names differ from storage so the test cannot pass by generic enum serialization.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    strum::EnumString,
    strum::AsRefStr,
)]
pub enum Status {
    #[default]
    #[strum(serialize = "draft")]
    Draft,
    #[strum(serialize = "live")]
    Live,
}

/// Cast fields give originals a distinct stored shape to preserve.
#[model(table = "gap_enum_lists", timestamps = false, casts = {
    statuses = AsEnumCollection<Status>, active = AsBool,
})]
pub struct EnumList {
    pub id: i64,
    pub statuses: Vec<Status>,
    pub active: bool,
    pub note: Option<String>,
}

async fn fixture() -> Fixture {
    let fixture = Fixture::sqlite().await;
    fixture.exec("CREATE TABLE gap_enum_lists (id INTEGER PRIMARY KEY, statuses TEXT NOT NULL, active INTEGER NOT NULL, note TEXT NULL)").await;
    fixture
        .exec("INSERT INTO gap_enum_lists VALUES (1, '[\"draft\",\"live\"]', 1, NULL)")
        .await;
    fixture
}

#[tokio::test]
async fn laravel_enum_list_reads_and_writes_only_storage_strings() {
    let _fixture = fixture().await;
    let mut model = EnumList::find(1).await.unwrap().unwrap();
    assert_eq!(model.statuses, vec![Status::Draft, Status::Live]);
    model.statuses = vec![Status::Live, Status::Draft, Status::Live];
    model.save().await.unwrap();
    let reloaded = EnumList::find(1).await.unwrap().unwrap();
    assert_eq!(reloaded.statuses, model.statuses);
    assert_eq!(
        reloaded.get_raw_original("statuses"),
        Some(json!("[\"live\",\"draft\",\"live\"]"))
    );
    model.statuses.clear();
    model.save().await.unwrap();
    assert!(
        EnumList::find(1)
            .await
            .unwrap()
            .unwrap()
            .statuses
            .is_empty()
    );
    assert_eq!(model.get_raw_original("statuses"), Some(json!("[]")));
}

#[test]
fn enum_collection_handles_empty_duplicates_and_rejects_invalid_storage() {
    assert!(
        AsEnumCollection::<Status>::from_storage(&"[]".to_owned())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        AsEnumCollection::<Status>::to_storage(&vec![Status::Draft, Status::Draft]).unwrap(),
        "[\"draft\",\"draft\"]"
    );
    for invalid in ["not json", "null", "{}", "[1]", "[\"unknown\"]"] {
        assert!(
            AsEnumCollection::<Status>::from_storage(&invalid.to_owned()).is_err(),
            "{invalid}"
        );
    }
    let dynamic = AsEnumCollection::<Status>::into_dyn();
    assert_eq!(
        dynamic
            .from_storage_json(&json!("[\"draft\",\"live\"]"))
            .unwrap(),
        json!(["Draft", "Live"])
    );
    assert_eq!(
        dynamic.to_storage_json(&json!(["Draft", "Live"])).unwrap(),
        json!("[\"draft\",\"live\"]")
    );
    assert!(dynamic.from_storage_json(&json!(["draft"])).is_err());
    assert!(dynamic.to_storage_json(&json!(["unknown"])).is_err());
}

#[tokio::test]
async fn invalid_enum_column_returns_an_error_from_model_reads() {
    let fixture = fixture().await;
    fixture
        .exec("UPDATE gap_enum_lists SET statuses = '[\"unknown\"]' WHERE id = 1")
        .await;
    let error = EnumList::find(1).await.unwrap_err().to_string();
    assert!(error.contains("statuses"));
    assert!(error.contains("AsEnumCollection"));
}

#[tokio::test]
async fn raw_originals_include_every_loaded_value_before_casts_and_ignore_mutations() {
    let _fixture = fixture().await;
    let mut model = EnumList::find(1).await.unwrap().unwrap();
    model.statuses.clear();
    model.active = false;
    let originals = model.get_raw_originals().unwrap();
    assert_eq!(originals.len(), 4);
    assert_eq!(originals.get("id"), Some(&json!(1)));
    assert_eq!(
        originals.get("statuses"),
        Some(&json!("[\"draft\",\"live\"]"))
    );
    assert_eq!(originals.get("active"), Some(&json!(1)));
    assert_eq!(originals.get("note"), Some(&Value::Null));
    assert_eq!(model.get_original("active").unwrap(), Some(json!(true)));
    assert_eq!(model.get_raw_original_or("missing", 7), json!(7));
    assert_eq!(model.get_raw_original_or("note", 7), Value::Null);
    assert_eq!(model.get_raw_original_or("active", 7), json!(1));
    let unsaved = EnumList::default();
    assert!(unsaved.get_raw_originals().unwrap().is_empty());
    assert_eq!(
        unsaved.get_raw_original_or("active", "absent"),
        json!("absent")
    );
}
