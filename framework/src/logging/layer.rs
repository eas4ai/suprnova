//! The `tracing` side of the channels: the layers that carry events to the
//! default channel, and the subscriber built from them.

use super::channel::{LogLevel, LogRecord};
use super::config::{LogConfig, LogFormat};
use super::facade::{configured_default, default_sinks, set_default, stream_enabled};
use super::init::build_env_filter;
use super::sinks::{replace_placeholders, set_format};
use crate::error::FrameworkError;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// Check the channels `LOG_CHANNEL` and `LOG_STACK` name, and make the one
/// `LOG_CHANNEL` names the default. The server and the workers call it
/// before they install the subscriber, so a channel that does not exist
/// stops the boot.
///
/// # Errors
///
/// When `LOG_CHANNEL`, or a channel a stack lists, names no channel, or a
/// channel's settings are wrong, such as an unknown syslog facility.
pub fn check_channels() -> Result<(), FrameworkError> {
    let name = configured_default();
    set_default(&name)
        .map_err(|error| FrameworkError::internal(format!("LOG_CHANNEL is '{name}': {error}")))
}

/// The output layers: `tracing`'s own formatter for standard output and
/// standard error, each on while the default channel includes it, and the
/// layer that writes events to the default channel's other sinks.
pub(crate) fn output_layers<S>(config: &LogConfig) -> Vec<Box<dyn Layer<S> + Send + Sync>>
where
    S: Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    set_format(config.format);
    let stdout_on = filter_fn(|meta| stream_enabled(false, meta.level()));
    let stderr_on = filter_fn(|meta| stream_enabled(true, meta.level()));
    let (stdout, stderr): (
        Box<dyn Layer<S> + Send + Sync>,
        Box<dyn Layer<S> + Send + Sync>,
    ) = match config.format {
        LogFormat::Pretty => (
            Box::new(
                fmt::layer()
                    .with_target(true)
                    .with_thread_ids(false)
                    .pretty()
                    .with_filter(stdout_on),
            ),
            Box::new(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(true)
                    .with_thread_ids(false)
                    .pretty()
                    .with_filter(stderr_on),
            ),
        ),
        LogFormat::Json => (
            Box::new(
                fmt::layer()
                    .json()
                    .with_target(true)
                    .with_current_span(true)
                    .with_filter(stdout_on),
            ),
            Box::new(
                fmt::layer()
                    .json()
                    .with_writer(std::io::stderr)
                    .with_target(true)
                    .with_current_span(true)
                    .with_filter(stderr_on),
            ),
        ),
    };
    vec![stdout, stderr, Box::new(ChannelLayer)]
}

/// Build the subscriber the server installs, without installing it: the
/// `LOG_LEVEL` filter and the output layers, with the default channel
/// checked and set first. For an application that installs its own, or a
/// test that installs one for a thread with
/// `tracing::subscriber::with_default`.
///
/// # Errors
///
/// As [`check_channels`].
pub fn build_subscriber(
    config: LogConfig,
) -> Result<impl Subscriber + Send + Sync + 'static, FrameworkError> {
    check_channels()?;
    let registry = tracing_subscriber::registry().with(build_env_filter(&config.level));
    let layers = output_layers(&config);
    Ok(registry.with(layers))
}

/// Writes each event to the default channel's sinks other than the
/// standard streams: files, syslog, the application's drivers.
struct ChannelLayer;

impl<S: Subscriber> Layer<S> for ChannelLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let sinks = default_sinks();
        if sinks.is_empty() {
            return;
        }
        let level = LogLevel::from(*event.metadata().level());
        if !sinks.iter().any(|(_, minimum)| level.passes(*minimum)) {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let record = LogRecord {
            time: crate::clock::now(),
            level,
            target: event.metadata().target().to_owned(),
            message: replace_placeholders(&fields.message, &fields.context),
            context: fields.context,
        };
        for (sink, minimum) in sinks {
            if level.passes(minimum) {
                let _ = sink.write(&record);
            }
        }
    }
}

/// An event's message and its other fields, as text.
#[derive(Default)]
struct Fields {
    message: String,
    context: Vec<(String, String)>,
}

impl Fields {
    fn push(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = value;
        } else {
            self.context.push((field.name().to_owned(), value));
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value.to_owned());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.push(field, format!("{value:?}"));
    }
}
