//! The SDK's transport interface has a bounded, counted reqwest sender.

use super::event::CapturedEvent;
use super::pending::Pending;
use super::{ReportingConfig, ReportingMetrics};
use crate::FrameworkError;
use sentry::{Envelope, Transport};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) queued: AtomicU64,
    pub(crate) delivered: AtomicU64,
    pub(crate) dropped: AtomicU64,
    pub(crate) duplicates: AtomicU64,
}

impl Counters {
    pub(crate) fn snapshot(&self) -> ReportingMetrics {
        ReportingMetrics {
            queued: self.queued.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            duplicates: self.duplicates.load(Ordering::Relaxed),
        }
    }
}

pub(crate) struct QueueTransport {
    sender: SyncSender<Message>,
    pending: Arc<Pending>,
    pub(crate) counters: Arc<Counters>,
}

enum Message {
    Encoded(Vec<u8>),
    Captured(CapturedEvent),
}

impl Message {
    fn encode(self) -> Result<Vec<u8>, FrameworkError> {
        match self {
            Self::Encoded(bytes) => Ok(bytes),
            Self::Captured(captured) => {
                let event = serde_json::from_value(captured.resolve().payload).map_err(|_| {
                    FrameworkError::internal("reporting payload could not be encoded")
                })?;
                let mut envelope = Envelope::new();
                envelope.add_item(sentry::protocol::EnvelopeItem::Event(Box::new(event)));
                encode_envelope(envelope)
            }
        }
    }
}

fn encode_envelope(envelope: Envelope) -> Result<Vec<u8>, FrameworkError> {
    let mut bytes = Vec::new();
    envelope
        .to_writer(&mut bytes)
        .map_err(|_| FrameworkError::internal("reporting envelope could not be encoded"))?;
    if bytes.len() > 262_144 {
        return Err(FrameworkError::internal(
            "reporting envelope exceeds its size limit",
        ));
    }
    Ok(bytes)
}

impl QueueTransport {
    pub(crate) fn new(
        config: &ReportingConfig,
        dsn: &sentry::types::Dsn,
    ) -> Result<Arc<Self>, FrameworkError> {
        let (sender, receiver) = sync_channel::<Message>(config.queue_capacity);
        let pending = Arc::new(Pending::default());
        let counters = Arc::new(Counters::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| {
                FrameworkError::internal("could not start the reporting sender runtime")
            })?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                FrameworkError::internal("could not configure the reporting HTTP client")
            })?;
        let url = dsn.envelope_api_url();
        let auth = dsn.to_auth(Some("suprnova-reporting")).to_string();
        let worker_pending = pending.clone();
        let worker_counters = counters.clone();
        std::thread::Builder::new()
            .name("suprnova-reporting".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    let body =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| message.encode()))
                            .ok()
                            .and_then(Result::ok);
                    let delivered = body.is_some_and(|body| {
                        runtime.block_on(async {
                            client
                                .post(url.clone())
                                .header("X-Sentry-Auth", &auth)
                                .header("Content-Type", "application/x-sentry-envelope")
                                .body(body)
                                .send()
                                .await
                                .is_ok_and(|response| response.status().is_success())
                        })
                    });
                    if delivered {
                        worker_counters.delivered.fetch_add(1, Ordering::Relaxed);
                    } else {
                        worker_counters.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    worker_pending.finish();
                }
            })
            .map_err(|_| FrameworkError::internal("could not start the reporting sender thread"))?;
        Ok(Arc::new(Self {
            sender,
            pending,
            counters,
        }))
    }

    pub(crate) fn flush_pending(&self, timeout: Duration) -> bool {
        self.pending.wait(timeout)
    }

    /// Share the bounded sender without resolving symbols on the caller.
    pub(crate) fn send_capture(&self, captured: CapturedEvent) {
        self.enqueue(Message::Captured(captured));
    }

    fn enqueue(&self, message: Message) {
        self.pending.begin();
        match self.sender.try_send(message) {
            Ok(()) => {
                self.counters.queued.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                self.pending.finish();
            }
        }
    }
}

impl Transport for QueueTransport {
    fn send_envelope(&self, envelope: Envelope) {
        let Ok(bytes) = encode_envelope(envelope) else {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        self.enqueue(Message::Encoded(bytes));
    }
    fn flush(&self, timeout: Duration) -> bool {
        self.flush_pending(timeout)
    }
    fn shutdown(&self, timeout: Duration) -> bool {
        self.flush_pending(timeout)
    }
}
