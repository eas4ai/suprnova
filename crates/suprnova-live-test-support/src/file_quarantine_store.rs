//! Tokio-backed file quarantine adapter for tests and the thin reference host.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use suprnova_live::resource::PermitPool;
use suprnova_live::upload::{
    QuarantineBytes, QuarantineObject, QuarantineStore, RemoveDisposition, UploadError,
    UploadErrorKind, UploadFuture,
};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

const MAX_STORE_READ_BYTES: usize = 64 * 1024 * 1024;

/// One deterministic raw file-store fault injection point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileStoreFault {
    /// All bounded file operations remain healthy.
    None = 0,
    /// Exclusive object creation fails.
    Create = 1,
    /// Positional writes fail.
    Write = 2,
    /// Data synchronization fails.
    Sync = 3,
    /// Positional reads fail.
    Read = 4,
    /// Idempotent removal fails.
    Remove = 5,
}

impl FileStoreFault {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Create,
            2 => Self::Write,
            3 => Self::Sync,
            4 => Self::Read,
            5 => Self::Remove,
            _ => Self::None,
        }
    }
}

/// Async file store rooted beneath one canonical test-owned directory.
pub struct TokioFileQuarantineStore {
    root: PathBuf,
    descriptors: PermitPool,
    maximum_read_bytes: usize,
    fault: AtomicU8,
    write_fragment_limit: AtomicUsize,
    read_fragment_limit: AtomicUsize,
    maximum_observed_write: AtomicUsize,
    maximum_observed_read: AtomicUsize,
}

impl TokioFileQuarantineStore {
    /// Opens one existing canonical directory with finite descriptor and read bounds.
    pub async fn open(
        root: impl AsRef<Path>,
        maximum_descriptors: usize,
        maximum_read_bytes: usize,
    ) -> Result<Self, UploadError> {
        if maximum_read_bytes == 0 || maximum_read_bytes > MAX_STORE_READ_BYTES {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(provider_error)?;
        let metadata = tokio::fs::metadata(&root).await.map_err(provider_error)?;
        if !metadata.is_dir() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let descriptors = PermitPool::new(maximum_descriptors)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            root,
            descriptors,
            maximum_read_bytes,
            fault: AtomicU8::new(FileStoreFault::None as u8),
            write_fragment_limit: AtomicUsize::new(0),
            read_fragment_limit: AtomicUsize::new(0),
            maximum_observed_write: AtomicUsize::new(0),
            maximum_observed_read: AtomicUsize::new(0),
        })
    }

    /// Selects one deterministic file operation fault.
    pub fn set_fault(&self, fault: FileStoreFault) {
        self.fault.store(fault as u8, Ordering::SeqCst);
    }

    /// Limits each underlying write call while preserving write-all semantics.
    pub fn set_write_fragment_limit(&self, maximum: Option<usize>) {
        self.write_fragment_limit
            .store(maximum.unwrap_or(0), Ordering::SeqCst);
    }

    /// Limits each returned positional read to emulate legal short reads.
    pub fn set_read_fragment_limit(&self, maximum: Option<usize>) {
        self.read_fragment_limit
            .store(maximum.unwrap_or(0), Ordering::SeqCst);
    }

    /// Returns the largest underlying write observed by this adapter.
    #[must_use]
    pub fn maximum_observed_write(&self) -> usize {
        self.maximum_observed_write.load(Ordering::SeqCst)
    }

    /// Returns the largest returned positional read observed by this adapter.
    #[must_use]
    pub fn maximum_observed_read(&self) -> usize {
        self.maximum_observed_read.load(Ordering::SeqCst)
    }

    /// Resolves an opaque object for assertions without exposing a serving root.
    #[must_use]
    pub fn path_for_test(&self, object: &QuarantineObject) -> PathBuf {
        self.path_for(object)
    }

    fn path_for(&self, object: &QuarantineObject) -> PathBuf {
        self.root.join(object.storage_key())
    }

    fn require_healthy(&self, operation: FileStoreFault) -> Result<(), UploadError> {
        if FileStoreFault::from_u8(self.fault.load(Ordering::SeqCst)) == operation {
            Err(UploadError::new(UploadErrorKind::ProviderUnavailable))
        } else {
            Ok(())
        }
    }

    fn acquire_descriptor(&self) -> Result<suprnova_live::resource::Permit, UploadError> {
        self.descriptors
            .try_acquire()
            .map_err(|_| UploadError::new(UploadErrorKind::ResourceExhausted))
    }
}

impl QuarantineStore for TokioFileQuarantineStore {
    fn create_exclusive<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.require_healthy(FileStoreFault::Create)?;
            let _descriptor = self.acquire_descriptor()?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path_for(object))
                .await
            {
                Ok(_) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(UploadError::new(UploadErrorKind::StorageConflict))
                }
                Err(error) => Err(provider_error(error)),
            }
        })
    }

    fn write_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        bytes: &'a [u8],
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.require_healthy(FileStoreFault::Write)?;
            if bytes.is_empty() || bytes.len() > self.maximum_read_bytes {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let _descriptor = self.acquire_descriptor()?;
            let mut file = OpenOptions::new()
                .write(true)
                .open(self.path_for(object))
                .await
                .map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            write_all_fragmented(
                &mut file,
                bytes,
                self.write_fragment_limit.load(Ordering::SeqCst),
                &self.maximum_observed_write,
            )
            .await
        })
    }

    fn sync<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.require_healthy(FileStoreFault::Sync)?;
            let _descriptor = self.acquire_descriptor()?;
            let file = OpenOptions::new()
                .write(true)
                .open(self.path_for(object))
                .await
                .map_err(provider_error)?;
            file.sync_data().await.map_err(provider_error)
        })
    }

    fn read_at<'a>(
        &'a self,
        object: &'a QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move {
            self.require_healthy(FileStoreFault::Read)?;
            if maximum_bytes == 0 || maximum_bytes > self.maximum_read_bytes {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let _descriptor = self.acquire_descriptor()?;
            let mut file = File::open(self.path_for(object))
                .await
                .map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            let fragment = self.read_fragment_limit.load(Ordering::SeqCst);
            let requested = if fragment == 0 {
                maximum_bytes
            } else {
                maximum_bytes.min(fragment)
            };
            let mut bytes = vec![0_u8; requested];
            let read = file.read(&mut bytes).await.map_err(provider_error)?;
            bytes.truncate(read);
            self.maximum_observed_read.fetch_max(read, Ordering::SeqCst);
            Ok(QuarantineBytes::from(bytes))
        })
    }

    fn remove<'a>(
        &'a self,
        object: &'a QuarantineObject,
    ) -> UploadFuture<'a, Result<RemoveDisposition, UploadError>> {
        Box::pin(async move {
            self.require_healthy(FileStoreFault::Remove)?;
            let _descriptor = self.acquire_descriptor()?;
            match tokio::fs::remove_file(self.path_for(object)).await {
                Ok(()) => Ok(RemoveDisposition::Removed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(RemoveDisposition::AlreadyAbsent)
                }
                Err(error) => Err(provider_error(error)),
            }
        })
    }
}

async fn write_all_fragmented(
    file: &mut File,
    mut bytes: &[u8],
    fragment_limit: usize,
    maximum_observed: &AtomicUsize,
) -> Result<(), UploadError> {
    while !bytes.is_empty() {
        let requested = if fragment_limit == 0 {
            bytes.len()
        } else {
            bytes.len().min(fragment_limit)
        };
        let written = file
            .write(&bytes[..requested])
            .await
            .map_err(provider_error)?;
        if written == 0 {
            return Err(UploadError::new(UploadErrorKind::ProviderUnavailable));
        }
        maximum_observed.fetch_max(written, Ordering::SeqCst);
        bytes = &bytes[written..];
    }
    Ok(())
}

fn provider_error(_error: std::io::Error) -> UploadError {
    UploadError::new(UploadErrorKind::ProviderUnavailable)
}
