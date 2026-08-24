//! Generic, application-bound SeaORM storage primitives.
//!
//! Magnetar never names an application table or column. The [`AuthSchema`](crate::schema::AuthSchema)
//! descriptor and its typed field capabilities supply every identifier used by
//! the stores in this module.

use rand::RngCore;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};
use secrecy::SecretString;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use crate::schema::AuthSchema;
use crate::{Error, Result};

pub mod accounts;
pub mod ceremonies;
pub(crate) mod credential_writes;
pub mod device;
pub mod lockout;
pub mod methods;
pub mod migrations;
pub mod passkeys;
pub mod provider_tokens;
mod seaorm;
pub mod tokens;
pub mod users;

#[cfg(test)]
mod credential_write_tests;

pub use accounts::{
    LinkedAccountInitializer, LinkedAccountRecord, LinkedAccountStore, NewLinkedAccount,
};
pub use ceremonies::{CeremonyRecord, CeremonyStore, NewCeremony};
pub use credential_writes::{CredentialActor, fenced_credential_write};
pub use device::{DeviceRecord, DeviceStore};
pub use lockout::{AttemptStats, LockoutStore};
pub use methods::MethodStore;
pub use passkeys::{PasskeyRow, PasskeyStore};
pub use provider_tokens::{
    CommitProviderToken, NewProviderToken, ProviderTokenRow, ProviderTokenStore,
};
pub use seaorm::SeaOrmStorage;
pub use tokens::{
    ConsumedToken, IssueToken, IssuedToken, PASSWORD_RESET_PURPOSE, PasswordResetCommit,
    PasswordResetInput, PasswordResetStore, PresentedToken, TokenStore,
};
pub use users::{NewUser, UserRecord, UserStore};

/// A transaction borrowed by a composite storage operation.
///
/// The wrapper is intentionally non-owning: a caller that receives one must
/// commit or roll back the underlying transaction. [`TokenStore::consume_in`]
/// only borrows this value and never starts or commits it.
pub struct AuthTransaction<'a> {
    pub(crate) transaction: &'a mut DatabaseTransaction,
}

impl<'a> AuthTransaction<'a> {
    /// Borrow a SeaORM transaction for a composite operation.
    pub fn new(transaction: &'a mut DatabaseTransaction) -> Self {
        Self { transaction }
    }

    /// Access the underlying transaction for application-owned operations.
    pub fn connection(&self) -> &DatabaseTransaction {
        self.transaction
    }
}

/// Convert a SeaORM error into the crate's stable error boundary.
pub(crate) fn db_error(error: DbErr) -> Error {
    Error::Internal {
        message: format!("storage database error: {error}"),
    }
}

/// Generate an opaque identifier suitable for application ID accessors.
pub(crate) fn random_id() -> String {
    (rand::random::<u64>() % i64::MAX as u64).to_string()
}
/// Generate a 256-bit bearer secret using the operating system CSPRNG.
pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Convert a secret wrapper into its plaintext representation only inside the
/// storage boundary.
pub(crate) fn expose_secret(value: &SecretString) -> &str {
    secrecy::ExposeSecret::expose_secret(value)
}

/// A generic SeaORM storage operation that owns and commits one transaction.
pub(crate) async fn in_transaction<T, F>(db: &DatabaseConnection, operation: F) -> Result<T>
where
    F: for<'a> FnOnce(
        &'a mut AuthTransaction<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>,
{
    let mut transaction = db.begin().await.map_err(db_error)?;
    let mut borrowed = AuthTransaction::new(&mut transaction);
    match operation(&mut borrowed).await {
        Ok(value) => transaction.commit().await.map_err(db_error).map(|()| value),
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

/// Common constructor for typed stores.
#[derive(Clone)]
pub struct Store<S: AuthSchema> {
    /// Application-owned SeaORM connection.
    pub(crate) db: DatabaseConnection,
    /// Schema role marker.
    pub(crate) marker: PhantomData<S>,
}

impl<S: AuthSchema> Store<S> {
    /// Bind a store to an application-owned SeaORM connection.
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            marker: PhantomData,
        }
    }

    /// Borrow the configured database connection.
    pub fn database(&self) -> &DatabaseConnection {
        &self.db
    }
}
