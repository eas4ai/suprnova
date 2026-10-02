//! Contract tests for the eight-role schema boundary.

#[cfg(feature = "seaorm-sqlite")]
use sea_orm::EntityTrait;

#[path = "fixtures/renamed_schema.rs"]
mod renamed_schema;
use magnetar::schema::{
    AuthSchema, CeremonyFields, EntityBinding, LinkedAccountFields, LockoutFields,
    NOT_NULL_PASSWORD_EMPTY_SENTINEL, PasskeyFields, SessionEpoch, SessionFields, TokenFields,
    TokenRecordFields, UserFields, UserOptionalFields, password_hash_for_verifier,
};
use renamed_schema::{OriginalSchema, RenamedSchema, original_user, renamed_user};

fn count_users<S>(models: &[<S::User as EntityBinding>::Model]) -> usize
where
    S: AuthSchema,
    S::User: UserFields,
{
    models
        .iter()
        .filter(|model| !<S::User as UserFields>::read_email(model).is_empty())
        .count()
}

fn assert_all_role_capabilities<S>()
where
    S: AuthSchema,
    S::User: UserFields,
    S::User: SessionEpoch,
    S::User: UserOptionalFields,
    S::Session: SessionFields,
    S::LinkedAccount: LinkedAccountFields,
    S::Passkey: PasskeyFields,
    S::Token: TokenFields,
    S::Ceremony: CeremonyFields,
    S::Lockout: LockoutFields,
    S::TokenRecord: TokenRecordFields,
{
}

#[test]
fn auth_schema_exposes_all_eight_role_descriptors() {
    assert_all_role_capabilities::<OriginalSchema>();
    assert_all_role_capabilities::<RenamedSchema>();
}

#[test]
fn renamed_application_column_changes_only_the_binding() {
    let original = [original_user("user@example.test", Some("$argon2id$hash"))];
    let renamed = [renamed_user("user@example.test", Some("$argon2id$hash"))];

    assert_eq!(count_users::<OriginalSchema>(&original), 1);
    assert_eq!(count_users::<RenamedSchema>(&renamed), 1);
    assert_eq!(
        <<OriginalSchema as AuthSchema>::User as UserFields>::read_email(&original[0]),
        <<RenamedSchema as AuthSchema>::User as UserFields>::read_email(&renamed[0])
    );
}

#[test]
fn passwordless_reads_are_none_and_never_verifier_input() {
    let nullable = original_user("passwordless@example.test", None);
    let renamed = renamed_user(
        "passwordless@example.test",
        Some(NOT_NULL_PASSWORD_EMPTY_SENTINEL),
    );
    assert_eq!(
        <<OriginalSchema as AuthSchema>::User as UserFields>::read_password_hash(&nullable),
        None
    );
    assert_eq!(
        <<RenamedSchema as AuthSchema>::User as UserFields>::read_password_hash(&renamed),
        None
    );
    assert_eq!(
        password_hash_for_verifier::<<RenamedSchema as AuthSchema>::User>(&renamed),
        None
    );
    let hashed = original_user("hashed@example.test", Some("$argon2id$hash"));
    assert_eq!(
        password_hash_for_verifier::<<OriginalSchema as AuthSchema>::User>(&hashed),
        Some("$argon2id$hash".to_owned())
    );
}

#[cfg(feature = "seaorm-sqlite")]
async fn read_bound_emails<S>(db: &sea_orm::DatabaseConnection) -> Vec<String>
where
    S: AuthSchema,
    S::User: UserFields,
    <S::User as EntityBinding>::Entity: EntityTrait<Model = <S::User as EntityBinding>::Model>,
{
    <<S::User as EntityBinding>::Entity as EntityTrait>::find()
        .all(db)
        .await
        .expect("fixture query")
        .iter()
        .map(<S::User as UserFields>::read_email)
        .collect()
}

#[cfg(feature = "seaorm-sqlite")]
async fn read_bound_epoch<S>(db: &sea_orm::DatabaseConnection) -> u64
where
    S: AuthSchema,
    S::User: SessionEpoch,
    <S::User as EntityBinding>::Entity: EntityTrait<Model = <S::User as EntityBinding>::Model>,
{
    let rows = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
        .all(db)
        .await
        .expect("fixture epoch query");
    <S::User as SessionEpoch>::auth_epoch(&rows[0])
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn renamed_column_is_exercised_through_generic_seaorm_query() {
    let original_db = renamed_schema::original_fixture_db().await;
    let renamed_db = renamed_schema::renamed_fixture_db().await;

    assert_eq!(
        read_bound_emails::<OriginalSchema>(&original_db).await,
        vec!["original@example.test".to_owned()]
    );
    assert_eq!(
        read_bound_emails::<RenamedSchema>(&renamed_db).await,
        vec!["renamed@example.test".to_owned()]
    );
    assert_eq!(read_bound_epoch::<OriginalSchema>(&original_db).await, 7);
    assert_eq!(read_bound_epoch::<RenamedSchema>(&renamed_db).await, 7);
}
