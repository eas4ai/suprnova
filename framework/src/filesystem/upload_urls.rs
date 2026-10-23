//! The layer that answers a disk's presigned uploads from the callback
//! `Storage::build_temporary_upload_urls_using` installs.
//!
//! `DiskExt::temporary_upload_url` runs on the `Operator` that
//! `Storage::disk(name)` returns, and an operator does not know its disk
//! name, so a registry entry beside the disk cannot reach that call. The
//! callback therefore goes into the disk's own layer stack, outermost: a
//! presigned write is answered from it before any presigning below it, a
//! read-through disk's included, as Laravel's local and read-through disks
//! ask the callback first. Every other operation passes through untouched.

use super::TemporaryUploadUrl;
use crate::FrameworkError;
use opendal::raw::{
    Layer, OpCopier, OpCopy, OpCreateDir, OpList, OpPresign, OpRead, OpRename, OpStat, OpWrite,
    PresignOperation, PresignedRequest, RpCreateDir, RpPresign, RpRename, RpStat, Service,
    ServiceInfo, Servicer, oio,
};
use opendal::{Capability, Error, ErrorKind, OperationContext, Result};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// The function a disk's presigned uploads come from: it receives the path
/// and the lifetime the caller asked for.
pub(crate) type UploadUrlCallback =
    dyn Fn(&str, Duration) -> Result<TemporaryUploadUrl, FrameworkError> + Send + Sync;

/// [`Layer`] that answers a presigned write from an application callback.
#[derive(Clone)]
pub(crate) struct UploadUrlLayer {
    callback: Arc<UploadUrlCallback>,
}

impl UploadUrlLayer {
    /// A layer that answers presigned writes from `callback`.
    pub(crate) fn new(callback: Arc<UploadUrlCallback>) -> Self {
        Self { callback }
    }
}

impl fmt::Debug for UploadUrlLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadUrlLayer").finish_non_exhaustive()
    }
}

impl Layer for UploadUrlLayer {
    fn apply_service(&self, inner: Servicer) -> Servicer {
        Arc::new(UploadUrlService {
            inner,
            callback: Arc::clone(&self.callback),
        })
    }
}

/// The service [`UploadUrlLayer`] builds.
pub(crate) struct UploadUrlService {
    inner: Servicer,
    callback: Arc<UploadUrlCallback>,
}

impl fmt::Debug for UploadUrlService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadUrlService")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

/// The opendal form of the upload the callback answered, so
/// `DiskExt::temporary_upload_url` reads it back as it reads a backend's.
fn presigned(path: &str, upload: TemporaryUploadUrl) -> Result<PresignedRequest> {
    let refused = |what: &str| {
        Error::new(
            ErrorKind::Unexpected,
            format!("the temporary upload URL callback for '{path}' answered {what}"),
        )
    };
    let method = http::Method::from_bytes(upload.method.as_bytes())
        .map_err(|_| refused("a method that is not an HTTP method"))?;
    let uri: http::Uri = upload
        .url
        .parse()
        .map_err(|_| refused("a URL that does not parse"))?;
    let mut headers = http::HeaderMap::new();
    for (name, value) in upload.headers {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| refused("a header name that is not one"))?;
        // The value is not quoted: it can carry a token.
        let value = http::HeaderValue::from_str(&value)
            .map_err(|_| refused(&format!("a value for the header '{name}' that is not text")))?;
        headers.append(name, value);
    }
    Ok(PresignedRequest::new(method, uri, headers))
}

impl Service for UploadUrlService {
    type Reader = oio::Reader;
    type Writer = oio::Writer;
    type Lister = oio::Lister;
    type Deleter = oio::Deleter;
    type Copier = oio::Copier;

    fn info(&self) -> ServiceInfo {
        self.inner.info()
    }

    fn capability(&self) -> Capability {
        // The disk presigns writes now, whatever the backend below does:
        // this is what `Storage::provides_temporary_upload_urls` reads.
        let mut capability = self.inner.capability();
        capability.presign = true;
        capability.presign_write = true;
        capability
    }

    async fn create_dir(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpCreateDir,
    ) -> Result<RpCreateDir> {
        self.inner.create_dir(ctx, path, args).await
    }

    async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner.stat(ctx, path, args).await
    }

    fn read(&self, ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
        self.inner.read(ctx, path, args)
    }

    fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
        self.inner.write(ctx, path, args)
    }

    fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
        self.inner.delete(ctx)
    }

    fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
        self.inner.list(ctx, path, args)
    }

    fn copy(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpCopy,
        opts: OpCopier,
    ) -> Result<Self::Copier> {
        self.inner.copy(ctx, from, to, args, opts)
    }

    async fn rename(
        &self,
        ctx: &OperationContext,
        from: &str,
        to: &str,
        args: OpRename,
    ) -> Result<RpRename> {
        self.inner.rename(ctx, from, to, args).await
    }

    async fn presign(
        &self,
        ctx: &OperationContext,
        path: &str,
        args: OpPresign,
    ) -> Result<RpPresign> {
        if matches!(args.operation(), PresignOperation::Write(_)) {
            let upload = (self.callback)(path, args.expire())
                .map_err(|error| Error::new(ErrorKind::Unexpected, error.to_string()))?;
            return presigned(path, upload).map(RpPresign::new);
        }
        self.inner.presign(ctx, path, args).await
    }
}
