//! Observe the facade's resolved store once, preserving every driver's own atomic operations.

use crate::{CacheStore, ConditionalIncrement, FrameworkError};
use async_trait::async_trait;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

pub(super) struct ObservedCache(pub(super) Arc<dyn CacheStore>);

impl ObservedCache {
    fn observe<T: Debug + ?Sized>(&self, key: &str, operation: &str, value: &T) {
        if !super::active() {
            return;
        }
        if crate::error::redaction::names_a_secret(key) {
            super::record(
                "cache",
                operation,
                &(self.0.name(), key, crate::error::redaction::REDACTED),
            );
        } else {
            super::record("cache", operation, &(self.0.name(), key, value));
        }
    }
}

#[async_trait]
impl CacheStore for ObservedCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        let result = self.0.get_raw(key).await;
        let outcome = match &result {
            Ok(value) => {
                if value.is_some() {
                    "hit"
                } else {
                    "miss"
                }
            }
            Err(_) => "error",
        };
        self.observe(key, outcome, &result);
        result
    }

    async fn get_raw_capped(
        &self,
        key: &str,
        max: usize,
    ) -> Result<Option<String>, FrameworkError> {
        let result = self.0.get_raw_capped(key, max).await;
        let outcome = match &result {
            Ok(Some(_)) => "hit",
            Ok(None) => "miss",
            Err(_) => "error",
        };
        self.observe(key, outcome, &result);
        result
    }

    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        let result = self.0.put_raw(key, value, ttl).await;
        self.observe(key, "write", value);
        result
    }

    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        let result = self.0.add_raw(key, value, ttl).await;
        self.observe(key, "add", value);
        result
    }

    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        let result = self.0.has(key).await;
        let outcome = match &result {
            Ok(value) => {
                if *value {
                    "hit"
                } else {
                    "miss"
                }
            }
            Err(_) => "error",
        };
        self.observe(key, outcome, &result);
        result
    }

    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        let result = self.0.forget(key).await;
        self.observe(key, "forget", &result);
        result
    }

    async fn flush(&self) -> Result<(), FrameworkError> {
        let result = self.0.flush().await;
        self.observe("*", "flush", &result);
        result
    }

    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let result = self.0.increment(key, amount).await;
        self.observe(key, "increment", &result);
        result
    }

    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let result = self.0.decrement(key, amount).await;
        self.observe(key, "decrement", &result);
        result
    }

    async fn increment_if_below(
        &self,
        key: &str,
        amount: i64,
        ceiling: i64,
    ) -> Result<ConditionalIncrement, FrameworkError> {
        let result = self.0.increment_if_below(key, amount, ceiling).await;
        self.observe(key, "increment_if_below", &result);
        result
    }

    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        let result = self.0.tagged_put_raw(tags, key, value, ttl).await;
        self.observe(key, "tagged_write", value);
        result
    }

    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        let result = self.0.flush_tags(tags).await;
        self.observe("*", "flush_tags", &result);
        result
    }

    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        let result = self.0.acquire_lock(key, ttl).await;
        self.observe(key, "lock", "<lock credential>");
        result
    }

    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        let result = self.0.release_lock(key, token).await;
        self.observe(key, "unlock", &result);
        result
    }

    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        let result = self.0.refresh_lock(key, token, ttl).await;
        self.observe(key, "refresh_lock", &result);
        result
    }

    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        let result = self.0.touch(key, ttl).await;
        self.observe(key, "touch", &result);
        result
    }

    fn name(&self) -> &str {
        self.0.name()
    }
    fn default_ttl(&self) -> Option<Duration> {
        self.0.default_ttl()
    }
    fn locks_are_shared(&self) -> bool {
        self.0.locks_are_shared()
    }
    fn set_connection(&self, name: &str) -> Result<(), FrameworkError> {
        self.0.set_connection(name)
    }
}
