//! Atomic authentication-method census and removal.

use async_trait::async_trait;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::credential_writes::fenced_credential_write;
use super::{AuthTransaction, CredentialActor, SeaOrmStorage, db_error};
use crate::schema::{
    AuthSchema, EntityBinding, LinkedAccountFields, PasskeyFields, SessionEpoch, SessionFields,
    UserFields,
};
use crate::{Error, Result};

/// Atomic method-removal storage API.
#[async_trait]
pub trait MethodStore: Send + Sync {
    /// Count the user's sign-in methods: password presence plus passkey and
    /// linked-account rows. Feeds the last-method protection.
    async fn census(&self, user_id: &str) -> Result<usize>;
    /// Remove the password while the authenticating actor remains live and
    /// only when another sign-in method remains.
    async fn remove_password_if_not_last(&self, actor: &CredentialActor) -> Result<bool>;
    /// Remove one passkey while the authenticating actor remains live and
    /// only when another sign-in method remains.
    async fn remove_passkey_if_not_last(
        &self,
        actor: &CredentialActor,
        passkey_id: &str,
    ) -> Result<bool>;
    /// Remove one linked account while the authenticating actor remains live
    /// and only when another sign-in method remains.
    async fn remove_linked_account_if_not_last(
        &self,
        actor: &CredentialActor,
        account_id: &str,
    ) -> Result<bool>;
}

async fn remove_password_if_not_last_in_transaction<S>(
    transaction: &mut AuthTransaction<'_>,
    user_id: &str,
) -> Result<bool>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::LinkedAccount: LinkedAccountFields,
    S::Passkey: PasskeyFields,
{
    let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotFound {
            resource: "user".to_owned(),
            identifier: user_id.to_owned(),
        })?;
    let password_present = S::User::read_password_hash(&user).is_some();
    if !password_present {
        return Ok(false);
    }
    let account_count = <S::LinkedAccount as EntityBinding>::Entity::find()
        .filter(S::LinkedAccount::user_id_column().eq(S::LinkedAccount::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .len();
    let passkey_count = <S::Passkey as EntityBinding>::Entity::find()
        .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .len();
    if 1 + account_count + passkey_count <= 1 {
        return Ok(false);
    }

    let expected_epoch = S::User::auth_epoch(&user);
    let mut user_update: <S::User as EntityBinding>::ActiveModel = Default::default();
    S::User::write_password_hash(&mut user_update, None);
    let user_cas = <<S::User as EntityBinding>::Entity as EntityTrait>::update_many()
        .set(user_update)
        .col_expr(
            S::User::auth_epoch_column(),
            Expr::col(S::User::auth_epoch_column()).add(1),
        )
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .filter(S::User::auth_epoch_column().eq(S::User::auth_epoch_value(expected_epoch)?))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    Ok(user_cas.rows_affected == 1)
}

async fn remove_passkey_if_not_last_in_transaction<S>(
    transaction: &mut AuthTransaction<'_>,
    user_id: &str,
    passkey_id: &str,
) -> Result<bool>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::LinkedAccount: LinkedAccountFields,
    S::Passkey: PasskeyFields,
{
    let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotFound {
            resource: "user".to_owned(),
            identifier: user_id.to_owned(),
        })?;
    let account_count = <S::LinkedAccount as EntityBinding>::Entity::find()
        .filter(S::LinkedAccount::user_id_column().eq(S::LinkedAccount::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .len();
    let passkey_rows = <S::Passkey as EntityBinding>::Entity::find()
        .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?;
    if !passkey_rows
        .iter()
        .any(|row| S::Passkey::read_passkey_id(row) == passkey_id)
    {
        return Ok(false);
    }
    let total = usize::from(S::User::read_password_hash(&user).is_some())
        + account_count
        + passkey_rows.len();
    if total <= 1 {
        return Ok(false);
    }

    let expected_epoch = S::User::auth_epoch(&user);
    let user_update: <S::User as EntityBinding>::ActiveModel = Default::default();
    let user_cas = <<S::User as EntityBinding>::Entity as EntityTrait>::update_many()
        .set(user_update)
        .col_expr(
            S::User::auth_epoch_column(),
            Expr::col(S::User::auth_epoch_column()).add(1),
        )
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .filter(S::User::auth_epoch_column().eq(S::User::auth_epoch_value(expected_epoch)?))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    if user_cas.rows_affected != 1 {
        return Ok(false);
    }
    let removed = <S::Passkey as EntityBinding>::Entity::delete_many()
        .filter(S::Passkey::passkey_id_column().eq(passkey_id.to_owned()))
        .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    if removed.rows_affected != 1 {
        return Err(Error::Conflict {
            resource: "authentication method".to_owned(),
            message: "method disappeared during conditional removal".to_owned(),
        });
    }
    Ok(true)
}

async fn remove_linked_account_if_not_last_in_transaction<S>(
    transaction: &mut AuthTransaction<'_>,
    user_id: &str,
    account_id: &str,
) -> Result<bool>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::LinkedAccount: LinkedAccountFields,
    S::Passkey: PasskeyFields,
{
    let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::NotFound {
            resource: "user".to_owned(),
            identifier: user_id.to_owned(),
        })?;
    let account_rows = <S::LinkedAccount as EntityBinding>::Entity::find()
        .filter(S::LinkedAccount::user_id_column().eq(S::LinkedAccount::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?;
    if !account_rows
        .iter()
        .any(|row| S::LinkedAccount::read_account_id(row) == account_id)
    {
        return Ok(false);
    }
    let passkey_count = <S::Passkey as EntityBinding>::Entity::find()
        .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
        .all(transaction.connection())
        .await
        .map_err(db_error)?
        .len();
    let total = usize::from(S::User::read_password_hash(&user).is_some())
        + account_rows.len()
        + passkey_count;
    if total <= 1 {
        return Ok(false);
    }

    let expected_epoch = S::User::auth_epoch(&user);
    let user_update: <S::User as EntityBinding>::ActiveModel = Default::default();
    let user_cas = <<S::User as EntityBinding>::Entity as EntityTrait>::update_many()
        .set(user_update)
        .col_expr(
            S::User::auth_epoch_column(),
            Expr::col(S::User::auth_epoch_column()).add(1),
        )
        .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
        .filter(S::User::auth_epoch_column().eq(S::User::auth_epoch_value(expected_epoch)?))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    if user_cas.rows_affected != 1 {
        return Ok(false);
    }
    let removed = <S::LinkedAccount as EntityBinding>::Entity::delete_many()
        .filter(S::LinkedAccount::account_id_column().eq(account_id.to_owned()))
        .filter(S::LinkedAccount::user_id_column().eq(S::LinkedAccount::user_id_value(user_id)))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    if removed.rows_affected != 1 {
        return Err(Error::Conflict {
            resource: "authentication method".to_owned(),
            message: "method disappeared during conditional removal".to_owned(),
        });
    }
    Ok(true)
}

#[async_trait]
impl<S> MethodStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::Session: SessionFields,
    S::LinkedAccount: LinkedAccountFields,
    S::Passkey: PasskeyFields,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
    <S::LinkedAccount as EntityBinding>::Entity: EntityTrait<
            Model = <S::LinkedAccount as EntityBinding>::Model,
            ActiveModel = <S::LinkedAccount as EntityBinding>::ActiveModel,
        >,
    <S::LinkedAccount as EntityBinding>::Column: ColumnTrait,
    <S::Passkey as EntityBinding>::Entity: EntityTrait<
            Model = <S::Passkey as EntityBinding>::Model,
            ActiveModel = <S::Passkey as EntityBinding>::ActiveModel,
        >,
    <S::Passkey as EntityBinding>::Column: ColumnTrait,
{
    async fn census(&self, user_id: &str) -> Result<usize> {
        if user_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "user_id".to_owned(),
                message: "user identifier must be non-empty".to_owned(),
            });
        }
        let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
            .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
            .all(self.database())
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        let password_present = S::User::read_password_hash(&user).is_some();
        let accounts = <S::LinkedAccount as EntityBinding>::Entity::find()
            .filter(S::LinkedAccount::user_id_column().eq(S::LinkedAccount::user_id_value(user_id)))
            .all(self.database())
            .await
            .map_err(db_error)?
            .len();
        let passkeys = <S::Passkey as EntityBinding>::Entity::find()
            .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
            .all(self.database())
            .await
            .map_err(db_error)?
            .len();
        Ok(usize::from(password_present) + accounts + passkeys)
    }

    async fn remove_password_if_not_last(&self, actor: &CredentialActor) -> Result<bool> {
        let user_id = actor.user_id().to_owned();
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(async move {
                remove_password_if_not_last_in_transaction::<S>(transaction, &user_id).await
            })
        })
        .await
    }

    async fn remove_passkey_if_not_last(
        &self,
        actor: &CredentialActor,
        passkey_id: &str,
    ) -> Result<bool> {
        if passkey_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "passkey_id".to_owned(),
                message: "passkey identifier must be non-empty".to_owned(),
            });
        }
        let user_id = actor.user_id().to_owned();
        let passkey_id = passkey_id.to_owned();
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(async move {
                remove_passkey_if_not_last_in_transaction::<S>(transaction, &user_id, &passkey_id)
                    .await
            })
        })
        .await
    }

    async fn remove_linked_account_if_not_last(
        &self,
        actor: &CredentialActor,
        account_id: &str,
    ) -> Result<bool> {
        if account_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "account_id".to_owned(),
                message: "linked account identifier must be non-empty".to_owned(),
            });
        }
        let user_id = actor.user_id().to_owned();
        let account_id = account_id.to_owned();
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(async move {
                remove_linked_account_if_not_last_in_transaction::<S>(
                    transaction,
                    &user_id,
                    &account_id,
                )
                .await
            })
        })
        .await
    }
}
