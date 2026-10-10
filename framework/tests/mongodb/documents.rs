//! PAR-183: `#[suprnova::document(collection = "...")]` generates a document
//! model with the Eloquent shape: create, find, find_or_fail, all, query,
//! save, update, delete, fresh and refresh; an `_id` of `ObjectId` unless a
//! field is declared the key; fillable and guarded with the `#[model]`
//! syntax; casts where BSON has the type; managed timestamps; soft deletes;
//! embedded documents; the array operators; the model events through the
//! observer shape; serde serialization honouring hidden and visible; and
//! route binding by the key.
//!
//! The tests without the `mongodb_` prefix run without a server: the
//! generated metadata, the BSON a model writes and reads, the serialized
//! form, the fillable filter, route keys, the events that fire before any
//! write, and the errors that come before the server is needed. The
//! `mongodb_` tests need `MONGODB_TEST_URL` and are ignored without it.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use suprnova::bson::oid::ObjectId;
use suprnova::bson::{Bson, Document, doc};
use suprnova::events::{EventFacade, Listener};
use suprnova::mongodb::events::{
    Created, Creating, Deleted, Deleting, Restored, Saved, Saving, Updated, Updating,
};
use suprnova::{
    AsBsonDateTime, CancellableListener, DocumentCast, DocumentKey, DocumentModel,
    DocumentObserver, EventResult, FrameworkError, Mongo, MongoConfig, RouteBinding,
};

use suprnova::mongodb::{__rendered_array_update, __rendered_embedded_update};

use crate::support::{database_of, test_url};

// --- Models ----------------------------------------------------------------

/// An address embedded in a member's `addresses` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Address {
    pub street: String,
    pub city: String,
}

/// A profile embedded as one document in a member.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub bio: String,
}

/// A unit enum, stored as its serde name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    Free,
    Pro,
}

/// A member: an `ObjectId` key the macro adds, a hidden password, casts,
/// embedded documents, timestamps and soft deletes.
#[suprnova::document(
    collection = "m2_members",
    fillable = [
        "name", "email", "password", "tags", "plan", "balance", "joined_at",
        "settings", "logins", "addresses", "profile"
    ],
    hidden = ["password"],
    soft_deletes
)]
pub struct Member {
    pub name: String,
    pub email: String,
    pub password: Option<String>,
    pub is_admin: Option<bool>,
    pub tags: Vec<String>,
    pub plan: Option<Plan>,
    pub balance: Option<Decimal>,
    pub joined_at: Option<DateTime<Utc>>,
    pub settings: Option<Document>,
    pub logins: i64,
    #[embeds_many]
    pub addresses: Vec<Address>,
    #[embeds_one]
    pub profile: Option<Profile>,
    pub created_at: Option<suprnova::bson::DateTime>,
    pub updated_at: Option<suprnova::bson::DateTime>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A product whose key is its SKU, stored as `_id`; no timestamps.
#[suprnova::document(collection = "m2_products", primary_key = "sku", fillable = ["sku", "title"])]
pub struct Product {
    pub sku: String,
    pub title: String,
}

/// A model that asks for its timestamps with `timestamps = true`.
#[suprnova::document(collection = "m2_stamped", fillable = ["tags"], timestamps = true)]
pub struct Stamped {
    pub tags: Vec<String>,
    pub created_at: Option<suprnova::bson::DateTime>,
    pub updated_at: Option<suprnova::bson::DateTime>,
}

/// A model without timestamps that embeds documents: its embedded
/// relations send their operator alone.
#[suprnova::document(collection = "m2_itineraries", timestamps = false)]
pub struct Itinerary {
    #[embeds_many]
    pub stops: Vec<Address>,
    #[embeds_one]
    pub origin: Option<Address>,
}

/// A card that serializes only the fields `visible` lists.
#[suprnova::document(collection = "m2_cards", visible = ["title"], timestamps = false)]
pub struct Card {
    pub title: String,
    pub secret: String,
}

/// A model whose `creating` listener cancels every create.
#[suprnova::document(collection = "m2_vetoed")]
pub struct Vetoed {
    pub name: String,
}

/// A model observed through `#[suprnova::observer]`.
#[suprnova::document(collection = "m2_watched")]
pub struct Watched {
    pub name: String,
}

/// A private model: its event type names are as private as it is.
#[suprnova::document(collection = "m2_private_notes")]
struct PrivateNote {
    text: String,
}

/// A crate-visible model.
#[suprnova::document(collection = "m2_crate_notes")]
pub(crate) struct CrateNote {
    pub(crate) text: String,
}

fn member_attributes() -> Document {
    doc! {
        "name": "Ada",
        "email": "ada@example.com",
        "password": "secret-hash",
        "tags": ["math"],
        "plan": "pro",
        "balance": "12.50",
        "joined_at": "2026-10-10T08:30:00Z",
        "settings": { "theme": "dark", "nested": { "level": 2 } },
        "logins": 3_i64,
        "addresses": [{ "street": "1 Analytical Way", "city": "London" }],
        "profile": { "bio": "Countess" },
    }
}

fn names(error: &FrameworkError, what: &str) -> bool {
    error.to_string().contains(what)
}

// --- The generated model ---------------------------------------------------

#[test]
fn the_attribute_records_the_collection_the_key_and_the_managed_fields() {
    assert_eq!(Member::COLLECTION, "m2_members");
    assert_eq!(Member::CONNECTION, None);
    assert_eq!(Member::KEY_FIELD, "id");
    assert_eq!(Member::TIMESTAMPS, Some(("created_at", "updated_at")));
    assert_eq!(Member::SOFT_DELETES, Some("deleted_at"));
    assert_eq!(Member::HIDDEN, &["password"]);
    assert!(Member::FIELDS.contains(&"id"));
    assert!(Member::FIELDS.contains(&"addresses"));

    assert_eq!(Product::KEY_FIELD, "sku");
    assert_eq!(
        Product::TIMESTAMPS,
        None,
        "no timestamp fields, no timestamps"
    );
    assert_eq!(Product::SOFT_DELETES, None);
    assert_eq!(Card::VISIBLE, Some(&["title"][..]));
}

#[test]
fn explicit_timestamps_with_both_fields_are_managed() {
    assert_eq!(Stamped::TIMESTAMPS, Some(("created_at", "updated_at")));
}

#[test]
fn the_array_operators_set_updated_at_in_the_same_update_on_a_model_with_timestamps() {
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    for operator in ["$push", "$addToSet", "$pull"] {
        let update = __rendered_array_update::<Member, _>(operator, "tags", &"a", now)
            .expect("render the update");
        let mut expected = Document::new();
        expected.insert(operator, doc! { "tags": "a" });
        expected.insert("$set", doc! { "updated_at": now });
        assert_eq!(update, expected, "{operator}");

        let update = __rendered_array_update::<Stamped, _>(operator, "tags", &"a", now)
            .expect("render the update");
        assert_eq!(
            update.get_document("$set").expect("a `$set`"),
            &doc! { "updated_at": now },
            "`timestamps = true` manages them too: {update}"
        );
    }

    // A dotted path into an embedded document keeps the `$set` beside it.
    let update = __rendered_array_update::<Member, _>(
        "$push",
        "settings.labels",
        &doc! { "name": "x" },
        now,
    )
    .expect("render a dotted path");
    assert_eq!(
        update,
        doc! {
            "$push": { "settings.labels": { "name": "x" } },
            "$set": { "updated_at": now },
        }
    );
}

#[test]
fn the_array_operators_send_the_operator_alone_on_a_model_without_timestamps() {
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    for operator in ["$push", "$addToSet", "$pull"] {
        let update = __rendered_array_update::<Product, _>(operator, "title", &"x", now)
            .expect("render the update");
        let mut expected = Document::new();
        expected.insert(operator, doc! { "title": "x" });
        assert_eq!(update, expected, "{operator}");
    }
}

#[test]
fn an_array_update_on_the_key_an_unknown_field_or_another_operator_is_refused() {
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    let error =
        __rendered_array_update::<Member, _>("$push", "id", &"x", now).expect_err("the key");
    assert!(names(&error, "push") && names(&error, "`id`"), "{error}");
    let error = __rendered_array_update::<Member, _>("$addToSet", "nicknames", &"x", now)
        .expect_err("no such field");
    assert!(
        names(&error, "push_unique") && names(&error, "nicknames"),
        "{error}"
    );
    let error = __rendered_array_update::<Member, _>("$set", "tags", &"x", now)
        .expect_err("no array operator");
    assert!(names(&error, "`$set`"), "{error}");
}

#[test]
fn the_embedded_relations_set_updated_at_in_the_same_update_on_a_model_with_timestamps() {
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    let address = doc! { "street": "2 Engine Row", "city": "Leeds" };

    // `EmbedsMany::save` and `destroy`: the operator, and a `$set` of
    // `updated_at` beside it.
    for operator in ["$push", "$pull"] {
        let update = __rendered_embedded_update::<Member>(
            operator,
            "addresses",
            Bson::Document(address.clone()),
            now,
        )
        .expect("render the update");
        let mut expected = Document::new();
        expected.insert(operator, doc! { "addresses": address.clone() });
        expected.insert("$set", doc! { "updated_at": now });
        assert_eq!(update, expected, "{operator}");
    }

    // `EmbedsMany::save_many`: one `$push` with `$each`.
    let update = __rendered_embedded_update::<Member>(
        "$push",
        "addresses",
        Bson::Document(doc! { "$each": [address] }),
        now,
    )
    .expect("render save_many");
    assert_eq!(
        update,
        doc! {
            "$push": { "addresses": { "$each": [{ "street": "2 Engine Row", "city": "Leeds" }] } },
            "$set": { "updated_at": now },
        }
    );

    // `EmbedsOne::save`, `EmbedsOne::delete` and `EmbedsMany::clear` write
    // with `$set`. An update holds one `$set`, so `updated_at` joins it and
    // the embedded value stays.
    let cases = [
        ("profile", Bson::Document(doc! { "bio": "Analyst" })),
        ("profile", Bson::Null),
        ("addresses", Bson::Array(Vec::new())),
    ];
    for (field, value) in cases {
        let update = __rendered_embedded_update::<Member>("$set", field, value.clone(), now)
            .expect("render a `$set`");
        let mut set = Document::new();
        set.insert(field, value);
        set.insert("updated_at", now);
        let mut expected = Document::new();
        expected.insert("$set", set);
        assert_eq!(update, expected, "{field}");
    }
}

#[test]
fn the_embedded_relations_send_the_operator_alone_on_a_model_without_timestamps() {
    assert_eq!(Itinerary::TIMESTAMPS, None);
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    let stop = doc! { "street": "3 Mill Lane", "city": "York" };
    for (operator, field) in [("$push", "stops"), ("$pull", "stops"), ("$set", "origin")] {
        let update = __rendered_embedded_update::<Itinerary>(
            operator,
            field,
            Bson::Document(stop.clone()),
            now,
        )
        .expect("render the update");
        let mut set = Document::new();
        set.insert(field, stop.clone());
        let mut expected = Document::new();
        expected.insert(operator, set);
        assert_eq!(update, expected, "{operator}");
    }
}

#[test]
fn an_embedded_update_with_another_operator_is_refused_naming_it() {
    let now = suprnova::bson::DateTime::from_millis(1_760_090_400_000);
    let error = __rendered_embedded_update::<Member>("$addToSet", "addresses", Bson::Null, now)
        .expect_err("no embedded-document operator");
    assert!(names(&error, "`$addToSet`"), "{error}");
}

#[test]
fn a_private_or_crate_visible_model_names_its_events_at_its_own_visibility() {
    let note = PrivateNote::make(doc! { "text": "x" }).expect("make");
    assert_eq!(note.text, "x");
    let created: Option<private_note::events::Created> = None;
    assert!(created.is_none());
    let note = CrateNote::make(doc! { "text": "y" }).expect("make");
    assert_eq!(note.text, "y");
    let saved: Option<crate_note::events::Saved> = None;
    assert!(saved.is_none());
}

#[test]
fn without_a_declared_key_the_model_gets_an_object_id_stored_as_underscore_id() {
    let member = Member::make(member_attributes()).expect("make");
    let key: &ObjectId = member.key();
    assert_eq!(key, &member.id);

    let stored = member.to_document().expect("to_document");
    assert_eq!(stored.get("_id"), Some(&Bson::ObjectId(member.id)));
    assert!(
        !stored.contains_key("id"),
        "the key is stored as `_id` only: {stored}"
    );
    // A second model gets another key.
    let other = Member::make(member_attributes()).expect("make");
    assert_ne!(member.id, other.id);
}

#[test]
fn a_declared_key_is_stored_as_underscore_id_and_must_be_given() {
    let product = Product::make(doc! { "sku": "A-1", "title": "Anvil" }).expect("make");
    assert_eq!(product.key(), "A-1");
    let stored = product.to_document().expect("to_document");
    assert_eq!(stored.get_str("_id").expect("_id"), "A-1");
    assert!(!stored.contains_key("sku"), "{stored}");

    // Read back from storage, `_id` fills the declared key.
    let read = Product::from_document(stored).expect("from_document");
    assert_eq!(read.sku, "A-1");

    // A key with nothing to generate it must be given.
    let error = Product::make(doc! { "title": "Anvil" }).expect_err("no key");
    assert!(names(&error, "sku"), "{error}");
}

#[test]
fn make_leaves_out_a_field_outside_fillable() {
    let mut attributes = member_attributes();
    attributes.insert("is_admin", true);
    let member = Member::make(attributes).expect("make");
    assert_eq!(member.is_admin, None, "`is_admin` is not fillable");
    assert_eq!(member.name, "Ada");
}

#[test]
fn an_attribute_that_names_no_field_is_an_error_naming_it() {
    let mut attributes = member_attributes();
    attributes.insert("nickname", "Ada");
    // `Member` lists its fillable fields, so the guard drops it silently;
    // a model that guards only its key reaches the field check.
    let member = Member::make(attributes).expect("the fillable list drops it");
    assert_eq!(member.name, "Ada");

    let error = Card::make(doc! { "title": "x", "secret": "y", "colour": "red" })
        .expect_err("`colour` is no field of Card");
    assert!(names(&error, "colour"), "{error}");
}

#[test]
fn a_value_of_the_wrong_type_is_an_error_naming_the_field() {
    let mut attributes = member_attributes();
    attributes.insert("logins", "many");
    let error = Member::make(attributes).expect_err("`logins` is an i64");
    assert!(names(&error, "logins"), "{error}");
}

#[test]
fn casts_store_dates_decimals_enums_and_nested_values_as_bson_types() {
    let member = Member::make(member_attributes()).expect("make");
    assert_eq!(
        member.joined_at,
        Some(Utc.with_ymd_and_hms(2026, 10, 10, 8, 30, 0).unwrap())
    );
    assert_eq!(member.balance, Some(Decimal::new(1250, 2)));
    assert_eq!(member.plan, Some(Plan::Pro));

    let stored = member.to_document().expect("to_document");
    assert!(
        matches!(stored.get("joined_at"), Some(Bson::DateTime(_))),
        "a date is a BSON datetime: {stored}"
    );
    assert!(
        matches!(stored.get("balance"), Some(Bson::Decimal128(_))),
        "a decimal is a Decimal128: {stored}"
    );
    assert_eq!(stored.get_str("plan").expect("plan"), "pro");
    assert_eq!(
        stored
            .get_document("settings")
            .expect("settings")
            .get_document("nested")
            .expect("nested")
            .get_i32("level")
            .expect("level"),
        2,
        "a nested value is kept as BSON"
    );

    let read = Member::from_document(stored).expect("from_document");
    assert_eq!(read.joined_at, member.joined_at);
    assert_eq!(read.balance, member.balance);
    assert_eq!(read.plan, member.plan);
    assert_eq!(read.settings, member.settings);
}

#[test]
fn the_date_time_cast_reads_a_bson_datetime_and_an_rfc_3339_string() {
    let moment = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let stored =
        <AsBsonDateTime as DocumentCast<DateTime<Utc>>>::to_bson(&moment).expect("to_bson");
    assert_eq!(
        stored,
        Bson::DateTime(suprnova::bson::DateTime::from_millis(
            moment.timestamp_millis()
        ))
    );
    let read: DateTime<Utc> =
        <AsBsonDateTime as DocumentCast<DateTime<Utc>>>::from_bson(stored).expect("from_bson");
    assert_eq!(read, moment);
    let parsed: DateTime<Utc> = <AsBsonDateTime as DocumentCast<DateTime<Utc>>>::from_bson(
        Bson::String("2026-01-02T03:04:05Z".into()),
    )
    .expect("an RFC 3339 string");
    assert_eq!(parsed, moment);
    let error = <AsBsonDateTime as DocumentCast<DateTime<Utc>>>::from_bson(Bson::Int32(7))
        .expect_err("a number is no date");
    assert!(names(&error, "date"), "{error}");
}

#[test]
fn embedded_documents_round_trip_through_bson() {
    let member = Member::make(member_attributes()).expect("make");
    assert_eq!(
        member.addresses,
        vec![Address {
            street: "1 Analytical Way".into(),
            city: "London".into()
        }]
    );
    assert_eq!(
        member.profile,
        Some(Profile {
            bio: "Countess".into()
        })
    );
    let stored = member.to_document().expect("to_document");
    let addresses = stored.get_array("addresses").expect("an array");
    assert!(matches!(addresses.first(), Some(Bson::Document(_))));
    assert!(matches!(stored.get("profile"), Some(Bson::Document(_))));

    let read = Member::from_document(stored).expect("from_document");
    assert_eq!(read.addresses, member.addresses);
    assert_eq!(read.profile, member.profile);
}

#[test]
fn a_missing_array_reads_as_empty_and_a_missing_option_as_none() {
    let stored = doc! {
        "_id": ObjectId::new(),
        "name": "Ada",
        "email": "ada@example.com",
        "logins": 0_i64,
    };
    let member = Member::from_document(stored).expect("from_document");
    assert!(member.tags.is_empty());
    assert!(member.addresses.is_empty());
    assert_eq!(member.profile, None);
    assert_eq!(member.deleted_at, None);
}

#[test]
fn a_missing_required_field_is_an_error_naming_it() {
    let stored = doc! { "_id": ObjectId::new(), "name": "Ada", "logins": 0_i64 };
    let error = Member::from_document(stored).expect_err("no email");
    assert!(names(&error, "email"), "{error}");
}

#[test]
fn a_hidden_field_never_appears_in_the_serialized_document() {
    let member = Member::make(member_attributes()).expect("make");
    let json = serde_json::to_value(&member).expect("serialize");
    let object = json.as_object().expect("an object");
    assert!(!object.contains_key("password"), "{json}");
    assert!(!object.contains_key("_id"), "{json}");
    assert_eq!(object["id"], serde_json::json!(member.id.to_hex()));
    assert_eq!(object["name"], serde_json::json!("Ada"));
    assert_eq!(object["balance"], serde_json::json!("12.50"));
    assert_eq!(
        object["joined_at"],
        serde_json::json!("2026-10-10T08:30:00Z")
    );
    assert_eq!(object["addresses"][0]["city"], serde_json::json!("London"));
}

#[test]
fn visible_lists_the_only_fields_serialized() {
    let card = Card::make(doc! { "title": "Ace", "secret": "hidden" }).expect("make");
    let json = serde_json::to_value(&card).expect("serialize");
    assert_eq!(json, serde_json::json!({ "title": "Ace" }));
}

#[test]
fn fill_applies_the_guard_and_keeps_the_key() {
    let mut member = Member::make(member_attributes()).expect("make");
    let key = member.id;
    member
        .fill(doc! { "name": "Grace", "is_admin": true })
        .expect("fill");
    assert_eq!(member.name, "Grace");
    assert_eq!(member.is_admin, None);
    assert_eq!(member.id, key);
    let error = member
        .fill(doc! { "logins": "x" })
        .expect_err("a wrong type");
    assert!(names(&error, "logins"), "{error}");
}

#[test]
fn the_route_key_is_the_key_and_a_malformed_one_binds_nothing() {
    let member = Member::make(member_attributes()).expect("make");
    assert_eq!(Member::route_key_name(), "id");
    assert_eq!(member.route_key(), member.id.to_hex());
    assert_eq!(
        <ObjectId as DocumentKey>::parse_route_key(&member.id.to_hex()),
        Some(member.id)
    );
    assert_eq!(<ObjectId as DocumentKey>::parse_route_key("not-hex"), None);
    assert_eq!(
        <String as DocumentKey>::parse_route_key("A-1"),
        Some("A-1".to_owned())
    );
    assert_eq!(<i64 as DocumentKey>::parse_route_key("12"), Some(12));
    assert_eq!(<i64 as DocumentKey>::parse_route_key("x"), None);
}

#[tokio::test]
async fn a_route_value_that_is_no_object_id_binds_nothing_without_a_query() {
    // No connection is registered: a lookup would fail, so `Ok(None)` shows
    // the malformed key never reached the server.
    let bound = Member::resolve_route_binding("not-an-object-id", None)
        .await
        .expect("a malformed key is no error");
    assert!(bound.is_none());
}

#[tokio::test]
async fn restore_on_a_model_without_soft_deletes_is_an_error_naming_restore() {
    let product = Product::make(doc! { "sku": "A-1", "title": "Anvil" }).expect("make");
    let error = product.restore().await.expect_err("no soft deletes");
    assert!(names(&error, "restore"), "{error}");
}

#[tokio::test]
async fn increment_by_a_value_that_is_no_number_is_an_error_naming_increment() {
    let mut member = Member::make(member_attributes()).expect("make");
    let error = member
        .increment("logins", "three")
        .await
        .expect_err("not a number");
    assert!(names(&error, "increment"), "{error}");
    let error = member
        .push("no_such_field", "a")
        .await
        .expect_err("no such field");
    assert!(names(&error, "no_such_field"), "{error}");
}

#[test]
fn queries_on_a_soft_deleting_model_leave_trashed_documents_out() {
    let filter = Member::query().to_filter().expect("render").filter;
    assert_eq!(filter, doc! { "deleted_at": null });
    let filter = Member::with_trashed().to_filter().expect("render").filter;
    assert_eq!(filter, doc! {});
    let filter = Member::only_trashed().to_filter().expect("render").filter;
    assert_eq!(filter, doc! { "deleted_at": { "$ne": null } });
    let filter = Product::query().to_filter().expect("render").filter;
    assert_eq!(filter, doc! {}, "no soft deletes, no scope");
}

// --- Events before any write -----------------------------------------------

struct Veto;

#[suprnova::async_trait]
impl CancellableListener<Creating<Vetoed>> for Veto {
    async fn handle(&self, event: &Creating<Vetoed>) -> EventResult {
        let name = event
            .attrs
            .lock()
            .await
            .get_str("name")
            .unwrap_or_default()
            .to_owned();
        EventResult::cancel(format!("no vetoed documents, not even {name}"))
    }
}

#[tokio::test]
async fn a_creating_listener_that_cancels_stops_create_before_any_write() {
    suprnova::listen_cancellable::<Creating<Vetoed>, Veto>(Arc::new(Veto)).await;
    // No connection is registered, so reaching the write would fail naming
    // MONGODB_URI. The listener's reason shows create stopped before it.
    let error = Vetoed::create(doc! { "name": "Eve" })
        .await
        .expect_err("cancelled");
    assert!(names(&error, "not even Eve"), "{error}");
    assert!(!names(&error, "MONGODB_URI"), "{error}");
}

pub struct WatchedObserver;

#[suprnova::observer(Watched)]
#[suprnova::async_trait]
impl DocumentObserver<Watched> for WatchedObserver {
    async fn saving(&self, attrs: &mut Document, is_creating: bool) -> EventResult {
        if is_creating && attrs.get_str("name").is_ok_and(|name| name == "blocked") {
            return EventResult::cancel("the observer blocks this name");
        }
        EventResult::ok()
    }
}

#[tokio::test]
async fn an_observer_registered_through_the_observer_attribute_sees_the_events() {
    suprnova::bootstrap_observers()
        .await
        .expect("install the observers");
    let error = Watched::create(doc! { "name": "blocked" })
        .await
        .expect_err("the observer cancels");
    assert!(names(&error, "the observer blocks this name"), "{error}");
}

// --- Against a server ------------------------------------------------------
//
// Tests run at once against one server, so each works on documents of its
// own: a member test on its own email, a product test on its own SKUs, and
// the event tests on models no other test writes, since a listener sees
// every write of its model in the process.

/// A product the event test alone writes.
#[suprnova::document(collection = "m2_gadgets", primary_key = "sku", fillable = ["sku", "title"])]
pub struct Gadget {
    pub sku: String,
    pub title: String,
}

/// A soft-deleting model the restore test alone writes.
#[suprnova::document(collection = "m2_badges", fillable = ["label"], soft_deletes)]
pub struct Badge {
    pub label: String,
    pub created_at: Option<suprnova::bson::DateTime>,
    pub updated_at: Option<suprnova::bson::DateTime>,
    pub deleted_at: Option<suprnova::bson::DateTime>,
}

async fn connect() {
    let url = test_url();
    let config = MongoConfig::builder()
        .uri(url.clone())
        .database(database_of(&url))
        .build()
        .expect("the test configuration");
    Mongo::init_with(config).await.expect("connect");
}

/// Remove the documents of `collection` that match `filter`, through the
/// driver, so a rerun starts clean.
async fn remove(collection: &str, filter: Document) {
    Mongo::collection::<Document>(collection)
        .expect("collection")
        .delete_many(filter)
        .await
        .expect("remove the test's documents");
}

async fn raw(collection: &str, id: impl Into<Bson>) -> Option<Document> {
    Mongo::collection::<Document>(collection)
        .expect("collection")
        .find_one(doc! { "_id": id.into() })
        .await
        .expect("read the raw document")
}

fn member_with_email(email: &str) -> Document {
    let mut attributes = member_attributes();
    attributes.insert("email", email);
    attributes
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_create_then_find_answers_the_created_fields_and_timestamps() {
    connect().await;
    let email = "create@example.com";
    remove(Member::COLLECTION, doc! { "email": email }).await;

    let mut attributes = member_with_email(email);
    attributes.insert("is_admin", true);
    let created = Member::create(attributes).await.expect("create");
    assert!(created.created_at.is_some(), "created_at is set");
    assert!(created.updated_at.is_some(), "updated_at is set");

    let found = Member::find(created.id)
        .await
        .expect("find")
        .expect("the created document");
    assert_eq!(found.name, "Ada");
    assert_eq!(found.email, email);
    assert_eq!(found.tags, vec!["math".to_owned()]);
    assert_eq!(found.addresses, created.addresses);
    assert_eq!(found.balance, Some(Decimal::new(1250, 2)));
    assert_eq!(found.created_at, created.created_at, "created_at is stored");

    let stored = raw(Member::COLLECTION, created.id).await.expect("stored");
    assert!(
        matches!(stored.get("created_at"), Some(Bson::DateTime(_))),
        "{stored}"
    );
    assert!(
        matches!(stored.get("is_admin"), None | Some(Bson::Null)),
        "a field outside fillable is not written: {stored}"
    );

    assert!(
        Member::find(ObjectId::new())
            .await
            .expect("find an absent id")
            .is_none(),
        "an absent id answers None"
    );
    let error = Member::find_or_fail(ObjectId::new())
        .await
        .expect_err("absent");
    assert_eq!(error.status_code(), 404, "{error}");
    assert!(
        Member::all()
            .await
            .expect("all")
            .iter()
            .any(|member| member.id == created.id),
        "all lists it"
    );
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_update_save_fresh_and_refresh_write_and_read_the_document() {
    connect().await;
    remove(
        Product::COLLECTION,
        doc! { "_id": { "$in": ["A-1", "B-2"] } },
    )
    .await;

    let product = Product::create(doc! { "sku": "A-1", "title": "Anvil" })
        .await
        .expect("create");
    let product = product
        .update(doc! { "title": "Big anvil" })
        .await
        .expect("update");
    assert_eq!(product.title, "Big anvil");

    let mut stale = Product::find("A-1").await.expect("find").expect("found");
    let mut current = product.clone();
    current.title = "Heavy anvil".into();
    current.save().await.expect("save an existing document");
    assert_eq!(
        stale.fresh().await.expect("fresh").expect("found").title,
        "Heavy anvil"
    );
    stale.refresh().await.expect("refresh");
    assert_eq!(stale.title, "Heavy anvil");

    // `save` on a document that was never stored inserts it.
    let mut new = Product {
        sku: "B-2".into(),
        title: "Bellows".into(),
    };
    new.save().await.expect("save a new document");
    assert!(Product::find("B-2").await.expect("find").is_some());
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_delete_on_a_soft_deleting_model_keeps_the_document_until_force_delete() {
    connect().await;
    let email = "soft@example.com";
    remove(Member::COLLECTION, doc! { "email": email }).await;

    let member = Member::create(member_with_email(email))
        .await
        .expect("create");
    let id = member.id;
    member.delete().await.expect("soft delete");

    let stored = raw(Member::COLLECTION, id).await.expect("still stored");
    assert!(
        matches!(stored.get("deleted_at"), Some(Bson::DateTime(_))),
        "{stored}"
    );
    assert!(
        Member::find(id).await.expect("find").is_none(),
        "scoped out"
    );
    let trashed = Member::only_trashed()
        .where_("email", "=", email)
        .get()
        .await
        .expect("only_trashed")
        .into_vec();
    assert_eq!(trashed.len(), 1, "only_trashed lists it");
    assert_eq!(
        Member::with_trashed()
            .where_("email", "=", email)
            .count()
            .await
            .expect("count"),
        1
    );

    let restored = trashed
        .into_iter()
        .next()
        .expect("one")
        .restore()
        .await
        .expect("restore");
    assert_eq!(restored.deleted_at, None);
    assert!(Member::find(id).await.expect("find").is_some());

    restored.force_delete().await.expect("force delete");
    assert!(raw(Member::COLLECTION, id).await.is_none(), "removed");
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_embedded_documents_round_trip_and_their_relations_write() {
    connect().await;
    let email = "embedded@example.com";
    remove(Member::COLLECTION, doc! { "email": email }).await;

    let mut member = Member::create(member_with_email(email))
        .await
        .expect("create");
    let created_at = member.created_at.expect("created_at is set");
    let mut updated_at = member.updated_at.expect("updated_at is set");

    // A BSON datetime holds milliseconds, so each write waits past one
    // before it runs: an `updated_at` it set is then strictly later.
    let advanced = |member: &Member, before: &mut suprnova::bson::DateTime, call: &str| {
        let after = member.updated_at.expect("updated_at stays set");
        assert!(
            after > *before,
            "{call} advances updated_at: {before:?} then {after:?}"
        );
        assert_eq!(
            member.created_at,
            Some(created_at),
            "{call} keeps created_at"
        );
        *before = after;
    };
    let tick = || tokio::time::sleep(std::time::Duration::from_millis(5));

    let second = Address {
        street: "2 Engine Row".into(),
        city: "Leeds".into(),
    };
    tick().await;
    member
        .addresses()
        .save(&second)
        .await
        .expect("save an embedded address");
    assert_eq!(member.addresses.len(), 2);
    advanced(&member, &mut updated_at, "EmbedsMany::save");
    let found = Member::find(member.id).await.expect("find").expect("found");
    assert_eq!(found.addresses, member.addresses);
    assert_eq!(
        found.updated_at,
        Some(updated_at),
        "the server holds the new updated_at"
    );

    tick().await;
    member
        .addresses()
        .destroy(&second)
        .await
        .expect("remove it");
    assert_eq!(member.addresses.len(), 1);
    advanced(&member, &mut updated_at, "EmbedsMany::destroy");

    tick().await;
    member
        .profile()
        .save(&Profile {
            bio: "Analyst".into(),
        })
        .await
        .expect("replace the profile");
    advanced(&member, &mut updated_at, "EmbedsOne::save");
    assert_eq!(
        Member::find(member.id)
            .await
            .expect("find")
            .expect("found")
            .profile,
        Some(Profile {
            bio: "Analyst".into()
        })
    );
    tick().await;
    member.profile().delete().await.expect("remove the profile");
    assert_eq!(member.profile, None);
    advanced(&member, &mut updated_at, "EmbedsOne::delete");
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_push_pull_increment_and_decrement_change_the_stored_document() {
    connect().await;
    let email = "arrays@example.com";
    remove(Member::COLLECTION, doc! { "email": email }).await;

    let mut member = Member::create(member_with_email(email))
        .await
        .expect("create");
    let created_at = member.created_at.expect("created_at is set");
    let mut updated_at = member.updated_at.expect("updated_at is set");

    // A BSON datetime holds milliseconds, so each write waits past one
    // before it runs: an `updated_at` it set is then strictly later.
    let advanced = |member: &Member, before: &mut suprnova::bson::DateTime, call: &str| {
        let after = member.updated_at.expect("updated_at stays set");
        assert!(
            after > *before,
            "{call} advances updated_at: {before:?} then {after:?}"
        );
        assert_eq!(
            member.created_at,
            Some(created_at),
            "{call} keeps created_at"
        );
        *before = after;
    };
    let tick = || tokio::time::sleep(std::time::Duration::from_millis(5));

    tick().await;
    member.push("tags", "a").await.expect("push");
    assert_eq!(member.tags, vec!["math".to_owned(), "a".to_owned()]);
    advanced(&member, &mut updated_at, "push");
    tick().await;
    member.push_unique("tags", "a").await.expect("push_unique");
    assert_eq!(member.tags.len(), 2, "already there");
    advanced(&member, &mut updated_at, "push_unique");
    tick().await;
    member.pull("tags", "math").await.expect("pull");
    assert_eq!(member.tags, vec!["a".to_owned()]);
    advanced(&member, &mut updated_at, "pull");

    tick().await;
    member.increment("logins", 2).await.expect("increment");
    assert_eq!(member.logins, 5);
    advanced(&member, &mut updated_at, "increment");
    tick().await;
    member.decrement("logins", 1).await.expect("decrement");
    assert_eq!(member.logins, 4);
    advanced(&member, &mut updated_at, "decrement");

    let found = Member::find(member.id).await.expect("find").expect("found");
    assert_eq!(found.tags, vec!["a".to_owned()]);
    assert_eq!(found.logins, 4);
    assert_eq!(
        found.updated_at,
        Some(updated_at),
        "the server holds the last updated_at"
    );
}

/// The events the gadget listeners saw, in order.
static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn saw(event: &'static str) {
    SEEN.lock().unwrap_or_else(|p| p.into_inner()).push(event);
}

/// A listener for every non-cancellable event the requirement names.
struct Recorder;

macro_rules! record {
    ($($event:ident => $name:literal),* $(,)?) => {
        $(
            #[suprnova::async_trait]
            impl Listener<$event<Gadget>> for Recorder {
                async fn handle(&self, _event: &$event<Gadget>) -> Result<(), FrameworkError> {
                    saw($name);
                    Ok(())
                }
            }
        )*
    };
}

record!(Created => "created", Updated => "updated", Saved => "saved", Deleted => "deleted");

/// A listener for every cancellable event the requirement names.
struct CancellableRecorder;

macro_rules! record_cancellable {
    ($($event:ident => $name:literal),* $(,)?) => {
        $(
            #[suprnova::async_trait]
            impl CancellableListener<$event<Gadget>> for CancellableRecorder {
                async fn handle(&self, _event: &$event<Gadget>) -> EventResult {
                    saw($name);
                    EventResult::ok()
                }
            }
        )*
    };
}

record_cancellable!(
    Saving => "saving",
    Creating => "creating",
    Updating => "updating",
    Deleting => "deleting",
);

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_create_update_and_delete_dispatch_the_model_events_in_order() {
    connect().await;
    remove(Gadget::COLLECTION, doc! { "_id": "E-1" }).await;
    EventFacade::listen::<Created<Gadget>, _>(Arc::new(Recorder)).await;
    EventFacade::listen::<Updated<Gadget>, _>(Arc::new(Recorder)).await;
    EventFacade::listen::<Saved<Gadget>, _>(Arc::new(Recorder)).await;
    EventFacade::listen::<Deleted<Gadget>, _>(Arc::new(Recorder)).await;
    suprnova::listen_cancellable::<Saving<Gadget>, _>(Arc::new(CancellableRecorder)).await;
    suprnova::listen_cancellable::<Creating<Gadget>, _>(Arc::new(CancellableRecorder)).await;
    suprnova::listen_cancellable::<Updating<Gadget>, _>(Arc::new(CancellableRecorder)).await;
    suprnova::listen_cancellable::<Deleting<Gadget>, _>(Arc::new(CancellableRecorder)).await;

    let gadget = Gadget::create(doc! { "sku": "E-1", "title": "Event" })
        .await
        .expect("create");
    let gadget = gadget
        .update(doc! { "title": "Evented" })
        .await
        .expect("update");
    gadget.delete().await.expect("delete");

    let seen = SEEN.lock().unwrap_or_else(|p| p.into_inner()).clone();
    assert_eq!(
        seen,
        vec![
            "saving", "creating", "created", "saved", "saving", "updating", "updated", "saved",
            "deleting", "deleted",
        ]
    );
}

/// Records each `restored` of a badge.
struct RestoreRecorder(Arc<Mutex<Vec<&'static str>>>);

#[suprnova::async_trait]
impl Listener<Restored<Badge>> for RestoreRecorder {
    async fn handle(&self, _event: &Restored<Badge>) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push("restored");
        Ok(())
    }
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_restore_dispatches_restored_and_route_binding_finds_by_key() {
    connect().await;
    remove(Badge::COLLECTION, doc! {}).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    EventFacade::listen::<Restored<Badge>, _>(Arc::new(RestoreRecorder(seen.clone()))).await;

    let badge = Badge::create(doc! { "label": "Gold" })
        .await
        .expect("create");
    let id = badge.id;
    let bound = Badge::resolve_route_binding(&id.to_hex(), None)
        .await
        .expect("bind")
        .expect("found by key");
    assert_eq!(bound.id, id);

    badge.delete().await.expect("soft delete");
    assert!(
        Badge::resolve_route_binding(&id.to_hex(), None)
            .await
            .expect("bind")
            .is_none(),
        "a trashed document is not bound"
    );
    let trashed = Badge::resolve_soft_deletable_route_binding(&id.to_hex(), None)
        .await
        .expect("bind")
        .expect("with_trashed binds it");
    trashed.restore().await.expect("restore");
    assert_eq!(
        *seen.lock().unwrap_or_else(|p| p.into_inner()),
        vec!["restored"]
    );
}
