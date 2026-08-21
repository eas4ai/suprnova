//! Atomic authentication-method census and removal.

use async_trait::async_trait;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::{SeaOrmStorage, db_error, in_transaction};
use crate::schema::{
    AuthSchema, EntityBinding, LinkedAccountFields, PasskeyFields, SessionEpoch, UserFields,
};
use crate::{Error, Result};

/// Authentication method targeted by a removal request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// Remove the user's password credential.
    Password,
    /// Remove one passkey by application-owned row id.
    Passkey(String),
    /// Remove one linked account by application-owned row id.
    LinkedAccount(String),
}

/// Atomic method-removal storage API.
#[async_trait]
pub trait MethodStore: Send + Sync {
    /// Count the user's sign-in methods: password presence plus passkey and
    /// linked-account rows. Feeds the last-method protection.
    async fn census(&self, user_id: &str) -> Result<usize>;
    /// Remove a method only when the pre-removal census leaves another method.
    /// A concurrent operation that changes the census loses the user epoch CAS.
    async fn remove_method_if_not_last(
        &self,
        user_id: &str,
        method: AuthMethod,
        expected_census: usize,
    ) -> Result<bool>;
}

#[async_trait]
impl<S> MethodStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
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

    async fn remove_method_if_not_last(
        &self,
        user_id: &str,
        method: AuthMethod,
        expected_census: usize,
    ) -> Result<bool> {
        if user_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "user_id".to_owned(),
                message: "user identifier must be non-empty".to_owned(),
            });
        }
        let user_id = user_id.to_owned();
        in_transaction(self.database(), move |tx| {
            Box::pin(async move {
                let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
                    .filter(S::User::user_id_column().eq(S::User::user_id_value(&user_id)))
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::NotFound {
                        resource: "user".to_owned(),
                        identifier: user_id.to_owned(),
                    })?;
                let password_present = S::User::read_password_hash(&user).is_some();
                let account_rows = <S::LinkedAccount as EntityBinding>::Entity::find()
                    .filter(
                        S::LinkedAccount::user_id_column()
                            .eq(S::LinkedAccount::user_id_value(&user_id)),
                    )
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?;
                let passkey_rows = <S::Passkey as EntityBinding>::Entity::find()
                    .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(&user_id)))
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?;
                let account_count = account_rows.len();
                let passkey_count = passkey_rows.len();
                let target_exists = match &method {
                    AuthMethod::Password => password_present,
                    AuthMethod::Passkey(id) => passkey_rows
                        .iter()
                        .any(|row| S::Passkey::read_passkey_id(row) == *id),
                    AuthMethod::LinkedAccount(id) => account_rows
                        .iter()
                        .any(|row| S::LinkedAccount::read_account_id(row) == *id),
                };
                if !target_exists {
                    return Ok(false);
                }
                let total = usize::from(password_present) + account_count + passkey_count;
                if total != expected_census || total <= 1 {
                    return Ok(false);
                }
                let expected_epoch = S::User::auth_epoch(&user);
                let mut user_update: <S::User as EntityBinding>::ActiveModel = Default::default();
                if matches!(method, AuthMethod::Password) {
                    if !password_present {
                        return Ok(false);
                    }
                    S::User::write_password_hash(&mut user_update, None);
                }
                let user_cas = <<S::User as EntityBinding>::Entity as EntityTrait>::update_many()
                    .set(user_update)
                    .col_expr(
                        S::User::auth_epoch_column(),
                        Expr::col(S::User::auth_epoch_column()).add(1),
                    )
                    .filter(S::User::user_id_column().eq(S::User::user_id_value(&user_id)))
                    .filter(S::User::auth_epoch_column().eq(expected_epoch as i64))
                    .exec(tx.connection())
                    .await
                    .map_err(db_error)?;
                if user_cas.rows_affected != 1 {
                    return Ok(false);
                }
                let removed = match method {
                    AuthMethod::Password => 1,
                    AuthMethod::Passkey(id) => {
                        let result = <S::Passkey as EntityBinding>::Entity::delete_many()
                            .filter(S::Passkey::passkey_id_column().eq(id))
                            .filter(
                                S::Passkey::user_id_column()
                                    .eq(S::Passkey::user_id_value(&user_id)),
                            )
                            .exec(tx.connection())
                            .await
                            .map_err(db_error)?;
                        result.rows_affected
                    }
                    AuthMethod::LinkedAccount(id) => {
                        let result = <S::LinkedAccount as EntityBinding>::Entity::delete_many()
                            .filter(S::LinkedAccount::account_id_column().eq(id))
                            .filter(
                                S::LinkedAccount::user_id_column()
                                    .eq(S::LinkedAccount::user_id_value(&user_id)),
                            )
                            .exec(tx.connection())
                            .await
                            .map_err(db_error)?;
                        result.rows_affected
                    }
                };
                if removed != 1 {
                    return Err(Error::Conflict {
                        resource: "authentication method".to_owned(),
                        message: "method disappeared during conditional removal".to_owned(),
                    });
                }
                Ok(true)
            })
        })
        .await
    }
}
