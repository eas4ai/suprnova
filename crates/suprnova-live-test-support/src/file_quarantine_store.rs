//! Tokio-backed file quarantine adapter for tests and the thin reference host.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use suprnova_live::resource::PermitPool;
use suprnova_live::upload::{
    QuarantineBytes, QuarantineObject, QuarantineOperation, QuarantineStore, RemoveDisposition,
    UploadError, UploadErrorKind,
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
    maximum_observed_write: Arc<AtomicUsize>,
    maximum_observed_read: Arc<AtomicUsize>,
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
            maximum_observed_write: Arc::new(AtomicUsize::new(0)),
            maximum_observed_read: Arc::new(AtomicUsize::new(0)),
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
    fn create_exclusive(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        if let Err(error) = self.require_healthy(FileStoreFault::Create) {
            return QuarantineOperation::ready(Err(error));
        }
        let descriptor = match self.acquire_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
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

    fn write_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        bytes: &[u8],
    ) -> QuarantineOperation<()> {
        if let Err(error) = self.require_healthy(FileStoreFault::Write) {
            return QuarantineOperation::ready(Err(error));
        }
        if bytes.is_empty() || bytes.len() > self.maximum_read_bytes {
            return QuarantineOperation::ready(Err(UploadError::new(
                UploadErrorKind::InputTooLarge,
            )));
        }
        let descriptor = match self.acquire_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        let bytes = QuarantineBytes::copy_from_slice(bytes);
        let fragment_limit = self.write_fragment_limit.load(Ordering::SeqCst);
        let maximum_observed = Arc::clone(&self.maximum_observed_write);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let mut file = OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            write_all_fragmented(&mut file, bytes.as_ref(), fragment_limit, &maximum_observed).await
        })
    }

    fn sync(&self, object: &QuarantineObject) -> QuarantineOperation<()> {
        if let Err(error) = self.require_healthy(FileStoreFault::Sync) {
            return QuarantineOperation::ready(Err(error));
        }
        let descriptor = match self.acquire_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let file = OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(provider_error)?;
            file.sync_data().await.map_err(provider_error)
        })
    }

    fn read_at(
        &self,
        object: &QuarantineObject,
        offset: u64,
        maximum_bytes: usize,
    ) -> QuarantineOperation<QuarantineBytes> {
        if let Err(error) = self.require_healthy(FileStoreFault::Read) {
            return QuarantineOperation::ready(Err(error));
        }
        if maximum_bytes == 0 || maximum_bytes > self.maximum_read_bytes {
            return QuarantineOperation::ready(Err(UploadError::new(
                UploadErrorKind::InputTooLarge,
            )));
        }
        let descriptor = match self.acquire_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        let fragment = self.read_fragment_limit.load(Ordering::SeqCst);
        let maximum_observed = Arc::clone(&self.maximum_observed_read);
        spawn_operation(async move {
            let _descriptor = descriptor;
            let mut file = File::open(path).await.map_err(provider_error)?;
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(provider_error)?;
            let requested = if fragment == 0 {
                maximum_bytes
            } else {
                maximum_bytes.min(fragment)
            };
            let mut bytes = vec![0_u8; requested];
            let read = file.read(&mut bytes).await.map_err(provider_error)?;
            bytes.truncate(read);
            maximum_observed.fetch_max(read, Ordering::SeqCst);
            Ok(QuarantineBytes::from(bytes))
        })
    }

    fn remove(&self, object: &QuarantineObject) -> QuarantineOperation<RemoveDisposition> {
        if let Err(error) = self.require_healthy(FileStoreFault::Remove) {
            return QuarantineOperation::ready(Err(error));
        }
        let descriptor = match self.acquire_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => return QuarantineOperation::ready(Err(error)),
        };
        let path = self.path_for(object);
        spawn_operation(async move {
            let _descriptor = descriptor;
            match tokio::fs::remove_file(path).await {
                Ok(()) => Ok(RemoveDisposition::Removed),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(RemoveDisposition::AlreadyAbsent)
                }
                Err(error) => Err(provider_error(error)),
            }
        })
    }
}

fn spawn_operation<T: Clone + Send + 'static>(
    future: impl Future<Output = Result<T, UploadError>> + Send + 'static,
) -> QuarantineOperation<T> {
    let (operation, completion) = QuarantineOperation::pending();
    tokio::spawn(async move {
        completion.complete(future.await);
    });
    operation
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
