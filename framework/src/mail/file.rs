//! File mail transport - writes one RFC 5322 `.eml` per message.
//!
//! `MailDriver::Log` renders a message into a `tracing::info!` and drops
//! it, which is unreadable for anything with markup, attachments, or
//! headers that matter. This driver writes the same bytes SMTP would put on
//! the wire, so you can open the result in a mail client and check what a
//! recipient actually receives.

use crate::error::FrameworkError;
use crate::mail::transport::{MailTransport, OutgoingMessage};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes each outgoing message to `dir` as a `.eml` file.
///
/// Non-delivering by design: nothing leaves the process. `MailDriver::File`
/// reports `delivers() == false`, so a production boot refuses this driver
/// unless the operator sets `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`.
pub struct FileMailTransport {
    dir: PathBuf,
    /// Disambiguates messages written inside the same millisecond. Without
    /// it a burst of mail from one request overwrites itself.
    seq: AtomicU64,
}

impl FileMailTransport {
    /// Construct a transport writing into `dir`. The directory is created
    /// on first send if it does not exist.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            seq: AtomicU64::new(0),
        }
    }

    fn next_filename(&self) -> String {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        format!("{millis}-{seq}.eml")
    }
}

#[async_trait]
impl MailTransport for FileMailTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        let builder = crate::mail::mime::base_builder(msg)?;
        // `true`: a message with neither body would otherwise produce a
        // zero-part `multipart/alternative`, which no mail client opens.
        // The guard is enabled here and nowhere else so SMTP's wire output
        // stays exactly as it was.
        let multipart = crate::mail::mime::build_body(msg, true)?;
        let email = builder
            .multipart(multipart)
            .map_err(|e| FrameworkError::internal(format!("mail file build message: {e}")))?;

        tokio::fs::create_dir_all(&self.dir).await.map_err(|e| {
            FrameworkError::internal(format!(
                "mail file transport could not create {}: {e}",
                self.dir.display()
            ))
        })?;

        let path = self.dir.join(self.next_filename());
        tokio::fs::write(&path, email.formatted())
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "mail file transport could not write {}: {e}",
                    path.display()
                ))
            })?;

        tracing::info!(path = %path.display(), "mail (file driver): wrote message");
        Ok(())
    }

    fn name(&self) -> &'static str {
        "file"
    }
}
