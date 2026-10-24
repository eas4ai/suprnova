//! Linked-account creation and provider-subject lookup.

use async_trait::async_trait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::credential_writes::fenced_credential_write;
use super::{AuthTransaction, CredentialActor, SeaOrmStorage, db_error, in_transaction, random_id};
use crate::schema::{
    AuthSchema, EntityBinding, LinkedAccountFields, SessionEpoch, SessionFields, UserFields,
};
use crate::{Error, Result};

/// Input for creating one linked-account row.
#[derive(Clone, Debug)]
pub struct NewLinkedAccount {
    /// Owning user identifier.
    pub user_id: String,
    /// Provider key (the `{provider}` route segment).
    pub provider: String,
    /// The provider's stable account identifier (`sub` for OIDC, `id` for
    /// GitHub-style providers).
    pub provider_account_id: String,
}

/// A generic view of one linked-account row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedAccountRecord {
    /// Application-owned linked-account identifier.
    pub account_id: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Provider key.
    pub provider: String,
    /// The provider's stable account identifier.
    pub provider_account_id: String,
}

/// Storage API for authenticated creation and lookup of linked provider
/// accounts.
///
/// Method removal (unlink) is deliberately absent here: taking away a
/// sign-in method must go through the census-guarded
/// [`super::MethodStore`]. Initialization and migration seeding are likewise
/// absent; those explicit boundaries use [`LinkedAccountInitializer`].
#[async_trait]
pub trait LinkedAccountStore: Send + Sync {
    /// Create a linked-account row while the authenticated actor remains
    /// current.
    ///
    /// `input.user_id` must equal the actor's user. The epoch/session fence
    /// and insert execute in one transaction. `(provider,
    /// provider_account_id)` uniqueness is enforced by the application's
    /// driver-level unique index (see
    /// [`crate::schema::LinkedAccountFields`]); a violation of that index
    /// surfaces as [`crate::Error::Conflict`]. Callers that lose the race
    /// should re-read via [`Self::find_by_provider_subject`] and continue with
    /// the owner-checked winner.
    async fn create(
        &self,
        actor: &CredentialActor,
        input: NewLinkedAccount,
    ) -> Result<LinkedAccountRecord>;
    /// Verify that an actor is still current without creating an account.
    ///
    /// This preserves the fence for idempotent same-owner outcomes that do
    /// not execute an insert.
    async fn validate_actor(&self, actor: &CredentialActor) -> Result<()>;
    /// Find one linked account by its provider and provider-account
    /// identifier (`(provider, subject)`).
    async fn find_by_provider_subject(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<LinkedAccountRecord>>;
}

/// Explicit initialization/import boundary for seeding linked accounts
/// without an authenticated actor.
///
/// Runtime identity resolution intentionally depends only on
/// [`LinkedAccountStore`] and cannot call this method.
#[async_trait]
pub trait LinkedAccountInitializer: LinkedAccountStore {
    /// Seed one linked-account row during an already-authorized
    /// initialization, migration, or first-proof boundary.
    async fn initialize(&self, input: NewLinkedAccount) -> Result<LinkedAccountRecord>;
}

fn record<S>(model: &<S::LinkedAccount as EntityBinding>::Model) -> LinkedAccountRecord
where
    S: AuthSchema,
    S::LinkedAccount: LinkedAccountFields,
{
    LinkedAccountRecord {
        account_id: S::LinkedAccount::read_account_id(model),
        user_id: S::LinkedAccount::read_user_id(model),
        provider: S::LinkedAccount::read_provider(model),
        provider_account_id: S::LinkedAccount::read_provider_account_id(model),
    }
}

fn empty(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: "must not be empty".to_owned(),
    }
}

/// Map a unique-constraint violation from the driver-level index on
/// `(provider, provider_account_id)` to [`Error::Conflict`]; every other
/// database error still becomes [`Error::Internal`] via [`db_error`].
fn unique_conflict_or_db_error(error: sea_orm::DbErr) -> Error {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        return Error::Conflict {
            resource: "linked-account".to_owned(),
            message: "provider identity is already linked to an account".to_owned(),
        };
    }
    db_error(error)
}

fn stale_actor() -> Error {
    Error::NotFound {
        resource: "credential actor".to_owned(),
        identifier: "expired or revoked".to_owned(),
    }
}

fn validate(input: &NewLinkedAccount) -> Result<()> {
    if input.user_id.is_empty() {
        return Err(empty("user_id"));
    }
    if input.provider.is_empty() {
        return Err(empty("provider"));
    }
    if input.provider_account_id.is_empty() {
        return Err(empty("provider_account_id"));
    }
    Ok(())
}

async fn insert_in<S>(
    transaction: &mut AuthTransaction<'_>,
    input: NewLinkedAccount,
) -> Result<LinkedAccountRecord>
where
    S: AuthSchema,
    S::LinkedAccount: LinkedAccountFields,
    <S::LinkedAccount as EntityBinding>::Entity: EntityTrait<
            Model = <S::LinkedAccount as EntityBinding>::Model,
            ActiveModel = <S::LinkedAccount as EntityBinding>::ActiveModel,
        >,
{
    let account_id = random_id();
    let mut model = <S::LinkedAccount as EntityBinding>::ActiveModel::default();
    S::LinkedAccount::write_account_id(&mut model, &account_id);
    S::LinkedAccount::write_user_id(&mut model, &input.user_id);
    S::LinkedAccount::write_provider(&mut model, &input.provider);
    S::LinkedAccount::write_provider_account_id(&mut model, &input.provider_account_id);
    <S::LinkedAccount as EntityBinding>::Entity::insert(model)
        .exec(transaction.connection())
        .await
        .map_err(unique_conflict_or_db_error)?;
    Ok(LinkedAccountRecord {
        account_id,
        user_id: input.user_id,
        provider: input.provider,
        provider_account_id: input.provider_account_id,
    })
}

#[async_trait]
impl<S> LinkedAccountStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::Session: SessionFields,
    S::LinkedAccount: LinkedAccountFields,
    <S::LinkedAccount as EntityBinding>::Entity: EntityTrait<
            Model = <S::LinkedAccount as EntityBinding>::Model,
            ActiveModel = <S::LinkedAccount as EntityBinding>::ActiveModel,
        >,
    <S::LinkedAccount as EntityBinding>::Column: ColumnTrait,
{
    async fn create(
        &self,
        actor: &CredentialActor,
        input: NewLinkedAccount,
    ) -> Result<LinkedAccountRecord> {
        validate(&input)?;
        if actor.user_id() != input.user_id {
            return Err(stale_actor());
        }
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(insert_in::<S>(transaction, input))
        })
        .await
    }

    async fn validate_actor(&self, actor: &CredentialActor) -> Result<()> {
        fenced_credential_write(self, actor, |_transaction| Box::pin(async { Ok(()) })).await
    }

    async fn find_by_provider_subject(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<LinkedAccountRecord>> {
        if provider.is_empty() {
            return Err(empty("provider"));
        }
        if provider_account_id.is_empty() {
            return Err(empty("provider_account_id"));
        }
        let rows = <S::LinkedAccount as EntityBinding>::Entity::find()
            .filter(S::LinkedAccount::provider_column().eq(provider.to_owned()))
            .filter(
                S::LinkedAccount::provider_account_id_column().eq(provider_account_id.to_owned()),
            )
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.first().map(record::<S>))
    }
}

#[async_trait]
impl<S> LinkedAccountInitializer for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::Session: SessionFields,
    S::LinkedAccount: LinkedAccountFields,
    <S::LinkedAccount as EntityBinding>::Entity: EntityTrait<
            Model = <S::LinkedAccount as EntityBinding>::Model,
            ActiveModel = <S::LinkedAccount as EntityBinding>::ActiveModel,
        >,
    <S::LinkedAccount as EntityBinding>::Column: ColumnTrait,
{
    async fn initialize(&self, input: NewLinkedAccount) -> Result<LinkedAccountRecord> {
        validate(&input)?;
        in_transaction(self.database(), move |transaction| {
            Box::pin(insert_in::<S>(transaction, input))
        })
        .await
    }
}
