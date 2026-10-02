//! Device-authorization ceremony reads and single-winner transitions.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::{CeremonyRecord, CeremonyStore, SeaOrmStorage};
use crate::Result;
use crate::schema::AuthSchema;

const DEVICE_KIND: &str = "device-authorization";

/// Device authorization record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRecord {
    /// Application-owned ceremony id.
    pub id: String,
    /// Device selector shown to the user.
    pub selector: String,
    /// Current device state.
    pub state: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Opaque device payload.
    pub payload: Vec<u8>,
}

impl From<CeremonyRecord> for DeviceRecord {
    fn from(value: CeremonyRecord) -> Self {
        Self {
            id: value.id,
            selector: value.selector,
            state: value.state,
            expires_at: value.expires_at,
            payload: value.payload,
        }
    }
}

/// Device state storage API.
#[async_trait]
pub trait DeviceStore: Send + Sync {
    /// Read an unexpired device request without deleting it.
    async fn peek_device(&self, selector: &str) -> Result<Option<DeviceRecord>>;
    /// Conditionally transition one state and report the single winner.
    async fn transition_device(&self, selector: &str, expected: &str, next: &str) -> Result<bool>;
    /// Approve a pending request.
    async fn approve_device(&self, selector: &str) -> Result<bool>;
    /// Deny a pending request.
    async fn deny_device(&self, selector: &str) -> Result<bool>;
}

#[async_trait]
impl<S> DeviceStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    Self: CeremonyStore,
{
    async fn peek_device(&self, selector: &str) -> Result<Option<DeviceRecord>> {
        <Self as CeremonyStore>::peek(self, selector, DEVICE_KIND)
            .await
            .map(|record| record.map(DeviceRecord::from))
    }

    async fn transition_device(&self, selector: &str, expected: &str, next: &str) -> Result<bool> {
        <Self as CeremonyStore>::transition(self, selector, DEVICE_KIND, expected, next).await
    }

    async fn approve_device(&self, selector: &str) -> Result<bool> {
        self.transition_device(selector, "pending", "approved")
            .await
    }

    async fn deny_device(&self, selector: &str) -> Result<bool> {
        self.transition_device(selector, "pending", "denied").await
    }
}
