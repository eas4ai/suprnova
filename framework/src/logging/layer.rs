//! The `tracing` side of the channels: the layers that carry events to the
//! default channel, and the subscriber built from them.

use super::channel::{LogLevel, LogRecord};
use super::config::{LogConfig, LogFormat};
use super::facade::{
    configured_default, default_sinks, set_default, stream_enabled, validate_environment,
};
use super::init::build_env_filter;
use super::sinks::{replace_placeholders, set_format};
use crate::error::FrameworkError;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::dynamic_filter_fn;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// Check the log settings of the environment and make the channel
/// `LOG_CHANNEL` names the default. The server and the workers call it once
/// the application's bootstrap has run, so a channel the bootstrap defines
/// is known, and a setting that is wrong stops the boot.
///
/// `LOG_STACK`, `LOG_SYSLOG_FACILITY` and `LOG_DAILY_DAYS` are checked
/// whenever they are set, whether or not the default channel uses them.
///
/// # Errors
///
/// When `LOG_CHANNEL`, or a name `LOG_STACK` lists, names no channel; when
/// `LOG_SYSLOG_FACILITY` is no facility or `LOG_DAILY_DAYS` no number; or
/// when the default channel's settings are wrong.
pub fn check_channels() -> Result<(), FrameworkError> {
    validate_environment()?;
    let name = configured_default();
    set_default(&name)
        .map_err(|error| FrameworkError::internal(format!("LOG_CHANNEL is '{name}': {error}")))
}

/// Make the channel `LOG_CHANNEL` names the default if it can be, and
/// stdout otherwise. A subscriber installed before the application's
/// bootstrap uses this: the bootstrap may define the channel, and the boot
/// checks it with [`check_channels`] once the bootstrap has run.
fn default_or_stdout() {
    if set_default(&configured_default()).is_err() {
        let _ = set_default("stdout");
    }
}

/// Apply `config` to the channels once a subscriber built from it has been
/// installed: the default channel, and the format of the lines the file,
/// syslog and driver sinks write, both process-wide.
///
/// Only after the install: a subscriber that was refused because another
/// is already in place leaves that one's configuration as it was, as the
/// refusal promises, rather than switching the live sinks to its format or
/// moving the default channel back to `LOG_CHANNEL`.
pub(crate) fn adopt_installed(config: &LogConfig) {
    default_or_stdout();
    set_format(config.format);
}

/// The output layers: `tracing`'s own formatter for standard output and
/// standard error, each on while the default channel includes it, and the
/// layer that writes events to the default channel's other sinks.
///
/// Building them changes nothing outside them; [`adopt_installed`] applies
/// the configuration once they are installed.
pub(crate) fn output_layers<S>(config: &LogConfig) -> Vec<Box<dyn Layer<S> + Send + Sync>>
where
    S: Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    // A dynamic filter, so a callsite's interest is never cached: the
    // default channel can move with `Log::set_default_channel`, and each
    // event asks again whether the stream is in it.
    let stdout_on = dynamic_filter_fn(|meta, _| stream_enabled(false, meta.level()));
    let stderr_on = dynamic_filter_fn(|meta, _| stream_enabled(true, meta.level()));
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
/// The channels are process-wide, not the subscriber's: building it makes
/// the channel `LOG_CHANNEL` names the default and `config.format` the
/// format of the file, syslog and driver lines for the whole process, as
/// installing it would. Build it only to use it.
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
    set_format(config.format);
    Ok(registry.with(layers))
}

/// Writes each event to the default channel's sinks other than the
/// standard streams: files, syslog, the application's drivers. An event's
/// record carries the fields of the spans it is in, the request span's
/// `request_id` among them, as the stdout formatter shows them.
struct ChannelLayer;

/// The fields a span was created or recorded with, kept for its events.
#[derive(Default)]
struct SpanFields(Vec<(String, String)>);

impl<S> Layer<S> for ChannelLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(fields.context));
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            match extensions.get_mut::<SpanFields>() {
                Some(kept) => {
                    for (key, value) in fields.context {
                        match kept.0.iter_mut().find(|(name, _)| *name == key) {
                            Some(slot) => slot.1 = value,
                            None => kept.0.push((key, value)),
                        }
                    }
                }
                None => extensions.insert(SpanFields(fields.context)),
            }
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
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
        // The spans' fields, outermost first, then the event's own. A name
        // appears once: an inner span's value replaces an outer span's, and
        // the event's own replaces both, so the message's placeholders and
        // the context written beside it read the same value.
        if let Some(scope) = ctx.event_scope(event) {
            let mut inherited: Vec<(String, String)> = Vec::new();
            for span in scope.from_root() {
                if let Some(kept) = span.extensions().get::<SpanFields>() {
                    for (key, value) in &kept.0 {
                        if fields.context.iter().any(|(name, _)| name == key) {
                            continue;
                        }
                        match inherited.iter_mut().find(|(name, _)| name == key) {
                            Some(slot) => slot.1.clone_from(value),
                            None => inherited.push((key.clone(), value.clone())),
                        }
                    }
                }
            }
            inherited.append(&mut fields.context);
            fields.context = inherited;
        }
        let record = LogRecord {
            time: crate::clock::now(),
            level,
            target: event.metadata().target().to_owned(),
            message: replace_placeholders(&fields.message, &fields.context),
            context: fields.context,
        };
        for (sink, minimum) in sinks {
            if level.passes(minimum) {
                // Each sink reports its own failure once on stderr, a
                // driver's through its `ReportedSink`.
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
