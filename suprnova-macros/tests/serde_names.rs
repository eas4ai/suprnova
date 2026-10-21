//! `#[derive(Data)]` and `#[derive(InertiaProps)]` against serde's own
//! derive. For every serde attribute the two honor, a twin struct derived
//! with serde must write the same keys, and a `Data` struct must read the
//! keys the twin writes and refuse the ones serde refuses.

use serde::{Deserialize, Serialize};
use serde_json::json;
use suprnova::{Data, InertiaProps};

macro_rules! rule_twins {
    ($module:ident, $rule:literal) => {
        mod $module {
            use super::*;

            #[derive(Data, validator::Validate, Debug)]
            #[serde(rename_all = $rule)]
            pub struct DataTwin {
                pub display_name: String,
                pub unit_price_cents: i64,
            }

            #[derive(InertiaProps)]
            #[serde(rename_all = $rule)]
            pub struct PropsTwin {
                pub display_name: String,
                pub unit_price_cents: i64,
            }

            #[derive(Serialize, Deserialize, Debug)]
            #[serde(rename_all = $rule, deny_unknown_fields)]
            pub struct SerdeTwin {
                pub display_name: String,
                pub unit_price_cents: i64,
            }

            #[test]
            fn both_derives_use_serdes_keys() {
                let serde = serde_json::to_value(SerdeTwin {
                    display_name: "Ada".into(),
                    unit_price_cents: 5,
                })
                .unwrap();
                let data: DataTwin =
                    serde_json::from_value(serde.clone()).expect("Data reads serde's keys");
                assert_eq!(
                    serde_json::to_value(&data).unwrap(),
                    serde,
                    "Data writes them"
                );
                let props = serde_json::to_value(PropsTwin {
                    display_name: "Ada".into(),
                    unit_price_cents: 5,
                })
                .unwrap();
                assert_eq!(props, serde, "InertiaProps writes them");
            }
        }
    };
}

rule_twins!(lowercase, "lowercase");
rule_twins!(uppercase, "UPPERCASE");
rule_twins!(pascal_case, "PascalCase");
rule_twins!(camel_case, "camelCase");
rule_twins!(snake_case, "snake_case");
rule_twins!(screaming_snake_case, "SCREAMING_SNAKE_CASE");
rule_twins!(kebab_case, "kebab-case");
rule_twins!(screaming_kebab_case, "SCREAMING-KEBAB-CASE");

#[derive(Data, validator::Validate, Debug)]
#[serde(rename_all = "camelCase")]
struct ProfileData {
    display_name: String,
    #[serde(rename = "id")]
    profile_id: i64,
    #[serde(skip)]
    cache: String,
    #[serde(skip_serializing)]
    secret: String,
    #[serde(skip_deserializing)]
    computed: String,
    r#type: String,
    // Named like the visitor's own variables: they must not shadow them.
    key: String,
    map: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileSerde {
    display_name: String,
    #[serde(rename = "id")]
    profile_id: i64,
    #[serde(skip)]
    cache: String,
    #[serde(skip_serializing)]
    secret: String,
    #[serde(skip_deserializing)]
    computed: String,
    r#type: String,
    key: String,
    map: String,
}

fn profile_input() -> serde_json::Value {
    json!({
        "displayName": "Ada",
        "id": 7,
        "secret": "s3cret",
        "type": "admin",
        "key": "k",
        "map": "m",
    })
}

#[test]
fn renames_skips_and_raw_identifiers_match_serde() {
    let data: ProfileData = serde_json::from_value(profile_input()).expect("Data reads the input");
    let serde: ProfileSerde =
        serde_json::from_value(profile_input()).expect("serde reads the input");
    assert_eq!(
        (data.secret.as_str(), data.key.as_str(), data.map.as_str()),
        ("s3cret", "k", "m")
    );
    assert_eq!(data.cache, "");
    assert_eq!(data.computed, "");
    let written = serde_json::to_value(&data).unwrap();
    assert_eq!(written, serde_json::to_value(&serde).unwrap());
    assert_eq!(
        written,
        json!({"displayName": "Ada", "id": 7, "computed": "", "type": "admin", "key": "k", "map": "m"})
    );
    assert_eq!(
        (serde.secret.as_str(), serde.cache.as_str()),
        ("s3cret", "")
    );
}

#[test]
fn a_key_serde_refuses_the_derive_refuses() {
    for (label, input) in [
        (
            "the Rust name of a renamed field",
            json!({"display_name": "Ada"}),
        ),
        (
            "a field serde skips when deserializing",
            json!({"computed": "x"}),
        ),
        ("a field serde skips altogether", json!({"cache": "x"})),
    ] {
        let mut body = profile_input();
        body.as_object_mut()
            .unwrap()
            .extend(input.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<ProfileSerde>(body.clone()).is_err(),
            "serde refuses {label}"
        );
        let error = serde_json::from_value::<ProfileData>(body)
            .expect_err(label)
            .to_string();
        assert!(error.contains("unknown field"), "{label}: {error}");
    }
    let missing = serde_json::from_value::<ProfileData>(json!({"id": 7}))
        .expect_err("a required field is missing")
        .to_string();
    assert!(missing.contains("`displayName`"), "{missing}");
}

#[derive(Data, validator::Validate, Debug)]
#[serde(rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
struct SplitData {
    display_name: String,
    #[serde(rename(serialize = "out", deserialize = "in"))]
    moved: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(
    rename_all(serialize = "camelCase", deserialize = "kebab-case"),
    deny_unknown_fields
)]
struct SplitSerde {
    display_name: String,
    #[serde(rename(serialize = "out", deserialize = "in"))]
    moved: String,
}

#[test]
fn each_direction_takes_its_own_name() {
    let input = json!({"display-name": "Ada", "in": "x"});
    let data: SplitData =
        serde_json::from_value(input.clone()).expect("Data reads the input names");
    let serde: SplitSerde = serde_json::from_value(input).expect("serde reads them");
    let written = serde_json::to_value(&data).unwrap();
    assert_eq!(written, serde_json::to_value(&serde).unwrap());
    assert_eq!(written, json!({"displayName": "Ada", "out": "x"}));
    assert!(
        serde_json::from_value::<SplitData>(json!({"displayName": "Ada", "out": "x"})).is_err()
    );
}

#[derive(InertiaProps)]
struct RawProps {
    r#type: String,
    #[serde(skip)]
    internal: String,
    #[serde(skip_serializing)]
    hidden: String,
}

#[test]
fn inertia_props_skip_and_unraw_like_serde() {
    let props = RawProps {
        r#type: "admin".into(),
        internal: "x".into(),
        hidden: "y".into(),
    };
    assert_eq!(
        serde_json::to_value(&props).unwrap(),
        json!({"type": "admin"})
    );
    let _ = (&props.internal, &props.hidden);
}

#[derive(Data, validator::Validate, Debug)]
#[data(allow_unknown_fields)]
struct LenientData {
    name: String,
    #[serde(skip_deserializing)]
    computed: String,
    #[data(output_only)]
    handle: String,
}

#[test]
fn a_lenient_struct_still_never_sets_a_skipped_or_output_only_field() {
    let data: LenientData =
        serde_json::from_value(json!({"name": "Ada", "computed": "x", "extra": 1}))
            .expect("unknown keys and a skipped field's key are dropped");
    assert_eq!((data.name.as_str(), data.computed.as_str()), ("Ada", ""));
    let error = serde_json::from_value::<LenientData>(json!({"name": "Ada", "handle": "@ada"}))
        .expect_err("an output_only key is still refused")
        .to_string();
    assert!(error.contains("output_only"), "{error}");
    assert_eq!(data.handle, "");
}
