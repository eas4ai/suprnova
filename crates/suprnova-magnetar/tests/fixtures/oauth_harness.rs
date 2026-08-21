//! Shared composition harness for the OAuth-domain suites (Task 2).
//!
//! Composes the real SeaORM stores over in-memory SQLite (reusing
//! `storage_schema`'s fixture entities and `LinkedAccountStore` addition)
//! plus the shared recording/counting fakes from `fakes.rs`, so these
//! suites build under `--features oauth,seaorm-sqlite` alone without
//! depending on password-domain plugins.

#![allow(dead_code)]

use std::sync::Arc;

use magnetar::crypto::AeadEncryptor;
use magnetar::storage::SeaOrmStorage;

use super::storage_schema::{StorageSchema, database};

#[path = "fakes.rs"]
mod fakes;
#[allow(unused_imports)]
pub use fakes::{CountingLimiter, LimiterMode, RecordingMail, TestLinks};

/// The composed world one OAuth suite operates in.
pub struct OAuthHarness {
    pub db: sea_orm::DatabaseConnection,
    pub storage: Arc<SeaOrmStorage<StorageSchema>>,
    pub encryptor: Arc<AeadEncryptor>,
    pub mail: Arc<RecordingMail>,
    pub limiter: Arc<CountingLimiter>,
    pub links: Arc<TestLinks>,
}

/// Compose the OAuth harness.
pub async fn harness() -> OAuthHarness {
    let db = database().await;
    let storage = Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
    let encryptor = Arc::new(AeadEncryptor::new([9; 32]));
    let mail = Arc::new(RecordingMail::default());
    let limiter = Arc::new(CountingLimiter::default());
    let links = Arc::new(TestLinks);
    OAuthHarness {
        db,
        storage,
        encryptor,
        mail,
        limiter,
        links,
    }
}
