//! Generic, application-bound user reads and credential writes.
//!
//! The user table belongs to the application; this store only performs the
//! operations the authentication domains need, through [`UserFields`] and its
//! sibling capabilities. Method *removal* is deliberately absent here: taking
//! away a sign-in method must go through the census-guarded
//! [`super::MethodStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::{SeaOrmStorage, db_error, random_id};
use crate::schema::{
    AuthSchema, EntityBinding, SessionEpoch, UserFields, UserOptionalFields,
    password_hash_for_verifier,
};
use crate::sessions::JwtEpochStore;
use crate::{Error, Result};

/// A generic view of one application user row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRecord {
    /// Application-owned user identifier.
    pub user_id: String,
    /// Normalized email address.
    pub email: String,
    /// Verifier-safe password hash; `None` for passwordless users. The
    /// NOT-NULL empty sentinel never appears here.
    pub password_hash: Option<String>,
    /// Email-verification timestamp, when stamped.
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Account lock timestamp, when locked.
    pub locked_at: Option<DateTime<Utc>>,
    /// Current authentication epoch.
    pub auth_epoch: u64,
}

/// Input for creating one user row.
#[derive(Clone, Debug)]
pub struct NewUser {
    /// Normalized email address.
    pub email: String,
    /// Already-hashed credential; `None` creates a passwordless user.
    pub password_hash: Option<String>,
}

/// Storage API for generic user reads and credential writes.
#[async_trait]
pub trait UserStore: Send + Sync {
    /// Find one user by normalized email.
    async fn find_by_email(&self, email: &str) -> Result<Option<UserRecord>>;
    /// Find one user by identifier.
    async fn find_by_id(&self, user_id: &str) -> Result<Option<UserRecord>>;
    /// Create a user row and return its stored view.
    async fn create_user(&self, input: NewUser) -> Result<UserRecord>;
    /// Replace the stored credential hash for a user.
    async fn set_password_hash(&self, user_id: &str, password_hash: &str) -> Result<()>;
    /// Stamp the email-verification timestamp.
    async fn mark_email_verified(&self, user_id: &str, at: DateTime<Utc>) -> Result<()>;
    /// Stamp or clear the account lock timestamp by email. Unknown emails are
    /// a silent no-op so lockout bookkeeping cannot leak account existence.
    async fn set_locked_at_by_email(
        &self,
        email: &str,
        locked_at: Option<DateTime<Utc>>,
    ) -> Result<()>;
}

fn record<S>(model: &<S::User as EntityBinding>::Model) -> UserRecord
where
    S: AuthSchema,
    S::User: UserFields + UserOptionalFields + SessionEpoch,
{
    UserRecord {
        user_id: S::User::read_user_id(model),
        email: S::User::read_email(model),
        password_hash: password_hash_for_verifier::<S::User>(model),
        email_verified_at: S::User::read_email_verified_at(model),
        locked_at: S::User::read_locked_at(model),
        auth_epoch: S::User::auth_epoch(model),
    }
}

fn empty(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: "must not be empty".to_owned(),
    }
}

#[async_trait]
impl<S> UserStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + UserOptionalFields + SessionEpoch,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
{
    async fn find_by_email(&self, email: &str) -> Result<Option<UserRecord>> {
        if email.is_empty() {
            return Err(empty("email"));
        }
        let rows = <S::User as EntityBinding>::Entity::find()
            .filter(S::User::email_column().eq(email.to_owned()))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.first().map(record::<S>))
    }

    async fn find_by_id(&self, user_id: &str) -> Result<Option<UserRecord>> {
        if user_id.is_empty() {
            return Err(empty("user_id"));
        }
        let rows = <S::User as EntityBinding>::Entity::find()
            .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.first().map(record::<S>))
    }

    async fn create_user(&self, input: NewUser) -> Result<UserRecord> {
        if input.email.is_empty() {
            return Err(empty("email"));
        }
        let user_id = random_id();
        let mut model = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_user_id(&mut model, &user_id);
        S::User::write_email(&mut model, &input.email);
        S::User::write_password_hash(&mut model, input.password_hash.as_deref());
        S::User::write_locked_at(&mut model, None);
        S::User::write_auth_epoch(&mut model, 0);
        <S::User as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await
            .map_err(db_error)?;
        self.find_by_id(&user_id)
            .await?
            .ok_or_else(|| Error::Internal {
                message: "created user row could not be read back".to_owned(),
            })
    }

    async fn set_password_hash(&self, user_id: &str, password_hash: &str) -> Result<()> {
        if user_id.is_empty() {
            return Err(empty("user_id"));
        }
        if password_hash.is_empty() {
            return Err(empty("password_hash"));
        }
        let mut model = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_password_hash(&mut model, Some(password_hash));
        let update = <S::User as EntityBinding>::Entity::update_many()
            .set(model)
            .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        if update.rows_affected != 1 {
            return Err(Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            });
        }
        Ok(())
    }

    async fn mark_email_verified(&self, user_id: &str, at: DateTime<Utc>) -> Result<()> {
        if user_id.is_empty() {
            return Err(empty("user_id"));
        }
        let mut model = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_email_verified_at(&mut model, Some(at));
        let update = <S::User as EntityBinding>::Entity::update_many()
            .set(model)
            .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        if update.rows_affected != 1 {
            return Err(Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            });
        }
        Ok(())
    }

    async fn set_locked_at_by_email(
        &self,
        email: &str,
        locked_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        if email.is_empty() {
            return Err(empty("email"));
        }
        let mut model = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_locked_at(&mut model, locked_at);
        // Absent emails are a deliberate no-op: lockout state is tracked for
        // unknown identities too, and this write must not become an oracle.
        let _ = <S::User as EntityBinding>::Entity::update_many()
            .set(model)
            .filter(S::User::email_column().eq(email.to_owned()))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(())
    }
}

#[async_trait]
impl<S> JwtEpochStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + UserOptionalFields + SessionEpoch,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
{
    async fn current_auth_epoch(&self, user_id: &str) -> Result<u64> {
        let user = self
            .find_by_id(user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            })?;
        Ok(user.auth_epoch)
    }

    async fn bump_auth_epoch(&self, user_id: &str) -> Result<u64> {
        if user_id.is_empty() {
            return Err(empty("user_id"));
        }
        let update = <S::User as EntityBinding>::Entity::update_many()
            .col_expr(
                S::User::auth_epoch_column(),
                Expr::col(S::User::auth_epoch_column()).add(1),
            )
            .filter(S::User::user_id_column().eq(S::User::user_id_value(user_id)))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        if update.rows_affected != 1 {
            return Err(Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            });
        }
        self.current_auth_epoch(user_id).await
    }
}
