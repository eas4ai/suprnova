//! Counted factory terminals and explicit per-record overrides.

use sea_orm::EntityTrait;
use serial_test::serial;
use suprnova::{Factory, FactoryBuilder, attrs};

use super::persist::{UserFactory, fresh_db, toy_user};

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
