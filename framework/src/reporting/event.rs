use super::ReportingConfig;
use crate::{App, FrameworkError};
use serde_json::{Value, json};

static STACK_ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Separate protocol item kinds let logs and traces use the same bounded sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingKind {
    /// An exception event appears in the tracker's issue view.
    Event,
    /// A structured log appears in the tracker's log view.
    Log,
    /// A sampled request or job appears in its performance view.
    Transaction,
}

/// An owned protocol payload keeps driver implementations independent of the SDK.
#[derive(Debug, Clone)]
pub struct ReportingEvent {
    /// The protocol item kind keeps logs separate from exception events.
    pub kind: ReportingKind,
    /// A Sentry-compatible payload, scrubbed before a driver receives it.
    pub payload: Value,
}

impl ReportingEvent {
    /// Preserve the error's type, causes and reporting stack when it is dropped.
    pub fn from_error(error: &FrameworkError) -> Self {
        let config = App::get::<ReportingConfig>().unwrap_or_default();
        Self::from_error_with_config(error, &config)
    }

    /// Use a standalone driver's configuration before the privacy boundary is applied.
    pub fn from_error_with_config(error: &FrameworkError, config: &ReportingConfig) -> Self {
        CapturedEvent::from_error(error, config).resolve()
    }

    /// Freeze scrubbed diagnostics before a queued driver checks capacity or duplicates.
    pub(crate) fn snapshot(error: &FrameworkError, config: &ReportingConfig) -> Self {
        let report = crate::ErrorReport::from_error(error);
        let mut stack = json!({"frames":[]});
        if let Some(frames) = report.frames() {
            append_location(&mut stack, frames.site());
        }
        let values: Vec<_> = report.chain().iter().rev().map(|message| {
            json!({"type":report.type_name().unwrap_or("Error"),"value":message,"stacktrace":stack})
        }).collect();
        let mut payload = base(config);
        payload["level"] = json!("error");
        payload["exception"] = json!({"values":values});
        let _ = PANIC.try_with(|(message, location)| {
            if let Some(location) = location {
                append_location(&mut stack, location);
            }
            payload["exception"]["values"] =
                json!([{"type":"panic","value":message,"stacktrace":stack}]);
        });
        payload["extra"]["context"] = json!(crate::Context::all());
        super::scope::enrich(&mut payload, config.send_pii);
        crate::error::redaction::scrub(&mut payload, config.send_pii);
        Self {
            kind: ReportingKind::Event,
            payload,
        }
    }

    fn hook_snapshot(message: &str, location: Option<&str>, config: &ReportingConfig) -> Self {
        let mut payload = base(config);
        let mut stack = json!({"frames":[]});
        if let Some(location) = location {
            append_location(&mut stack, location);
        }
        payload["level"] = json!("error");
        payload["exception"] =
            json!({"values":[{"type":"panic","value":message,"stacktrace":stack}]});
        crate::error::redaction::scrub(&mut payload, config.send_pii);
        Self {
            kind: ReportingKind::Event,
            payload,
        }
    }
}

/// Keep request-owned diagnostics and raw addresses together until a worker resolves them.
pub(crate) struct CapturedEvent {
    /// Deduplicate scrubbed diagnostics without resolving their symbols.
    pub(crate) event: ReportingEvent,
    backtrace: std::backtrace::Backtrace,
    send_pii: bool,
}

impl CapturedEvent {
    /// Freeze task-local context before the request or job scope ends.
    pub(crate) fn from_error(error: &FrameworkError, config: &ReportingConfig) -> Self {
        Self::capture(ReportingEvent::snapshot(error, config), config.send_pii)
    }

    fn capture(event: ReportingEvent, send_pii: bool) -> Self {
        Self {
            event,
            backtrace: std::backtrace::Backtrace::force_capture(),
            send_pii,
        }
    }

    /// Busy resolution must drop diagnostics instead of blocking on std's backtrace lock.
    pub(crate) fn try_capture(event: ReportingEvent, send_pii: bool) -> Option<Self> {
        let _guard = match STACK_ACCESS.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some(Self::capture(event, send_pii))
    }

    /// Keep the unwinding stack without waiting on another report's resolver.
    pub(crate) fn from_hook(
        message: &str,
        location: Option<&str>,
        config: &ReportingConfig,
    ) -> Option<Self> {
        Self::try_capture(
            ReportingEvent::hook_snapshot(message, location, config),
            config.send_pii,
        )
    }

    /// Resolve on a worker while keeping the original recorded location last.
    pub(crate) fn resolve(mut self) -> ReportingEvent {
        let stack = {
            let _guard = STACK_ACCESS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            stacktrace(&self.backtrace)
        };
        if let Some(values) = self.event.payload["exception"]["values"].as_array_mut() {
            for value in values {
                let mut resolved = stack.clone();
                if let Some(frames) = resolved["frames"].as_array_mut() {
                    let locations = value["stacktrace"]["frames"].as_array();
                    let reserved = locations.map_or(0, Vec::len).min(128);
                    frames.truncate(128 - reserved);
                    frames.extend(locations.into_iter().flatten().take(128).cloned());
                }
                value["stacktrace"] = resolved;
            }
        }
        crate::error::redaction::scrub(&mut self.event.payload, self.send_pii);
        self.event
    }
}

pub(crate) fn base(config: &ReportingConfig) -> Value {
    let release = match config.release.as_deref() {
        Some(release) => release,
        None => crate::boot::default_build_id().unwrap_or("unknown"),
    };
    let host = std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".into());
    json!({"event_id":uuid::Uuid::new_v4().simple().to_string(),"platform":"rust","release":release,"environment":config.environment,"server_name":host,"timestamp":super::scope::timestamp(crate::clock::now().into())})
}

fn stacktrace(backtrace: &std::backtrace::Backtrace) -> Value {
    let text = backtrace.to_string();
    #[cfg(feature = "reporting-sentry")]
    if let Some(stack) = sentry::integrations::backtrace::parse_stacktrace(&text)
        && let Ok(value) = serde_json::to_value(stack)
    {
        return value;
    }
    let frames: Vec<_> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(128)
        .map(|line| json!({"function":line.trim()}))
        .collect();
    json!({"frames":frames})
}

fn append_location(stack: &mut Value, location: &str) {
    let mut parts = location.rsplitn(3, ':');
    let column = parts.next().and_then(|p| p.parse::<u64>().ok());
    let line = parts.next().and_then(|p| p.parse::<u64>().ok());
    let file = parts.next().unwrap_or(location);
    if let Some(frames) = stack["frames"].as_array_mut() {
        // Scrubbing bounds arrays to 128 entries. Reserve the final entry
        // for the original site so deep framework stacks cannot discard it.
        frames.truncate(127);
        frames.push(json!({"filename":file,"lineno":line,"colno":column,"in_app":true}));
    }
}

tokio::task_local! { static PANIC: (String, Option<String>); }

pub(crate) fn with_panic<T>(message: &str, location: Option<&str>, f: impl FnOnce() -> T) -> T {
    PANIC.sync_scope((message.to_owned(), location.map(str::to_owned)), f)
}

#[cfg(test)]
mod tests {
    use super::append_location;
    use serde_json::json;

    #[test]
    #[cfg(feature = "reporting-sentry")]
    fn report_005_busy_symbol_resolution_drops_without_waiting_and_accepts_after_release() {
        use std::time::{Duration, Instant};
        let reporter = crate::ReportingConfig::default()
            .dsn("http://public@127.0.0.1:1/1")
            .timeout(Duration::from_millis(100))
            .build()
            .expect("reporter");
        let guard = super::STACK_ACCESS.lock().expect("resolution guard");
        let started = Instant::now();
        reporter
            .report_error(&crate::FrameworkError::internal("busy symbol resolution"))
            .expect("best effort report");
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(reporter.metrics().queued, 0);
        assert_eq!(reporter.metrics().dropped, 1);
        drop(guard);
        reporter
            .report_error(&crate::FrameworkError::internal("capture after release"))
            .expect("accepted report");
        assert_eq!(reporter.metrics().queued, 1);
        assert!(reporter.flush(Duration::from_secs(5)).expect("drain"));
    }

    #[test]
    fn a_deep_reporting_stack_keeps_the_original_panic_location_after_scrubbing() {
        let mut stack = json!({"frames":vec![json!({"function":"framework dispatch"}); 256]});
        append_location(&mut stack, "application/component.rs:42:9");
        crate::error::redaction::scrub(&mut stack, false);
        let frames = stack["frames"].as_array().expect("stack frames");
        assert!(frames.len() <= 128);
        assert_eq!(
            frames.last().expect("panic site")["filename"],
            "application/component.rs"
        );
        assert_eq!(frames.last().expect("panic site")["lineno"], 42);
        assert_eq!(frames.last().expect("panic site")["colno"], 9);
    }
}
