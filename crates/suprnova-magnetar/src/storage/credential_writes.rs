use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

use super::{AuthTransaction, SeaOrmStorage, db_error};
use crate::auth::VerifiedPrincipal;
use crate::schema::{AuthSchema, EntityBinding, SessionEpoch, SessionFields, UserFields};
use crate::sessions::{SessionCarrier, VerifiedSession};
use crate::{Error, Result};

/// Provenance required to authorize one credential mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialActor {
    user_id: String,
    issuance_epoch: u64,
    opaque_session_id: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl CredentialActor {
    /// Bind a credential mutation to a session produced by a trusted
    /// [`crate::sessions::SessionQueries`] implementation. Host/provider code
    /// must never construct this witness from untrusted request fields. A
    /// session id is retained only when an opaque store can prove that row is
    /// still live.
    #[must_use]
    pub fn from_session(session: &VerifiedSession) -> Self {
        Self {
            user_id: session.user_id().to_owned(),
            issuance_epoch: session.auth_epoch(),
            opaque_session_id: (session.carrier() == SessionCarrier::Opaque)
                .then(|| session.session_id().to_owned()),
            expires_at: Some(session.expires_at()),
        }
    }

    /// Bind a credential mutation to a freshly verified primary principal.
    #[must_use]
    pub fn from_verified_primary(principal: &VerifiedPrincipal) -> Self {
        Self::verified_primary(principal.user_id(), principal.context().auth_epoch)
    }

    /// Bind a credential mutation to an epoch observed while verifying a
    /// primary credential.
    pub(crate) fn verified_primary(user_id: &str, issuance_epoch: u64) -> Self {
        Self {
            user_id: user_id.to_owned(),
            issuance_epoch,
            opaque_session_id: None,
            expires_at: None,
        }
    }

    /// Return the authenticated user's application identifier.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Return the authentication epoch captured when this actor was issued.
    #[must_use]
    pub fn issuance_epoch(&self) -> u64 {
        self.issuance_epoch
    }

    /// Return the bound opaque session identifier, when the actor uses one.
    #[must_use]
    pub fn opaque_session_id(&self) -> Option<&str> {
        self.opaque_session_id.as_deref()
    }

    /// Return the actor expiry snapshot, when its source has an expiry.
    #[must_use]
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    pub(crate) fn from_snapshot(
        user_id: String,
        issuance_epoch: u64,
        opaque_session_id: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            user_id,
            issuance_epoch,
            opaque_session_id,
            expires_at,
        }
    }
}

fn stale_actor() -> Error {
    Error::NotFound {
        resource: "credential actor".to_owned(),
        identifier: "expired or revoked".to_owned(),
    }
}

/// Run a credential mutation only while its authenticating actor remains live.
///
/// Custom credential stores use this extension point to bind their write to the
/// actor's session and authentication epoch. The supplied operation must perform
/// the mutation through the provided transaction; it must not open a nested
/// transaction or write through the store's outer database connection.
pub async fn fenced_credential_write<S, T, F>(
    storage: &SeaOrmStorage<S>,
    actor: &CredentialActor,
    operation: F,
) -> Result<T>
where
    S: AuthSchema,
    S::User: UserFields + SessionEpoch,
    S::Session: SessionFields,
    F: for<'a> FnOnce(
        &'a mut AuthTransaction<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    if actor.user_id().is_empty() {
        return Err(Error::InvalidInput {
            field: "user_id".to_owned(),
            message: "user identifier must be non-empty".to_owned(),
        });
    }

    let mut database_transaction = storage.database().begin().await.map_err(db_error)?;
    let mut transaction = AuthTransaction::new(&mut database_transaction);

    let user_id_column = S::User::user_id_column();
    let epoch_column = S::User::auth_epoch_column();
    let fence_result = async {
        // This conditional write is deliberately the first database action. It
        // acquires write authority without trusting backend-specific no-op row
        // counts when the epoch is assigned to its existing value.
        <<S::User as EntityBinding>::Entity as EntityTrait>::update_many()
            .col_expr(epoch_column, Expr::col(epoch_column).into())
            .filter(user_id_column.eq(S::User::user_id_value(&actor.user_id)))
            .filter(epoch_column.eq(S::User::auth_epoch_value(actor.issuance_epoch)?))
            .exec(transaction.connection())
            .await
            .map_err(db_error)?;

        let user = <<S::User as EntityBinding>::Entity as EntityTrait>::find()
            .filter(user_id_column.eq(S::User::user_id_value(&actor.user_id)))
            .one(transaction.connection())
            .await
            .map_err(db_error)?
            .ok_or_else(stale_actor)?;
        if S::User::auth_epoch(&user) != actor.issuance_epoch {
            return Err(stale_actor());
        }
        if actor
            .expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(stale_actor());
        }

        if let Some(session_id) = &actor.opaque_session_id {
            let session_id_column = S::Session::session_id_column();
            let session_user_id_column = S::Session::user_id_column();
            let session_epoch_column = S::Session::auth_epoch_column();
            let revoked_at_column = S::Session::revoked_at_column();
            <<S::Session as EntityBinding>::Entity as EntityTrait>::update_many()
                .col_expr(session_epoch_column, Expr::col(session_epoch_column).into())
                .filter(session_id_column.eq(session_id.to_owned()))
                .filter(session_user_id_column.eq(S::Session::user_id_value(&actor.user_id)))
                .filter(revoked_at_column.is_null())
                .filter(
                    session_epoch_column.eq(S::Session::auth_epoch_value(actor.issuance_epoch)?),
                )
                .exec(transaction.connection())
                .await
                .map_err(db_error)?;
            let now = Utc::now();

            let session = <<S::Session as EntityBinding>::Entity as EntityTrait>::find()
                .filter(session_id_column.eq(session_id.to_owned()))
                .one(transaction.connection())
                .await
                .map_err(db_error)?
                .ok_or_else(stale_actor)?;
            if S::Session::read_session_id(&session) != *session_id
                || S::Session::read_user_id(&session) != actor.user_id
                || S::Session::read_auth_epoch(&session)? != actor.issuance_epoch
                || S::Session::read_expires_at(&session) <= now
                || S::Session::read_revoked_at(&session).is_some()
            {
                return Err(stale_actor());
            }
        }

        Ok(())
    }
    .await;

    let result = match fence_result {
        Ok(()) => operation(&mut transaction).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(value) => database_transaction
            .commit()
            .await
            .map_err(db_error)
            .map(|()| value),
        Err(error) => {
            let _ = database_transaction.rollback().await;
            Err(error)
        }
    }
}

#[cfg(all(test, feature = "seaorm-sqlite"))]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::{Duration, Utc};
    use sea_orm::Database;

    use super::{CredentialActor, fenced_credential_write};
    use crate::default_schema::DefaultAuthSchema;
    use crate::sessions::{SessionCarrier, SessionMetadata, VerifiedSession};
    use crate::storage::{NewUser, SeaOrmStorage, UserStore};

    #[tokio::test]
    async fn expired_jwt_actor_is_rejected_before_credential_write_closure_runs() {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        crate::default_schema::migrate(&database).await.unwrap();
        let storage = SeaOrmStorage::<DefaultAuthSchema>::new(database);
        let user = storage
            .create_user(NewUser {
                email: "expired-actor@example.test".to_owned(),
                password_hash: None,
            })
            .await
            .unwrap();
        let actor = CredentialActor::from_session(&VerifiedSession::new(
            SessionCarrier::Jwt,
            "expired-jwt-session".to_owned(),
            user.user_id,
            user.auth_epoch,
            Utc::now() - Duration::seconds(1),
            SessionMetadata::default(),
        ));
        let closure_invoked = Arc::new(AtomicBool::new(false));
        let marker = Arc::clone(&closure_invoked);

        let error = fenced_credential_write(&storage, &actor, move |_transaction| {
            Box::pin(async move {
                marker.store(true, Ordering::SeqCst);
                Ok(())
            })
        })
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            crate::Error::NotFound { resource, identifier }
                if resource == "credential actor" && identifier == "expired or revoked"
        ));
        assert!(!closure_invoked.load(Ordering::SeqCst));
    }
}
