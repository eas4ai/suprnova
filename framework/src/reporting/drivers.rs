use super::{Reporter, ReportingEvent};
use crate::FrameworkError;
use std::time::Duration;

/// An explicit no-op driver makes disabling reporting independent of handlers.
pub struct NullReporter;

impl Reporter for NullReporter {
    fn report_error(&self, _error: &FrameworkError) -> Result<(), FrameworkError> {
        Ok(())
    }

    fn report(&self, _event: ReportingEvent) -> Result<(), FrameworkError> {
        Ok(())
    }
    fn flush(&self, _timeout: Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

/// Reports through existing log channels for applications without a tracker.
pub struct LogReporter;

impl Reporter for LogReporter {
    fn report(&self, mut event: ReportingEvent) -> Result<(), FrameworkError> {
        crate::error::redaction::scrub(&mut event.payload, false);
        tracing::error!(event = %event.payload, "reported exception");
        Ok(())
    }
    fn flush(&self, _timeout: Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

/// The official MIT SDK encodes envelopes for any Sentry-protocol tracker.
#[cfg(feature = "reporting-sentry")]
pub struct SentryReporter {
    client: sentry::Client,
    transport: std::sync::Arc<super::transport::QueueTransport>,
    config: super::ReportingConfig,
    seen: std::sync::Mutex<std::collections::HashMap<u64, std::time::Instant>>,
}

#[cfg(feature = "reporting-sentry")]
impl SentryReporter {
    pub(crate) fn new(config: &super::ReportingConfig) -> Result<Self, FrameworkError> {
        let dsn: sentry::types::Dsn = config
            .dsn
            .as_deref()
            .ok_or_else(|| FrameworkError::internal("the sentry driver needs REPORTING_DSN"))?
            .parse()
            .map_err(|_| FrameworkError::internal("REPORTING_DSN must be a valid Sentry DSN"))?;
        let transport = super::transport::QueueTransport::new(config, &dsn)?;
        let mut options = sentry::ClientOptions::default();
        options.dsn = Some(dsn);
        options.default_integrations = false;
        options.transport = Some(std::sync::Arc::new(transport.clone()));
        Ok(Self {
            client: sentry::Client::from_config(options),
            transport,
            config: config.clone(),
            seen: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn repeated(&self, event: &ReportingEvent) -> bool {
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::Ordering;
        if event.kind != super::ReportingKind::Event || self.config.duplicate_window.is_zero() {
            return false;
        }
        let mut hash = std::hash::DefaultHasher::new();
        if let Some(values) = event.payload["exception"]["values"].as_array() {
            for value in values {
                value["type"].to_string().hash(&mut hash);
                value["value"].to_string().hash(&mut hash);
            }
        } else {
            event.payload["message"].to_string().hash(&mut hash);
        }
        let fingerprint = hash.finish();
        let now = std::time::Instant::now();
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.retain(|_, sent| now.duration_since(*sent) < self.config.duplicate_window);
        if seen.contains_key(&fingerprint) {
            self.transport
                .counters
                .duplicates
                .fetch_add(1, Ordering::Relaxed);
            return true;
        }
        if seen.len() >= 1024 {
            self.transport
                .counters
                .dropped
                .fetch_add(1, Ordering::Relaxed);
            return true;
        }
        seen.insert(fingerprint, now);
        false
    }
}

#[cfg(feature = "reporting-sentry")]
impl Reporter for SentryReporter {
    fn report_error(&self, error: &FrameworkError) -> Result<(), FrameworkError> {
        let event = ReportingEvent::snapshot(error, &self.config);
        if self.repeated(&event) {
            return Ok(());
        }
        let Some(captured) = super::event::CapturedEvent::try_capture(event, self.config.send_pii)
        else {
            self.transport
                .counters
                .dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        };
        self.transport.send_capture(captured);
        Ok(())
    }

    fn report(&self, mut event: ReportingEvent) -> Result<(), FrameworkError> {
        if event.kind != super::ReportingKind::Log {
            let metadata = super::event::base(&self.config);
            for field in ["release", "environment", "server_name"] {
                event.payload[field] = metadata[field].clone();
            }
        }
        crate::error::redaction::scrub(&mut event.payload, self.config.send_pii);
        if self.repeated(&event) {
            return Ok(());
        }
        let invalid = |_| FrameworkError::internal("reporting payload could not be encoded");
        match event.kind {
            super::ReportingKind::Event => {
                let event = serde_json::from_value(event.payload).map_err(invalid)?;
                self.client.capture_event(event, None);
            }
            super::ReportingKind::Log => {
                let log = serde_json::from_value(event.payload).map_err(invalid)?;
                let mut envelope = sentry::Envelope::new();
                envelope.add_item(sentry::protocol::EnvelopeItem::ItemContainer(
                    sentry::protocol::ItemContainer::Logs(vec![log]),
                ));
                self.client.send_envelope(envelope);
            }
            super::ReportingKind::Transaction => {
                let transaction = serde_json::from_value(event.payload).map_err(invalid)?;
                let mut envelope = sentry::Envelope::new();
                envelope.add_item(sentry::protocol::EnvelopeItem::Transaction(Box::new(
                    transaction,
                )));
                self.client.send_envelope(envelope);
            }
        }
        Ok(())
    }
    fn flush(&self, timeout: Duration) -> Result<bool, FrameworkError> {
        Ok(self.transport.flush_pending(timeout))
    }
    fn metrics(&self) -> super::ReportingMetrics {
        self.transport.counters.snapshot()
    }
}
