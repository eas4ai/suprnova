//! Storage fencing keeps independent server and console publishers from racing.

use bytes::Bytes;
use opendal::{ErrorKind, Operator};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{Manifest, error};
use crate::FrameworkError;

pub(crate) const LEASE: &str = "_pub/lease.json";
const MANIFEST: &str = "_pub/manifest.json";

#[derive(Serialize, Deserialize)]
struct Record {
    owner: String,
    expires_at_ms: u64,
}

pub(crate) struct StorageLease {
    disk: Operator,
    owner: String,
    released: bool,
    deadline: tokio::time::Instant,
    work: Option<super::work::WorkGuard>,
}

fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
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
        deadline: tokio::time::Instant,
    ) -> Result<Self, FrameworkError> {
        let capability = disk.info().capability();
        if !capability.write_with_if_not_exists && !capability.write_with_if_none_match {
            return Err(error(
                "publication lease requires atomic conditional object creation",
            ));
        }
        let owner = uuid::Uuid::now_v7().to_string();
        let record = Record {
            owner: owner.clone(),
            expires_at_ms: now_ms().saturating_add(
                u64::try_from(timeout.as_millis())
                    .unwrap_or(u64::MAX)
                    .saturating_mul(2)
                    .saturating_add(1000),
            ),
        };
        let bytes = serde_json::to_vec(&record).map_err(error)?;
        let mut create = disk.write_with(LEASE, bytes.clone());
        create = if capability.write_with_if_not_exists {
            create.if_not_exists(true)
        } else {
            create.if_none_match("*")
        };
        match create.await {
            Ok(_) => {}
            Err(failure)
                if matches!(
                    failure.kind(),
                    ErrorKind::AlreadyExists | ErrorKind::ConditionNotMatch
                ) =>
            {
                let metadata = disk.stat(LEASE).await.map_err(error)?;
                let previous = optional_bytes(&disk, LEASE)
                    .await?
                    .ok_or_else(|| error("publication lease changed while acquiring"))?;
                let previous: Record = serde_json::from_slice(&previous).map_err(error)?;
                if previous.expires_at_ms > now_ms() {
                    return Err(error("publication lease is held by another publisher"));
                }
                if !capability.write_with_if_match {
                    return Err(error(
                        "expired publication lease requires operator recovery on a backend without conditional replacement",
                    ));
                }
                let etag = metadata
                    .etag()
                    .ok_or_else(|| error("conditional lease replacement requires an ETag"))?;
                disk.write_with(LEASE, bytes)
                    .if_match(etag)
                    .await
                    .map_err(|failure| error(format!("publication lease changed: {failure}")))?;
            }
            Err(failure) => return Err(error(failure)),
        }
        Ok(Self {
            disk,
            owner,
            released: false,
            deadline,
            work: None,
        })
    }

    pub(crate) fn track(&mut self, work: super::work::WorkGuard) {
        self.work = Some(work);
    }

    pub(crate) async fn verify(&self) -> Result<(), FrameworkError> {
        let bytes = optional_bytes(&self.disk, LEASE)
            .await?
            .ok_or_else(|| error("publication lease disappeared"))?;
        let record: Record = serde_json::from_slice(&bytes).map_err(error)?;
        if record.owner != self.owner || record.expires_at_ms <= now_ms() {
            return Err(error("publication lease changed or expired"));
        }
        Ok(())
    }

    pub(crate) async fn release(&mut self) -> Result<(), FrameworkError> {
        release(&self.disk, &self.owner).await?;
        self.released = true;
        Ok(())
    }
}

async fn release(disk: &Operator, owner: &str) -> Result<(), FrameworkError> {
    let metadata = match disk.stat(LEASE).await {
        Ok(metadata) => metadata,
        Err(failure) if failure.kind() == ErrorKind::NotFound => return Ok(()),
        Err(failure) => return Err(error(failure)),
    };
    let Some(bytes) = optional_bytes(disk, LEASE).await? else {
        return Ok(());
    };
    let record: Record = serde_json::from_slice(&bytes).map_err(error)?;
    if record.owner != owner {
        return Ok(());
    }
    if disk.info().capability().write_with_if_match {
        let etag = metadata
            .etag()
            .ok_or_else(|| error("conditional lease release requires an ETag"))?;
        let record = Record {
            owner: owner.to_string(),
            expires_at_ms: 0,
        };
        disk.write_with(LEASE, serde_json::to_vec(&record).map_err(error)?)
            .if_match(etag)
            .await
            .map_err(error)?;
    } else {
        // These backends never steal expired leases, so only this owner can remove it.
        disk.delete(LEASE).await.map_err(error)?;
    }
    Ok(())
}

impl Drop for StorageLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let disk = self.disk.clone();
        let owner = self.owner.clone();
        let work = self.work.take();
        let deadline = self.deadline;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _work = work;
                match tokio::time::timeout_at(deadline, release(&disk, &owner)).await {
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
