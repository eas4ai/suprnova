//! Shared recording/counting fakes used by every domain harness
//! (`password_harness.rs`, `oauth_harness.rs`, ...). Touches only
//! unconditional modules (`magnetar::plugin`, `magnetar::abuse`), so it is
//! safe to include from suites that enable no plugin feature at all.

#![allow(dead_code)]

use async_trait::async_trait;
use magnetar::Result;
use magnetar::abuse::{AbuseLimiter, AbusePolicy, Permit};
use magnetar::plugin::{LinkGenerator, MailDriver, MailMessage};
use parking_lot::Mutex;
use serde_json::Value;

/// Recording mail driver.
#[derive(Default)]
pub struct RecordingMail {
    /// Every message handed to the driver, in order.
    pub sent: Mutex<Vec<MailMessage>>,
    /// When set, every send fails (notification-failure paths).
    pub fail: Mutex<bool>,
}

#[async_trait]
impl MailDriver for RecordingMail {
    async fn send(&self, message: MailMessage) -> Result<()> {
        if *self.fail.lock() {
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "mail".into(),
                message: "harness failure".into(),
            });
        }
        self.sent.lock().push(message);
        Ok(())
    }
}

impl RecordingMail {
    pub fn count(&self) -> usize {
        self.sent.lock().len()
    }
    pub fn last_payload(&self) -> Option<Value> {
        self.sent
            .lock()
            .last()
            .map(|message| message.payload.clone())
    }
    pub fn names(&self) -> Vec<String> {
        self.sent
            .lock()
            .iter()
            .map(|message| message.name.clone())
            .collect()
    }
}

/// Limiter behavior selected per test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimiterMode {
    Allow,
    Reject,
    Error,
}

/// Counting abuse-limiter fake.
pub struct CountingLimiter {
    /// Every `(key, max_requests)` acquisition, in order.
    pub acquired: Mutex<Vec<(String, u32)>>,
    /// Behavior applied to every acquisition.
    pub mode: Mutex<LimiterMode>,
}

impl Default for CountingLimiter {
    fn default() -> Self {
        Self {
            acquired: Mutex::new(Vec::new()),
            mode: Mutex::new(LimiterMode::Allow),
        }
    }
}

impl CountingLimiter {
    pub fn count(&self) -> usize {
        self.acquired.lock().len()
    }
    pub fn keys(&self) -> Vec<String> {
        self.acquired
            .lock()
            .iter()
            .map(|(key, _)| key.clone())
            .collect()
    }
    pub fn set_mode(&self, mode: LimiterMode) {
        *self.mode.lock() = mode;
    }
}

#[async_trait]
impl AbuseLimiter for CountingLimiter {
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit> {
        self.acquired
            .lock()
            .push((key.to_owned(), policy.max_requests));
        match *self.mode.lock() {
            LimiterMode::Allow => Ok(Permit::Allowed { retry_after: None }),
            LimiterMode::Reject => Ok(Permit::Rejected {
                retry_after: std::time::Duration::from_secs(30),
            }),
            LimiterMode::Error => Err(magnetar::Error::DependencyUnavailable {
                dependency: "limiter".into(),
                message: "harness outage".into(),
            }),
        }
    }
}

/// Deterministic link generator.
pub struct TestLinks;

#[async_trait]
impl LinkGenerator for TestLinks {
    async fn url_for(&self, route_name: &str, params: &[(String, String)]) -> Result<String> {
        let query = params
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        Ok(format!("https://app.test/{route_name}?{query}"))
    }
}
