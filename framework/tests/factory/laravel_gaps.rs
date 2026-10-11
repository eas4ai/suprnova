//! Counted factory terminals and explicit per-record overrides.

use sea_orm::EntityTrait;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Factory, FactoryBuilder, Model, attrs, model};

use super::persist::{UserFactory, fresh_db, toy_user};

/// A required column the output hides, so attribute sets must merge over
/// the stored attributes rather than the filtered serde output.
#[model(table = "gap_factory_secrets", timestamps = false, hidden = ["secret"])]
pub struct FactorySecret {
    pub id: i64,
    pub name: String,
    pub secret: String,
}

struct FactorySecretFactory;

impl Factory for FactorySecretFactory {
    type Model = FactorySecret;

    fn definition() -> FactorySecret {
        FactorySecret {
            id: 0,
            name: "definition".into(),
            secret: "definition secret".into(),
            ..Default::default()
        }
    }
}

async fn secret_db() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE gap_factory_secrets (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            secret TEXT NOT NULL\
         )",
    )
    .await
    .unwrap();
    db
}

async fn stored_secrets(db: &TestDatabase) -> Vec<(String, String)> {
    db.fetch_all(
        "SELECT name, secret FROM gap_factory_secrets ORDER BY id",
        vec![],
    )
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        (
            row.try_get::<String>("", "name").unwrap(),
            row.try_get::<String>("", "secret").unwrap(),
        )
    })
    .collect()
}

#[test]
fn make_has_a_compile_time_single_or_vector_result() {
    let one: toy_user::Model = UserFactory::new().make();
    assert!(!one.email.is_empty());
    let builder: FactoryBuilder<toy_user::Model, true> = UserFactory::new().count(3);
    let many: Vec<toy_user::Model> = builder.with(|user| user.name = "counted".into()).make();
    assert_eq!(many.len(), 3);
    assert!(many.iter().all(|user| user.name == "counted"));
    let many: Vec<toy_user::Model> = UserFactory::times(3).make();
    assert_eq!(many.len(), 3);
    assert_eq!(UserFactory::new().times(3).make().len(), 3);
    assert!(UserFactory::times(0).make().is_empty());
    assert_eq!(UserFactory::times(1).make().len(), 1);
    assert_eq!(UserFactory::times(9).count(2).make().len(), 2);
    let one: toy_user::Model = UserFactory::times(0).make_one();
    assert!(!one.email.is_empty());
}

#[tokio::test]
#[serial]
async fn create_has_a_compile_time_single_or_vector_result() {
    let (_guard, conn) = fresh_db().await;
    let one: toy_user::Model = UserFactory::new().create().await.unwrap();
    let many: Vec<toy_user::Model> = UserFactory::new().count(3).create().await.unwrap();
    assert_eq!(many.len(), 3);
    assert!(many.iter().all(|user| user.id > one.id));
    assert_eq!(toy_user::Entity::find().all(&conn).await.unwrap().len(), 4);
    assert!(UserFactory::times(0).create().await.unwrap().is_empty());
    assert_eq!(UserFactory::times(1).create().await.unwrap().len(), 1);
    assert_eq!(toy_user::Entity::find().all(&conn).await.unwrap().len(), 5);
}

#[tokio::test]
#[serial]
async fn create_many_accepts_a_count_and_overlays_each_attribute_set() {
    let (_guard, conn) = fresh_db().await;
    let counted = UserFactory::new().create_many(2).await.unwrap();
    assert_eq!(counted.len(), 2);
    let varied = UserFactory::new()
        .with(|user| user.name = "builder default".into())
        .with(|user| user.email = "definition@example.test".into())
        .create_many([
            attrs! { name: "Ada" },
            attrs! { name: "Grace", email: "grace@example.test" },
        ])
        .await
        .unwrap();
    assert_eq!(varied[0].name, "Ada");
    assert_eq!(varied[0].email, "definition@example.test");
    assert_eq!(varied[1].name, "Grace");
    assert_eq!(varied[1].email, "grace@example.test");
    let stored = toy_user::Entity::find().all(&conn).await.unwrap();
    assert_eq!(stored.len(), 4);
    assert_eq!(stored[2..], varied);
    assert!(UserFactory::new().create_many(0).await.unwrap().is_empty());
    assert!(
        UserFactory::new()
            .create_many(Vec::<suprnova::Attrs>::new())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[serial]
async fn invalid_attributes_fail_before_any_record_is_inserted() {
    let (_guard, conn) = fresh_db().await;
    let error = UserFactory::new()
        .create_many([attrs! { name: "valid" }, attrs! { name: 42 }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("factory attributes"));
    let error = UserFactory::new()
        .create_many([attrs! { typo: "x" }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("typo"));
    assert!(
        toy_user::Entity::find()
            .all(&conn)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[serial]
async fn counted_create_stops_on_storage_failure_and_joins_a_transaction() {
    let (_guard, conn) = fresh_db().await;
    use sea_orm::ConnectionTrait;
    conn.execute_unprepared("CREATE UNIQUE INDEX unique_email ON toy_users(email)")
        .await
        .unwrap();
    let result = suprnova::DB::transaction(|_| {
        Box::pin(async {
            UserFactory::times(3)
                .with(|user| user.email = "duplicate@example.test".into())
                .create()
                .await
        })
    })
    .await;
    assert!(result.is_err());
    assert!(
        toy_user::Entity::find()
            .all(&conn)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[serial]
async fn create_many_overrides_a_hidden_definition_field() {
    let db = secret_db().await;
    let saved = FactorySecretFactory::new()
        .create_many([attrs! { secret: "override" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].id > 0);
    assert_eq!(saved[0].name, "definition");
    assert_eq!(saved[0].secret, "override");
    assert_eq!(
        stored_secrets(&db).await,
        vec![("definition".to_string(), "override".to_string())]
    );
}

#[tokio::test]
#[serial]
async fn create_many_keeps_a_hidden_definition_field_it_does_not_override() {
    let db = secret_db().await;
    let saved = FactorySecretFactory::new()
        .create_many([attrs! { name: "x" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "x");
    assert_eq!(saved[0].secret, "definition secret");
    let saved = FactorySecretFactory::new()
        .with(|record| record.secret = "builder secret".into())
        .create_many([attrs! { name: "y" }, attrs! { name: "z" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].secret, "builder secret");
    assert_eq!(saved[1].name, "z");
    assert_eq!(
        stored_secrets(&db).await,
        vec![
            ("x".to_string(), "definition secret".to_string()),
            ("y".to_string(), "builder secret".to_string()),
            ("z".to_string(), "builder secret".to_string()),
        ]
    );
    let error = FactorySecretFactory::new()
        .create_many([attrs! { typo: "x" }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("typo"));
    assert_eq!(stored_secrets(&db).await.len(), 3);
}

#[tokio::test]
#[serial]
async fn create_many_leaves_the_hidden_field_out_of_the_output() {
    let _db = secret_db().await;
    let saved = FactorySecretFactory::new()
        .create_many([attrs! { secret: "override" }])
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&saved[0].to_json()).unwrap();
    assert_eq!(json["name"], "definition");
    assert!(json.get("secret").is_none());
    let value = serde_json::to_value(&saved[0]).unwrap();
    assert!(value.get("secret").is_none());
    assert!(saved[0].to_array().get("secret").is_none());
}

/// Reads a visit count written as decimal text, so an attribute must reach
/// the field through the field's own deserializer.
mod text_count {
    pub fn serialize<S: serde::Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        let text: String = serde::Deserialize::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Required columns the model skips on output, so an attribute set must
/// reach each field directly rather than through the model's serde output.
#[model(table = "gap_factory_skipped", timestamps = false)]
#[serde(crate = "suprnova::serde")]
pub struct FactorySkipped {
    pub id: i64,
    #[serde(rename = "displayName")]
    pub name: String,
    #[serde(skip_serializing)]
    pub token: String,
    #[serde(skip)]
    pub note: String,
    #[serde(with = "text_count")]
    pub visits: i64,
}

struct FactorySkippedFactory;

impl Factory for FactorySkippedFactory {
    type Model = FactorySkipped;

    fn definition() -> FactorySkipped {
        FactorySkipped {
            id: 0,
            name: "definition".into(),
            token: "definition token".into(),
            note: "definition note".into(),
            visits: 42,
            ..Default::default()
        }
    }
}

async fn skipped_db() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE gap_factory_skipped (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            token TEXT NOT NULL, \
            note TEXT NOT NULL, \
            visits INTEGER NOT NULL\
         )",
    )
    .await
    .unwrap();
    db
}

async fn stored_skipped(db: &TestDatabase) -> Vec<(String, String, String, i64)> {
    db.fetch_all(
        "SELECT name, token, note, visits FROM gap_factory_skipped ORDER BY id",
        vec![],
    )
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        (
            row.try_get::<String>("", "name").unwrap(),
            row.try_get::<String>("", "token").unwrap(),
            row.try_get::<String>("", "note").unwrap(),
            row.try_get::<i64>("", "visits").unwrap(),
        )
    })
    .collect()
}

fn skipped_row(name: &str, token: &str, note: &str, visits: i64) -> (String, String, String, i64) {
    (name.into(), token.into(), note.into(), visits)
}

#[tokio::test]
#[serial]
async fn create_many_overrides_a_skip_serializing_field() {
    let db = skipped_db().await;
    let saved = FactorySkippedFactory::new()
        .create_many([attrs! { token: "override token" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].id > 0);
    assert_eq!(saved[0].token, "override token");
    assert_eq!(saved[0].note, "definition note");
    assert_eq!(
        stored_skipped(&db).await,
        vec![skipped_row(
            "definition",
            "override token",
            "definition note",
            42
        )]
    );
}

#[tokio::test]
#[serial]
async fn create_many_overrides_a_skip_field() {
    let db = skipped_db().await;
    let saved = FactorySkippedFactory::new()
        .create_many([attrs! { note: "override note" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].note, "override note");
    assert_eq!(saved[0].token, "definition token");
    assert_eq!(
        stored_skipped(&db).await,
        vec![skipped_row(
            "definition",
            "definition token",
            "override note",
            42
        )]
    );
}

#[tokio::test]
#[serial]
async fn create_many_keeps_skipped_fields_it_does_not_override() {
    let db = skipped_db().await;
    let saved = FactorySkippedFactory::new()
        .create_many([attrs! { displayName: "x" }])
        .await
        .unwrap();
    assert_eq!(saved[0].name, "x");
    assert_eq!(saved[0].token, "definition token");
    assert_eq!(saved[0].note, "definition note");
    let saved = FactorySkippedFactory::new()
        .with(|record| {
            record.token = "builder token".into();
            record.note = "builder note".into();
        })
        .create_many([attrs! { displayName: "y" }, attrs! { displayName: "z" }])
        .await
        .unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[1].token, "builder token");
    assert_eq!(saved[1].note, "builder note");
    assert_eq!(
        stored_skipped(&db).await,
        vec![
            skipped_row("x", "definition token", "definition note", 42),
            skipped_row("y", "builder token", "builder note", 42),
            skipped_row("z", "builder token", "builder note", 42),
        ]
    );
}

#[tokio::test]
#[serial]
async fn create_many_reads_an_attribute_through_the_field_deserializer() {
    let db = skipped_db().await;
    let saved = FactorySkippedFactory::new()
        .create_many([attrs! { visits: "7" }])
        .await
        .unwrap();
    assert_eq!(saved[0].visits, 7);
    assert_eq!(
        stored_skipped(&db).await,
        vec![skipped_row(
            "definition",
            "definition token",
            "definition note",
            7
        )]
    );
}

#[tokio::test]
#[serial]
async fn create_many_refuses_an_attribute_that_names_no_field_or_does_not_fit() {
    let db = skipped_db().await;
    let error = FactorySkippedFactory::new()
        .create_many([attrs! { token: "valid" }, attrs! { typo: "x" }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("typo"), "{error}");
    // The serialized name is the attribute; the Rust field name is not.
    let error = FactorySkippedFactory::new()
        .create_many([attrs! { name: "x" }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("`name`"), "{error}");
    let error = FactorySkippedFactory::new()
        .create_many([attrs! { note: 42 }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("note"), "{error}");
    let error = FactorySkippedFactory::new()
        .create_many([attrs! { visits: 7 }])
        .await
        .unwrap_err();
    assert!(error.to_string().contains("visits"), "{error}");
    assert!(stored_skipped(&db).await.is_empty());
}

#[tokio::test]
#[serial]
async fn create_many_leaves_skipped_fields_out_of_the_output() {
    let _db = skipped_db().await;
    let saved = FactorySkippedFactory::new()
        .create_many([attrs! { token: "override token", note: "override note" }])
        .await
        .unwrap();
    assert_eq!(saved[0].token, "override token");
    assert_eq!(saved[0].note, "override note");
    let json: serde_json::Value = serde_json::from_str(&saved[0].to_json()).unwrap();
    assert_eq!(json["displayName"], "definition");
    assert_eq!(json["visits"], "42");
    assert!(json.get("token").is_none());
    assert!(json.get("note").is_none());
    let array = saved[0].to_array();
    assert!(array.get("token").is_none());
    assert!(array.get("note").is_none());
}
