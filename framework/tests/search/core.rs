//! Core schemas and settings exercise boundaries without an external driver.
use std::collections::{BTreeMap, BTreeSet};
use suprnova::{
    AttributeType, AttributeValue, GeoPoint, IndexSettings, SearchDocument, SettingsChange,
    Synonym, WeightedAttribute,
};

fn document() -> SearchDocument {
    SearchDocument {
        index: "documents".into(),
        key: "7".into(),
        source_version: 10,
        attributes: BTreeMap::from([
            (
                "title".into(),
                AttributeValue::Text("A <b>title</b>".into()),
            ),
            ("number".into(), AttributeValue::Integer(i64::MAX)),
            ("ratio".into(), AttributeValue::Float(1.25)),
            ("flag".into(), AttributeValue::Bool(true)),
            (
                "when".into(),
                AttributeValue::DateTime("2026-10-14T12:00:00Z".parse().unwrap()),
            ),
            (
                "geo".into(),
                AttributeValue::Geo(GeoPoint {
                    lat: 40.0,
                    lng: -73.0,
                }),
            ),
            (
                "tags".into(),
                AttributeValue::List(vec![AttributeValue::Text("rust".into())]),
            ),
            ("absent".into(), AttributeValue::Null),
        ]),
        embed_text: Some("Body".into()),
        title: Some("Title".into()),
    }
}

#[test]
fn typed_documents_round_trip_all_attribute_types() {
    let original = document();
    assert_eq!(
        serde_json::from_str::<SearchDocument>(&serde_json::to_string(&original).unwrap()).unwrap(),
        original
    );
}

#[test]
fn content_hash_ignores_freshness_but_tracks_content_and_generation_settings() {
    let mut doc = document();
    let hash = doc.content_hash("settings");
    doc.source_version += 1;
    assert_eq!(doc.content_hash("settings"), hash);
    assert_ne!(doc.content_hash("different settings"), hash);
    doc.attributes
        .insert("title".into(), AttributeValue::Text("New".into()));
    assert_ne!(doc.content_hash("settings"), hash);
}

#[test]
fn content_hash_uses_canonical_json_followed_by_settings_digest() {
    use sha2::{Digest, Sha256};
    let doc = SearchDocument {
        index: "a".into(),
        key: "b".into(),
        source_version: 1,
        attributes: BTreeMap::new(),
        embed_text: None,
        title: None,
    };
    let bytes = "{\"attributes\":{},\"embed_text\":null,\"index\":\"a\",\"key\":\"b\",\"title\":null}digest";
    assert_eq!(
        doc.content_hash("digest"),
        hex::encode(Sha256::digest(bytes.as_bytes()))
    );
}

#[test]
fn attribute_conversion_rejects_wrong_types_overflow_and_invalid_geo() {
    for (value, ty, field) in [
        (serde_json::json!(u64::MAX), AttributeType::Integer, "count"),
        (serde_json::json!("true"), AttributeType::Bool, "enabled"),
        (
            serde_json::json!("bad date"),
            AttributeType::DateTime,
            "published",
        ),
        (
            serde_json::json!({"lat":91,"lng":0}),
            AttributeType::Geo,
            "location",
        ),
        (serde_json::json!([1]), AttributeType::TextList, "tags"),
    ] {
        assert!(
            AttributeValue::from_json(&value, ty, field)
                .unwrap_err()
                .to_string()
                .contains(field)
        );
    }
}

#[test]
fn integer_comparisons_keep_precision_above_float_exactness() {
    assert_eq!(
        AttributeValue::Integer(i64::MAX).compare(&AttributeValue::Integer(i64::MAX - 1)),
        Some(std::cmp::Ordering::Greater)
    );
}

#[test]
fn naive_and_offset_timestamps_convert_to_utc() {
    let a = AttributeValue::from_json(
        &serde_json::json!("2026-10-14T12:00:00"),
        AttributeType::DateTime,
        "time",
    )
    .unwrap();
    let b = AttributeValue::from_json(
        &serde_json::json!("2026-10-14T14:00:00+02:00"),
        AttributeType::DateTime,
        "time",
    )
    .unwrap();
    assert_eq!(a, b);
}

#[test]
fn haversine_handles_dateline_and_identical_points() {
    let a = GeoPoint {
        lat: 0.0,
        lng: 179.0,
    };
    let b = GeoPoint {
        lat: 0.0,
        lng: -179.0,
    };
    assert!(a.distance_to(&b) > 222_000.0 && a.distance_to(&b) < 223_000.0);
    assert_eq!(a.distance_to(&a), 0.0);
    assert!(
        GeoPoint {
            lat: f64::NAN,
            lng: 0.0
        }
        .validate()
        .is_err()
    );
}

#[test]
fn query_settings_changes_do_not_change_the_index_digest() {
    let old = IndexSettings::default();
    let mut new = old.clone();
    new.query.stop_words = BTreeSet::from(["the".into()]);
    new.query.hybrid_ratio = 0.5;
    let changed = SettingsChange::between(&old, &new);
    assert_eq!(changed.query_time, vec!["hybrid_ratio", "stop_words"]);
    assert!(!changed.requires_generation());
    assert_eq!(old.index_time.digest(), new.index_time.digest());
}

#[test]
fn each_index_time_field_is_classified_and_changes_the_digest() {
    let old = IndexSettings::default();
    let cases = [
        (
            "language",
            serde_json::json!({"engine":"postgres","configuration":"english"}),
        ),
        (
            "searchable_attributes",
            serde_json::json!([{"name":"title","weight":2.0}]),
        ),
        (
            "embed_source",
            serde_json::json!({"attributes":["body"],"title":"title"}),
        ),
        (
            "embed_template",
            serde_json::json!("title: {title} | text: {text}"),
        ),
        (
            "filterable_attributes",
            serde_json::json!({"price":"float"}),
        ),
        ("sortable_attributes", serde_json::json!({"price":"float"})),
        ("facet_attributes", serde_json::json!({"price":"float"})),
        ("embedder", serde_json::json!("http")),
        ("model", serde_json::json!("bge-small-en-v1.5")),
        ("dimensions", serde_json::json!(384)),
    ];
    for (field, value) in cases {
        let mut json = serde_json::to_value(&old).unwrap();
        json["index_time"][field] = value;
        let new: IndexSettings = serde_json::from_value(json).unwrap();
        assert_eq!(
            SettingsChange::between(&old, &new).index_time,
            vec![field],
            "{field}"
        );
        assert_ne!(old.index_time.digest(), new.index_time.digest(), "{field}");
    }
}

#[test]
fn settings_readback_preserves_every_typed_field() {
    let mut settings = IndexSettings::default();
    settings.query.synonyms.push(Synonym {
        terms: vec!["new york".into(), "nyc".into()],
        one_way: false,
    });
    settings
        .index_time
        .searchable_attributes
        .push(WeightedAttribute {
            name: "title".into(),
            weight: 2.0,
        });
    settings
        .index_time
        .filterable_attributes
        .insert("tags".into(), AttributeType::TextList);
    assert_eq!(
        serde_json::from_str::<IndexSettings>(&serde_json::to_string(&settings).unwrap()).unwrap(),
        settings
    );
}

#[test]
fn invalid_settings_fail_without_accepting_executable_markup() {
    let mut settings = IndexSettings::default();
    settings.query.highlight_pre_tag = "<em onclick='run()'>".into();
    assert!(settings.validate().is_err());
    let mut settings = IndexSettings::default();
    settings.query.hybrid_ratio = f32::NAN;
    assert!(settings.validate().is_err());
    let mut settings = IndexSettings::default();
    settings.query.typo_tolerance.two_edit_min_length = 1;
    assert!(settings.validate().is_err());
    let mut settings = IndexSettings::default();
    settings.index_time.dimensions = Some(0);
    assert!(settings.validate().is_err());
    let mut settings = IndexSettings::default();
    settings
        .index_time
        .facet_attributes
        .insert("tag".into(), AttributeType::Text);
    assert!(settings.validate().is_err());
}

#[test]
fn facet_identity_treats_signed_zero_as_one_value_for_numbers_and_coordinates() {
    assert_eq!(
        AttributeValue::Float(-0.0).facet_key(),
        AttributeValue::Float(0.0).facet_key()
    );
    assert_eq!(
        AttributeValue::Geo(GeoPoint {
            lat: -0.0,
            lng: 0.0
        })
        .facet_key(),
        AttributeValue::Geo(GeoPoint {
            lat: 0.0,
            lng: -0.0
        })
        .facet_key()
    );
}
