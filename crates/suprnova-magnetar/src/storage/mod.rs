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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::schema::AuthSchema;
use crate::{Error, Result};

pub mod accounts;
pub mod ceremonies;
mod conflict;
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
pub use ceremonies::{
    CeremonyRecord, CeremonyRef, CeremonyStore, NewCeremony, TransitionAndConsume,
};
pub(crate) use conflict::in_retried_transaction;
pub use credential_writes::{CredentialActor, fenced_credential_write};
pub use device::{DeviceRecord, DeviceStore};
pub use lockout::{AttemptFinalization, AttemptReservation, AttemptStats, LockoutStore};
pub use methods::MethodStore;
pub use passkeys::{PasskeyRow, PasskeyStore};
pub use provider_tokens::{
    CommitProviderToken, NewProviderToken, ProviderTokenRow, ProviderTokenStore,
};
pub use seaorm::SeaOrmStorage;
pub use tokens::{
    ConsumedToken, IssueToken, IssuedToken, PASSWORD_RESET_PURPOSE, PasswordResetCommit,
    PasswordResetInput, PasswordResetStore, PresentedToken, TokenState, TokenStore, address_digest,
};
pub use users::{NewUser, UserRecord, UserStore};
pub(crate) use users::{SignUpAccount, create_or_find_existing};

/// A transaction borrowed by a composite storage operation.
///
/// The wrapper is intentionally non-owning: a caller that receives one must
/// commit or roll back the underlying transaction. [`TokenStore::consume_in`]
/// only borrows this value and never starts or commits it.
pub struct AuthTransaction<'a> {
    pub(crate) transaction: &'a mut DatabaseTransaction,
    /// Set when a statement failed because the database ended the
    /// transaction in a conflict with a concurrent one, for
    /// [`in_retried_transaction`] to run it again. `None` for a transaction
    /// nothing runs again.
    conflict: Option<Arc<AtomicBool>>,
}

impl<'a> AuthTransaction<'a> {
    /// Borrow a SeaORM transaction for a composite operation.
    pub fn new(transaction: &'a mut DatabaseTransaction) -> Self {
        Self {
            transaction,
            conflict: None,
        }
    }

    /// Borrow a transaction whose conflicts `conflict` records.
    fn noting_conflicts(
        transaction: &'a mut DatabaseTransaction,
        conflict: Arc<AtomicBool>,
    ) -> Self {
        Self {
            transaction,
            conflict: Some(conflict),
        }
    }

    /// Access the underlying transaction for application-owned operations.
    pub fn connection(&self) -> &DatabaseTransaction {
        self.transaction
    }

    /// The crate's error for a failed statement of this transaction, as
    /// [`db_error`] gives it, after noting whether the database ended the
    /// transaction in a conflict with a concurrent one
    /// ([`conflict::is_transaction_conflict`]). Under
    /// [`in_retried_transaction`] such a failure runs the transaction again;
    /// elsewhere it is an error like any other.
    pub(crate) fn database_error(&self, error: DbErr) -> Error {
        if let Some(conflict) = &self.conflict
            && conflict::is_transaction_conflict(&error)
        {
            conflict.store(true, Ordering::Relaxed);
        }
        db_error(error)
    }

    /// Record a conflict as a failed statement would, for the tests of
    /// [`in_retried_transaction`], which cannot make a database report one.
    #[cfg(test)]
    pub(crate) fn note_conflict(&self) {
        if let Some(conflict) = &self.conflict {
            conflict.store(true, Ordering::Relaxed);
        }
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
    hex_lower(&bytes)
}

/// Lowercase hex of `bytes`, written into one buffer of exactly its length.
/// Formatting each byte with `format!` allocated a string per byte, and
/// collecting them grew the result by doubling.
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from(DIGITS[usize::from(byte >> 4)]));
        hex.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    hex
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

#[cfg(test)]
mod mem_audit {
    use super::*;

    /// MEM-003: hex is written into one buffer of exactly its length.
    #[test]
    fn mem_audit_hex_is_written_into_one_exact_buffer() {
        let hex = hex_lower(&[0x00, 0xab, 0xff]);
        assert_eq!(hex, "00abff");
        assert_eq!(hex.capacity(), hex.len());
        let token = random_token();
        assert_eq!(token.len(), 64);
        assert!(
            token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }

    /// MEM-003: no Magnetar source formats hex one byte at a time.
    #[test]
    fn mem_audit_no_source_formats_hex_a_byte_at_a_time() {
        fn sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    sources(&path, found);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    found.push(path);
                }
            }
        }
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        // Built from two pieces so this test's own source does not match.
        let per_byte = concat!(":02", "x}");
        let mut offenders = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).expect("a source");
            for (number, line) in text.lines().enumerate() {
                if line.contains(".map(") && line.contains(per_byte) {
                    offenders.push(format!("{}:{}", file.display(), number + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "per-byte hex formatting at {offenders:?}"
        );
    }
}
