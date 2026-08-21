//! Linked-account creation and provider-subject lookup.

use async_trait::async_trait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::{SeaOrmStorage, db_error, random_id};
use crate::schema::{AuthSchema, EntityBinding, LinkedAccountFields};
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

/// Storage API for creating and looking up linked provider accounts.
///
/// Method removal (unlink) is deliberately absent here: taking away a
/// sign-in method must go through the census-guarded
/// [`super::MethodStore`].
#[async_trait]
pub trait LinkedAccountStore: Send + Sync {
    /// Create a linked-account row and return its stored view.
    ///
    /// `(provider, provider_account_id)` uniqueness is enforced by the
    /// application's driver-level unique index (see
    /// [`crate::schema::LinkedAccountFields`]); a violation of that index
    /// surfaces as [`crate::Error::Conflict`], not a check-then-insert race
    /// in this method. Callers that lose the race should re-read via
    /// [`Self::find_by_provider_subject`] and continue with the winner.
    async fn create(&self, input: NewLinkedAccount) -> Result<LinkedAccountRecord>;
    /// Find one linked account by its provider and provider-account
    /// identifier (`(provider, subject)`).
    async fn find_by_provider_subject(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<LinkedAccountRecord>>;
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

#[async_trait]
impl<S> LinkedAccountStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::LinkedAccount: LinkedAccountFields,
    <S::LinkedAccount as EntityBinding>::Entity: EntityTrait<
            Model = <S::LinkedAccount as EntityBinding>::Model,
            ActiveModel = <S::LinkedAccount as EntityBinding>::ActiveModel,
        >,
    <S::LinkedAccount as EntityBinding>::Column: ColumnTrait,
{
    async fn create(&self, input: NewLinkedAccount) -> Result<LinkedAccountRecord> {
        if input.user_id.is_empty() {
            return Err(empty("user_id"));
        }
        if input.provider.is_empty() {
            return Err(empty("provider"));
        }
        if input.provider_account_id.is_empty() {
            return Err(empty("provider_account_id"));
        }
        let account_id = random_id();
        let mut model = <S::LinkedAccount as EntityBinding>::ActiveModel::default();
        S::LinkedAccount::write_account_id(&mut model, &account_id);
        S::LinkedAccount::write_user_id(&mut model, &input.user_id);
        S::LinkedAccount::write_provider(&mut model, &input.provider);
        S::LinkedAccount::write_provider_account_id(&mut model, &input.provider_account_id);
        <S::LinkedAccount as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await
            .map_err(unique_conflict_or_db_error)?;
        Ok(LinkedAccountRecord {
            account_id,
            user_id: input.user_id,
            provider: input.provider,
            provider_account_id: input.provider_account_id,
        })
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
