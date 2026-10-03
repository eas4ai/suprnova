//! Generic passkey-credential rows, mirroring torii's repository shape.
//!
//! The fork's `968b0be` honesty discipline is load-bearing here: no surface
//! name implies verification. `find_user_by_credential` is an unverified
//! lookup over a public value (credential ids are public); only the passkey
//! domain's webauthn verification path may treat a row as authenticated.
//! Removal is deliberately absent: deleting a sign-in method goes through
//! the census-guarded [`super::MethodStore`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::credential_writes::fenced_credential_write;
use super::{AuthTransaction, CredentialActor, SeaOrmStorage, db_error, random_id};
use crate::schema::{
    AuthSchema, EntityBinding, PasskeyFields, SessionEpoch, SessionFields, UserFields,
};
use crate::{Error, Result};

/// One stored passkey row, raw as the application persists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasskeyRow {
    /// Application-owned row identifier (the census removal handle).
    pub passkey_id: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Base64-standard credential identifier, as deployed.
    pub credential_id: String,
    /// The serialized `data_json` envelope, as deployed.
    pub envelope_json: String,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Storage API for passkey credentials.
#[async_trait]
pub trait PasskeyStore: Send + Sync {
    /// Insert one credential row carrying the deployed envelope while the
    /// begin-time credential actor remains live.
    async fn insert_passkey(
        &self,
        actor: &CredentialActor,
        credential_id_b64: &str,
        envelope_json: &str,
    ) -> Result<PasskeyRow>;
    /// List every credential row for a user.
    async fn passkeys_for_user(&self, user_id: &str) -> Result<Vec<PasskeyRow>>;
    /// Unverified lookup by public credential identifier. The name follows
    /// the fork's honesty rename: storage cannot authenticate anyone.
    async fn find_user_by_credential(&self, credential_id_b64: &str) -> Result<Option<PasskeyRow>>;
    /// Atomically replace the envelope of exactly one credential row
    /// (post-authentication counter and `last_used_at` rewrite) while the
    /// verified primary actor remains live. A missing row is an internal
    /// inconsistency, not a caller error.
    async fn update_passkey_envelope(
        &self,
        actor: &CredentialActor,
        credential_id_b64: &str,
        envelope_json: &str,
    ) -> Result<()>;
}

fn row<S>(model: &<S::Passkey as EntityBinding>::Model) -> PasskeyRow
where
    S: AuthSchema,
    S::Passkey: PasskeyFields,
{
    PasskeyRow {
        passkey_id: S::Passkey::read_passkey_id(model),
        user_id: S::Passkey::read_user_id(model),
        credential_id: S::Passkey::read_credential_id(model),
        envelope_json: S::Passkey::read_public_key(model),
        created_at: S::Passkey::read_created_at(model),
    }
}

fn empty(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: "must not be empty".to_owned(),
    }
}

async fn insert_passkey_in_transaction<S>(
    transaction: &mut AuthTransaction<'_>,
    user_id: &str,
    credential_id_b64: &str,
    envelope_json: &str,
) -> Result<PasskeyRow>
where
    S: AuthSchema,
    S::Passkey: PasskeyFields,
{
    let passkey_id = random_id();
    let mut model = <S::Passkey as EntityBinding>::ActiveModel::default();
    S::Passkey::write_passkey_id(&mut model, &passkey_id);
    S::Passkey::write_user_id(&mut model, user_id);
    S::Passkey::write_credential_id(&mut model, credential_id_b64);
    S::Passkey::write_public_key(&mut model, envelope_json);
    <S::Passkey as EntityBinding>::Entity::insert(model)
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    let rows = <S::Passkey as EntityBinding>::Entity::find()
        .filter(S::Passkey::passkey_id_column().eq(passkey_id))
        .all(transaction.connection())
        .await
        .map_err(db_error)?;
    rows.first().map(row::<S>).ok_or_else(|| Error::Internal {
        message: "inserted passkey row could not be read back".to_owned(),
    })
}

async fn update_passkey_envelope_in_transaction<S>(
    transaction: &mut AuthTransaction<'_>,
    user_id: &str,
    credential_id_b64: &str,
    envelope_json: &str,
) -> Result<()>
where
    S: AuthSchema,
    S::Passkey: PasskeyFields,
{
    let mut model = <S::Passkey as EntityBinding>::ActiveModel::default();
    S::Passkey::write_public_key(&mut model, envelope_json);
    let update = <S::Passkey as EntityBinding>::Entity::update_many()
        .set(model)
        .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
        .filter(S::Passkey::credential_id_column().eq(credential_id_b64.to_owned()))
        .exec(transaction.connection())
        .await
        .map_err(db_error)?;
    if update.rows_affected != 1 {
        return Err(Error::Internal {
            message: "authenticated passkey row missing during envelope update".to_owned(),
        });
    }
    Ok(())
}
#[async_trait]
impl<S> PasskeyStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::Session: SessionFields,
    S::Passkey: PasskeyFields,
    <S::Passkey as EntityBinding>::Entity: EntityTrait<
            Model = <S::Passkey as EntityBinding>::Model,
            ActiveModel = <S::Passkey as EntityBinding>::ActiveModel,
        >,
    <S::Passkey as EntityBinding>::Column: ColumnTrait,
{
    async fn insert_passkey(
        &self,
        actor: &CredentialActor,
        credential_id_b64: &str,
        envelope_json: &str,
    ) -> Result<PasskeyRow> {
        if actor.user_id().is_empty() {
            return Err(empty("user_id"));
        }
        if credential_id_b64.is_empty() {
            return Err(empty("credential_id"));
        }
        let user_id = actor.user_id().to_owned();
        let credential_id_b64 = credential_id_b64.to_owned();
        let envelope_json = envelope_json.to_owned();
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(async move {
                insert_passkey_in_transaction::<S>(
                    transaction,
                    &user_id,
                    &credential_id_b64,
                    &envelope_json,
                )
                .await
            })
        })
        .await
    }

    async fn passkeys_for_user(&self, user_id: &str) -> Result<Vec<PasskeyRow>> {
        if user_id.is_empty() {
            return Err(empty("user_id"));
        }
        let rows = <S::Passkey as EntityBinding>::Entity::find()
            .filter(S::Passkey::user_id_column().eq(S::Passkey::user_id_value(user_id)))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.iter().map(row::<S>).collect())
    }

    async fn find_user_by_credential(&self, credential_id_b64: &str) -> Result<Option<PasskeyRow>> {
        if credential_id_b64.is_empty() {
            return Err(empty("credential_id"));
        }
        let rows = <S::Passkey as EntityBinding>::Entity::find()
            .filter(S::Passkey::credential_id_column().eq(credential_id_b64.to_owned()))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.first().map(row::<S>))
    }

    async fn update_passkey_envelope(
        &self,
        actor: &CredentialActor,
        credential_id_b64: &str,
        envelope_json: &str,
    ) -> Result<()> {
        if credential_id_b64.is_empty() {
            return Err(empty("credential_id"));
        }
        let user_id = actor.user_id().to_owned();
        let credential_id_b64 = credential_id_b64.to_owned();
        let envelope_json = envelope_json.to_owned();
        fenced_credential_write(self, actor, move |transaction| {
            Box::pin(async move {
                update_passkey_envelope_in_transaction::<S>(
                    transaction,
                    &user_id,
                    &credential_id_b64,
                    &envelope_json,
                )
                .await
            })
        })
        .await
    }
}
