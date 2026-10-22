//! Epoch records fence publishers using atomic creation on every storage backend.

use bytes::Bytes;
use opendal::{ErrorKind, Operator};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{Manifest, error};
use crate::FrameworkError;

/// Delivery and retention exclude this namespace from public objects.
pub(crate) const LEASE_PREFIX: &str = "_pub/lease/";
const MANIFEST: &str = "_pub/manifest.json";

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    owner: String,
    epoch: u64,
    expires_at_ms: u64,
}

pub(crate) struct StorageLease {
    disk: Operator,
    record: Record,
    released: bool,
    deadline: tokio::time::Instant,
    skew_ms: u64,
    work: Option<super::work::WorkGuard>,
}

fn now_ms() -> u64 {
    u64::try_from(crate::clock::now().timestamp_millis()).unwrap_or(0)
}

fn key(epoch: u64) -> String {
    format!("{LEASE_PREFIX}{epoch:020}.json")
}

/// Foreign keys cannot participate in ownership or epoch retention.
pub(crate) fn epoch_from_key(path: &str) -> Option<u64> {
    let epoch = path.strip_prefix(LEASE_PREFIX)?.strip_suffix(".json")?;
    (epoch.len() == 20 && epoch.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| epoch.parse().ok())
        .flatten()
}

async fn highest(disk: &Operator) -> Result<Option<Record>, FrameworkError> {
    let entries = match disk.list(LEASE_PREFIX).await {
        Ok(entries) => entries,
        Err(failure) if failure.kind() == ErrorKind::NotFound => return Ok(None),
        Err(failure) => return Err(error(failure)),
    };
    let Some(epoch) = entries
        .iter()
        .filter_map(|entry| epoch_from_key(entry.path()))
        .max()
    else {
        return Ok(None);
    };
    let bytes = disk.read(&key(epoch)).await.map_err(error)?.to_bytes();
    let record: Record = serde_json::from_slice(&bytes).map_err(error)?;
    if record.epoch != epoch {
        return Err(error(format!(
            "publication lease epoch {epoch} has mismatched record {}",
            record.epoch
        )));
    }
    Ok(Some(record))
}

async fn optional_bytes(disk: &Operator, key: &str) -> Result<Option<Bytes>, FrameworkError> {
    match disk.read(key).await {
        Ok(bytes) => Ok(Some(bytes.to_bytes())),
        Err(failure) if failure.kind() == ErrorKind::NotFound => Ok(None),
        Err(failure) => Err(error(failure)),
    }
}

impl StorageLease {
    pub(crate) async fn acquire(
        disk: Operator,
        timeout: Duration,
        skew: Duration,
        deadline: tokio::time::Instant,
    ) -> Result<Self, FrameworkError> {
        let capability = disk.info().capability();
        if !capability.write_with_if_not_exists && !capability.write_with_if_none_match {
            return Err(error(
                "publication lease requires atomic conditional object creation",
            ));
        }
        let skew_ms = u64::try_from(skew.as_millis()).unwrap_or(u64::MAX);
        let epoch = match highest(&disk).await? {
            Some(previous) => {
                if now_ms() < previous.expires_at_ms.saturating_add(skew_ms) {
                    return Err(error(format!(
                        "publication lease is held by {} at epoch {}",
                        previous.owner, previous.epoch
                    )));
                }
                previous
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| error("publication lease epoch is exhausted"))?
            }
            None => 1,
        };
        let record = Record {
            owner: uuid::Uuid::now_v7().to_string(),
            epoch,
            expires_at_ms: now_ms().saturating_add(
                u64::try_from(timeout.as_millis())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(2)
                    .saturating_add(skew_ms.saturating_mul(2))
                    .saturating_add(1000),
            ),
        };
        let path = key(epoch);
        let write = disk.write_with(&path, serde_json::to_vec(&record).map_err(error)?);
        let write = if capability.write_with_if_not_exists {
            write.if_not_exists(true)
        } else {
            write.if_none_match("*")
        };
        write.await.map_err(|failure| {
            if matches!(
                failure.kind(),
                ErrorKind::AlreadyExists | ErrorKind::ConditionNotMatch
            ) {
                error(format!(
                    "publication lease is held by another taker at epoch {epoch}: {failure}"
                ))
            } else {
                error(failure)
            }
        })?;
        let lease = Self {
            disk,
            record,
            released: false,
            deadline,
            skew_ms,
            work: None,
        };
        lease.verify().await?;
        Ok(lease)
    }

    pub(crate) fn track(&mut self, work: super::work::WorkGuard) {
        self.work = Some(work);
    }

    /// Retention keeps this epoch and verifies ownership before deleting earlier records.
    pub(crate) fn epoch(&self) -> u64 {
        self.record.epoch
    }

    pub(crate) async fn verify(&self) -> Result<(), FrameworkError> {
        let current = highest(&self.disk)
            .await?
            .ok_or_else(|| error("publication lease disappeared"))?;
        if current.epoch != self.record.epoch
            || current.owner != self.record.owner
            || current.expires_at_ms != self.record.expires_at_ms
            || now_ms() >= current.expires_at_ms.saturating_sub(self.skew_ms)
        {
            return Err(error(format!(
                "publication lease unusable: holder {} epoch {}, current owner {} epoch {}, expiry {}",
                self.record.owner,
                self.record.epoch,
                current.owner,
                current.epoch,
                current.expires_at_ms
            )));
        }
        Ok(())
    }

    pub(crate) async fn release(&mut self) -> Result<(), FrameworkError> {
        release(&self.disk, &self.record).await?;
        self.released = true;
        Ok(())
    }
}

async fn release(disk: &Operator, acquired: &Record) -> Result<(), FrameworkError> {
    // The owner releases only its own epoch; a successor's record is never overwritten.
    let record = Record {
        expires_at_ms: 0,
        ..acquired.clone()
    };
    disk.write(
        &key(record.epoch),
        serde_json::to_vec(&record).map_err(error)?,
    )
    .await
    .map_err(error)?;
    Ok(())
}

impl Drop for StorageLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let disk = self.disk.clone();
        let record = self.record.clone();
        let work = self.work.take();
        let deadline = self.deadline;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _work = work;
                match tokio::time::timeout_at(deadline, release(&disk, &record)).await {
                    Ok(Ok(())) => {}
                    failure => tracing::error!(
                        ?failure,
                        "publication lease cleanup failed; expiry protects the owner"
                    ),
                }
            });
        }
    }
}

pub(crate) struct ManifestSnapshot {
    pub(crate) manifest: Manifest,
    bytes: Option<Bytes>,
    etag: Option<String>,
}
impl ManifestSnapshot {
    pub(crate) async fn load(disk: &Operator) -> Result<Self, FrameworkError> {
        let etag = match disk.stat(MANIFEST).await {
            Ok(metadata) => metadata.etag().map(str::to_string),
            Err(failure) if failure.kind() == ErrorKind::NotFound => None,
            Err(failure) => return Err(error(failure)),
        };
        let bytes = optional_bytes(disk, MANIFEST).await?;
        let manifest = bytes
            .as_ref()
            .map(|bytes| serde_json::from_slice(bytes).map_err(error))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            manifest,
            bytes,
            etag,
        })
    }
    pub(crate) async fn verify(&self, disk: &Operator) -> Result<(), FrameworkError> {
        if optional_bytes(disk, MANIFEST).await? != self.bytes {
            return Err(error("publication manifest changed under this publisher"));
        }
        Ok(())
    }
    pub(crate) async fn commit(
        &self,
        disk: &Operator,
        manifest: &Manifest,
    ) -> Result<(), FrameworkError> {
        self.verify(disk).await?;
        let capability = disk.info().capability();
        let mut write = disk.write_with(MANIFEST, serde_json::to_vec(manifest).map_err(error)?);
        if capability.write_with_cache_control {
            write = write.cache_control("no-store");
        }
        if self.bytes.is_some() && capability.write_with_if_match {
            write = write.if_match(
                self.etag
                    .as_deref()
                    .ok_or_else(|| error("conditional manifest replacement requires an ETag"))?,
            );
        } else if self.bytes.is_none() && capability.write_with_if_not_exists {
            write = write.if_not_exists(true);
        } else if self.bytes.is_none() && capability.write_with_if_none_match {
            write = write.if_none_match("*");
        }
        write
            .await
            .map_err(|failure| error(format!("publication manifest commit refused: {failure}")))?;
        Ok(())
    }
}
