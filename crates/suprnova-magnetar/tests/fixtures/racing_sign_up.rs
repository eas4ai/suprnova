//! A user store that loses a sign-up race.
//!
//! Its first email lookup misses, as one of two concurrent sign-ups sees
//! before the other inserts. It refuses a second account for an address
//! that already has one, with the conflict a store with a unique email index
//! reports, as the default schema's does. Every other call reaches the
//! store it wraps.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use magnetar::storage::{CredentialActor, NewUser, UserRecord, UserStore};

pub struct RacingSignUp {
    inner: Arc<dyn UserStore>,
    missed: AtomicBool,
}

impl RacingSignUp {
    pub fn new(inner: Arc<dyn UserStore>) -> Self {
        Self {
            inner,
            missed: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl UserStore for RacingSignUp {
    async fn find_by_email(&self, email: &str) -> magnetar::Result<Option<UserRecord>> {
        if !self.missed.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        self.inner.find_by_email(email).await
    }
    async fn find_by_id(&self, user_id: &str) -> magnetar::Result<Option<UserRecord>> {
        self.inner.find_by_id(user_id).await
    }
    async fn create_user(&self, input: NewUser) -> magnetar::Result<UserRecord> {
        if self.inner.find_by_email(&input.email).await?.is_some() {
            return Err(magnetar::Error::Conflict {
                resource: "user".to_owned(),
                message: "an account with this email address already exists".to_owned(),
            });
        }
        self.inner.create_user(input).await
    }
    async fn set_password_hash(&self, actor: &CredentialActor, hash: &str) -> magnetar::Result<()> {
        self.inner.set_password_hash(actor, hash).await
    }
    async fn mark_email_verified(&self, user_id: &str, at: DateTime<Utc>) -> magnetar::Result<()> {
        self.inner.mark_email_verified(user_id, at).await
    }
    async fn lock_if_unlocked_by_email(
        &self,
        email: &str,
        at: DateTime<Utc>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        self.inner
            .lock_if_unlocked_by_email(email, at, window_start)
            .await
    }
    async fn set_locked_at_by_email(
        &self,
        email: &str,
        at: Option<DateTime<Utc>>,
    ) -> magnetar::Result<()> {
        self.inner.set_locked_at_by_email(email, at).await
    }
}
