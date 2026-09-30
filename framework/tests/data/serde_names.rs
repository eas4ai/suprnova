//! A request object whose fields serde renames is answered in the renamed
//! names everywhere the client reads a field name: the keys of validation
//! errors (nested ones included) and the messages built for them,
//! Precognition's `Validate-Only` filter, and the key a route parameter is
//! injected under. Every case sends a real HTTP request, on both of the
//! `Data` derive's `FormRequest` paths and through `#[derive(FormRequest)]`
//! and `#[request]`.

use suprnova::error::FrameworkError;
use suprnova::rules::AlphaDash;
use suprnova::{ValidationErrors, validate};
use validator::Validate;

use super::validation_hooks::{Precognition, extract, failed_keys};

#[derive(Debug, suprnova::Data, validator::Validate)]
#[serde(rename_all = "camelCase")]
struct LineData {
    #[validate(range(min = 1))]
    unit_price: i64,
}

#[derive(Debug, suprnova::Data, validator::Validate)]
#[serde(rename_all = "camelCase")]
#[data(after_validation = "order_rules")]
struct OrderData {
    #[validate(email)]
    customer_email: String,
    #[validate(nested)]
    line_items: Vec<LineData>,
    #[serde(rename = "ref")]
    reference: String,
}

fn order_rules(dto: &OrderData) -> Result<(), ValidationErrors> {
    validate! { dto =>
        reference => AlphaDash;
    }
}

/// The inlined `FormRequest` path, which a route-parameter field selects.
#[derive(Debug, suprnova::Data, validator::Validate)]
#[serde(rename_all = "camelCase")]
struct RepriceData {
    #[data(from_route_param("order"))]
    order_id: i64,
    #[validate(range(min = 1))]
    unit_price: i64,
}

#[derive(Debug, serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
#[serde(rename_all = "camelCase")]
struct SignupRequest {
    #[validate(email)]
    email_address: String,
}

#[suprnova::request]
#[derive(Debug)]
#[serde(rename_all = "camelCase")]
struct InviteRequest {
    #[validate(email)]
    invitee_email: String,
}

fn order(changes: serde_json::Value) -> serde_json::Value {
    let mut body = serde_json::json!({
        "customerEmail": "ada@example.test",
        "lineItems": [{"unitPrice": 5}, {"unitPrice": 7}],
        "ref": "a-1",
    });
    for (key, value) in changes.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

fn status(result: Result<impl std::fmt::Debug, FrameworkError>) -> u16 {
    match result {
        Err(error) => error.status_code(),
        Ok(value) => panic!("expected a refusal, got {value:?}"),
    }
}

#[tokio::test]
async fn errors_carry_the_input_names_at_every_level() {
    let result = extract::<OrderData>(
        &[],
        order(serde_json::json!({
            "customerEmail": "not an email",
            "lineItems": [{"unitPrice": 5}, {"unitPrice": 0}],
        })),
        Precognition::Off,
    )
    .await;
    let errors = match result {
        Err(FrameworkError::Validation(errors)) => errors,
        other => panic!("expected a validation failure, got {other:?}"),
    };
    let mut keys: Vec<&String> = errors.errors.keys().collect();
    keys.sort();
    assert_eq!(keys, ["customerEmail", "lineItems.1.unitPrice"]);
    // The message is built for the input name too: without a catalog it
    // names the key itself (the localized label is pinned in the
    // localization tests).
    let message = errors.messages_for("customerEmail").join(" ");
    assert!(message.contains("'customerEmail'"), "{message}");
}

#[tokio::test]
async fn a_hooks_errors_carry_the_input_names() {
    let keys = failed_keys(
        extract::<OrderData>(
            &[],
            order(serde_json::json!({"ref": "a 1!"})),
            Precognition::Off,
        )
        .await,
    );
    assert_eq!(keys, ["ref"]);
    extract::<OrderData>(&[], order(serde_json::json!({})), Precognition::Off)
        .await
        .expect("a valid order");
}

#[tokio::test]
async fn precognition_filters_by_the_input_names() {
    let body = order(serde_json::json!({
        "customerEmail": "not an email",
        "lineItems": [{"unitPrice": 0}],
    }));
    match extract::<OrderData>(
        &[],
        body.clone(),
        Precognition::Only("lineItems.0.unitPrice"),
    )
    .await
    {
        Err(FrameworkError::PrecognitionFailure(errors)) => {
            let keys: Vec<&String> = errors.errors.keys().collect();
            assert_eq!(keys, ["lineItems.0.unitPrice"]);
        }
        other => panic!("expected a filtered Precognition failure, got {other:?}"),
    }
    // A Rust name is not an input name: nothing matches it.
    assert!(matches!(
        extract::<OrderData>(&[], body, Precognition::Only("customer_email")).await,
        Err(FrameworkError::PrecognitionSuccess)
    ));
}

#[tokio::test]
async fn the_route_parameter_path_uses_the_input_names_too() {
    let dto = extract::<RepriceData>(
        &[("order", "9")],
        serde_json::json!({"unitPrice": 3}),
        Precognition::Off,
    )
    .await
    .expect("the route parameter is injected under the input name");
    assert_eq!((dto.order_id, dto.unit_price), (9, 3));
    let keys = failed_keys(
        extract::<RepriceData>(
            &[("order", "9")],
            serde_json::json!({"unitPrice": 0}),
            Precognition::Off,
        )
        .await,
    );
    assert_eq!(keys, ["unitPrice"]);
}

#[tokio::test]
async fn a_body_that_does_not_fit_answers_422_on_both_paths() {
    let unknown = serde_json::json!({"unit_price": 3});
    assert_eq!(
        status(extract::<RepriceData>(&[("order", "9")], unknown, Precognition::Off).await),
        422,
        "the route-parameter path refuses a Rust name as the default path does"
    );
    assert_eq!(
        status(
            extract::<OrderData>(
                &[],
                order(serde_json::json!({"line_items": []})),
                Precognition::Off
            )
            .await
        ),
        422
    );
}

#[tokio::test]
async fn form_request_structs_serde_derives_are_keyed_by_input_names() {
    let keys = failed_keys(
        extract::<SignupRequest>(
            &[],
            serde_json::json!({"emailAddress": "nope"}),
            Precognition::Off,
        )
        .await,
    );
    assert_eq!(keys, ["emailAddress"]);
    let keys = failed_keys(
        extract::<InviteRequest>(
            &[],
            serde_json::json!({"inviteeEmail": "nope"}),
            Precognition::Off,
        )
        .await,
    );
    assert_eq!(keys, ["inviteeEmail"]);
}

// ---- the output side ----

#[derive(suprnova::Data)]
#[serde(rename_all = "camelCase")]
struct AlbumPage {
    album_title: String,
    #[data(lazy)]
    song_list: suprnova::inertia::Prop,
}

/// A lazy prop is sent, allow-listed and included under one name, the
/// serialize name: the include gate looks the prop up by its key.
#[tokio::test]
async fn a_renamed_lazy_prop_is_included_by_its_sent_name() {
    use std::sync::Arc;
    use suprnova::data::{REQUEST_INCLUDE_SET, RequestIncludeSet, registry};
    use suprnova::inertia::{Prop, PropEntry};

    let page = AlbumPage {
        album_title: "Kind of Blue".into(),
        song_list: Prop::lazy(|| async { serde_json::json!(["So What"]) }),
    };
    let entries = page.__into_inertia_props();
    let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["albumTitle", "songList"]);
    let (_, entry) = entries
        .into_iter()
        .find(|(key, _)| key == "songList")
        .expect("the lazy prop");
    let PropEntry::LazyOwned { owner, field, prop } = entry else {
        panic!("expected a lazy prop");
    };
    assert_eq!(field, "songList");
    assert_eq!(registry::allowed_for(owner), ["songList"]);
    let include = |name: &str| {
        Arc::new(RequestIncludeSet {
            include: vec![name.to_owned()],
            ..Default::default()
        })
    };
    let resolved = REQUEST_INCLUDE_SET
        .scope(include("songList"), prop.resolve_with_owner(owner, field))
        .await
        .expect("an allowed include");
    assert_eq!(resolved, Some(serde_json::json!(["So What"])));
}

#[derive(Debug, Clone, suprnova::Data, suprnova::Validate)]
#[json_resource("people")]
#[serde(rename_all = "camelCase")]
struct PersonResource {
    id: i64,
    given_name: String,
    #[data(allow_include)]
    home_town: Option<TownResource>,
}

#[derive(Debug, Clone, suprnova::Data, suprnova::Validate)]
#[json_resource("towns")]
struct TownResource {
    id: i64,
    name: String,
}

/// JSON:API attribute and relationship names are the serialize names, and
/// a sparse fieldset names attributes by them.
#[test]
fn json_api_members_take_the_sent_names() {
    use suprnova::resources::IntoJsonResource;

    let person = PersonResource {
        id: 1,
        given_name: "Ada".into(),
        home_town: Some(TownResource {
            id: 2,
            name: "London".into(),
        }),
    };
    assert_eq!(
        person.resource_attributes(None),
        serde_json::json!({"givenName": "Ada"})
    );
    assert_eq!(
        person.resource_attributes(Some(&["given_name"])),
        serde_json::json!({}),
        "a fieldset names attributes by their sent names"
    );
    let relationships: Vec<String> = person
        .resource_relationships()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(relationships, ["homeTown"]);
    assert_eq!(
        person.home_town.map(|town| (town.id, town.name)),
        Some((2, "London".into()))
    );
}
