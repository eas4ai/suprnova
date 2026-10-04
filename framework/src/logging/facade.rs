//! The `Log` facade and the channel registry behind it.

use super::channel::{ChannelKind, LogChannel, LogLevel, LogRecord, LogSink, facility_number};
use super::sinks::{
    FileSink, ReportedSink, Rotation, StreamSink, flush_all, register_flushable,
    replace_placeholders,
};
use crate::error::FrameworkError;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A driver added with [`Log::extend`].
type Factory = Arc<dyn Fn(&LogChannel) -> Result<Arc<dyn LogSink>, FrameworkError> + Send + Sync>;

/// Where a resolved channel writes. The standard streams are kept apart
/// because, as the default channel, `tracing`'s own formatter writes them.
#[derive(Clone)]
pub(crate) enum Leaf {
    Stdout(Option<LogLevel>),
    Stderr(Option<LogLevel>),
    Sink(Arc<dyn LogSink>, Option<LogLevel>),
}

impl Leaf {
    /// The same leaf, keeping only what both its own lowest level and
    /// `outer`, the level of a stack that lists it, keep.
    fn within(self, outer: Option<LogLevel>) -> Self {
        let narrowed = |own: Option<LogLevel>| match (own, outer) {
            (Some(own), Some(outer)) => Some(own.min(outer)),
            (own, None) => own,
            (None, outer) => outer,
        };
        match self {
            Leaf::Stdout(level) => Leaf::Stdout(narrowed(level)),
            Leaf::Stderr(level) => Leaf::Stderr(narrowed(level)),
            Leaf::Sink(sink, level) => Leaf::Sink(sink, narrowed(level)),
        }
    }

    /// Whether this leaf writes to `sink`.
    fn writes_to(&self, sink: &Arc<dyn LogSink>) -> bool {
        matches!(self, Leaf::Sink(own, _) if Arc::ptr_eq(own, sink))
    }
}

#[derive(Default)]
struct Registry {
    defined: HashMap<String, LogChannel>,
    drivers: HashMap<String, Factory>,
    resolved: BTreeMap<String, Vec<Leaf>>,
    default: Option<String>,
    default_leaves: Vec<Leaf>,
}

static REGISTRY: LazyLock<RwLock<Registry>> = LazyLock::new(RwLock::default);

/// Whether the default channel includes standard output, and the lowest
/// level it keeps there (8 for every level); the same for standard error.
/// The `tracing` formatters read these on every event.
pub(crate) static STDOUT_ON: AtomicBool = AtomicBool::new(true);
pub(crate) static STDOUT_MIN: AtomicU8 = AtomicU8::new(8);
pub(crate) static STDERR_ON: AtomicBool = AtomicBool::new(false);
pub(crate) static STDERR_MIN: AtomicU8 = AtomicU8::new(8);

/// Set when the default channel was forgotten: the next event resolves it
/// again, as Laravel resolves a forgotten channel on its next use.
static DEFAULT_FORGOTTEN: AtomicBool = AtomicBool::new(false);

fn read() -> RwLockReadGuard<'static, Registry> {
    REGISTRY
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write() -> RwLockWriteGuard<'static, Registry> {
    REGISTRY
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The names of the built-in channels, for the error an unknown one gets.
const BUILT_IN: &str = "stdout, stderr, errorlog, single, daily, monthly, syslog, null, stack";

/// The built-in channel `name`, configured from the environment, if it is
/// one.
fn built_in(name: &str) -> Result<Option<LogChannel>, FrameworkError> {
    let var = |key: &str| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let file = || crate::app::paths::storage_path("logs/suprnova.log");
    Ok(Some(match name {
        "stdout" => LogChannel::stdout(),
        "stderr" | "errorlog" => LogChannel::stderr(),
        "single" => LogChannel::single(file()),
        "daily" => {
            let days = match var("LOG_DAILY_DAYS") {
                Some(days) => days.parse::<u32>().map_err(|_| {
                    FrameworkError::internal(format!(
                        "LOG_DAILY_DAYS is '{days}', and must be a number of days (0 keeps \
                         every file)"
                    ))
                })?,
                None => 14,
            };
            LogChannel::daily(file()).days(days)
        }
        "monthly" => LogChannel::monthly(file()),
        "syslog" => {
            let mut channel = LogChannel::syslog();
            if let Some(facility) = var("LOG_SYSLOG_FACILITY") {
                channel = channel.facility(&facility);
            }
            if let Some(socket) = var("LOG_SYSLOG_SOCKET") {
                channel = channel.socket(socket);
            }
            channel
        }
        "null" => LogChannel::null(),
        "stack" => LogChannel::stack(
            var("LOG_STACK")
                .unwrap_or_else(|| "single".to_owned())
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        _ => return Ok(None),
    }))
}

fn unknown(name: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "the log channel '{name}' does not exist: define it with Log::define in the \
         bootstrap, or name a built-in one ({BUILT_IN})"
    ))
}

/// The sinks of the channel `name`, resolved once and kept.
fn resolve_named(name: &str, depth: u8) -> Result<Vec<Leaf>, FrameworkError> {
    if let Some(leaves) = read().resolved.get(name) {
        return Ok(leaves.clone());
    }
    let channel = match read().defined.get(name).cloned() {
        Some(channel) => channel,
        None => built_in(name)?.ok_or_else(|| unknown(name))?,
    };
    let leaves = build(&channel, depth)?;
    write().resolved.insert(name.to_owned(), leaves.clone());
    Ok(leaves)
}

/// The sinks of `channel`.
fn build(channel: &LogChannel, depth: u8) -> Result<Vec<Leaf>, FrameworkError> {
    let level = channel.level;
    let file = |path: &std::path::Path, rotation| {
        let sink: Arc<dyn LogSink> = Arc::new(FileSink::new(path.to_path_buf(), rotation));
        register_flushable(&sink);
        Leaf::Sink(sink, level)
    };
    Ok(match &channel.kind {
        ChannelKind::Stdout => vec![Leaf::Stdout(level)],
        ChannelKind::Stderr => vec![Leaf::Stderr(level)],
        ChannelKind::Single(path) => vec![file(path, Rotation::None)],
        ChannelKind::Daily { path, days } => vec![file(path, Rotation::Daily(*days))],
        ChannelKind::Monthly { path, months } => vec![file(path, Rotation::Monthly(*months))],
        ChannelKind::Null => Vec::new(),
        ChannelKind::Syslog { facility, socket } => {
            let facility = facility_number(facility.as_deref().unwrap_or("user"))?;
            vec![syslog(facility, socket.clone(), level)?]
        }
        ChannelKind::Stack(names) => {
            if depth > 8 {
                return Err(FrameworkError::internal(
                    "log stacks nest more than eight deep; a stack probably lists itself",
                ));
            }
            // The stack's own level applies on top of each channel's, so a
            // stack at `Warning` drops info even in a channel that keeps
            // every level.
            let mut leaves = Vec::new();
            for name in names {
                leaves.extend(
                    resolve_named(name, depth + 1)?
                        .into_iter()
                        .map(|leaf| leaf.within(level)),
                );
            }
            leaves
        }
        ChannelKind::Driver(driver) => {
            let factory = read().drivers.get(driver).cloned().ok_or_else(|| {
                FrameworkError::internal(format!(
                    "the log driver '{driver}' does not exist: add it with Log::extend"
                ))
            })?;
            // The driver's failures are reported for it, as the `LogSink`
            // contract asks of its caller.
            let sink: Arc<dyn LogSink> = Arc::new(ReportedSink::new(factory(channel)?, driver));
            // A driver may buffer, so it is flushed with the files.
            register_flushable(&sink);
            vec![Leaf::Sink(sink, level)]
        }
    })
}

#[cfg(unix)]
fn syslog(
    facility: u8,
    socket: Option<std::path::PathBuf>,
    level: Option<LogLevel>,
) -> Result<Leaf, FrameworkError> {
    use super::sinks::SyslogSink;
    let socket = socket.unwrap_or_else(SyslogSink::default_socket);
    let sink = SyslogSink::new(socket, facility).map_err(|error| {
        FrameworkError::internal(format!("cannot open a syslog socket: {error}"))
    })?;
    Ok(Leaf::Sink(Arc::new(sink), level))
}

#[cfg(not(unix))]
fn syslog(
    _facility: u8,
    _socket: Option<std::path::PathBuf>,
    _level: Option<LogLevel>,
) -> Result<Leaf, FrameworkError> {
    Err(FrameworkError::internal(
        "the syslog log channel needs a Unix system with a syslog socket",
    ))
}

fn level_code(level: Option<LogLevel>) -> u8 {
    level.map_or(8, LogLevel::severity)
}

/// Make `name` the default channel: the one `tracing` events go to.
pub(crate) fn set_default(name: &str) -> Result<(), FrameworkError> {
    let leaves = resolve_named(name, 0)?;
    let stdout = leaves.iter().find_map(|leaf| match leaf {
        Leaf::Stdout(level) => Some(*level),
        _ => None,
    });
    let stderr = leaves.iter().find_map(|leaf| match leaf {
        Leaf::Stderr(level) => Some(*level),
        _ => None,
    });
    let mut registry = write();
    STDOUT_ON.store(stdout.is_some(), Ordering::Relaxed);
    STDOUT_MIN.store(level_code(stdout.flatten()), Ordering::Relaxed);
    STDERR_ON.store(stderr.is_some(), Ordering::Relaxed);
    STDERR_MIN.store(level_code(stderr.flatten()), Ordering::Relaxed);
    registry.default = Some(name.to_owned());
    registry.default_leaves = leaves;
    Ok(())
}

/// The sinks of the default channel that are not the standard streams.
pub(crate) fn default_sinks() -> Vec<(Arc<dyn LogSink>, Option<LogLevel>)> {
    if DEFAULT_FORGOTTEN.swap(false, Ordering::Relaxed) {
        refresh_default();
    }
    read()
        .default_leaves
        .iter()
        .filter_map(|leaf| match leaf {
            Leaf::Sink(sink, level) => Some((Arc::clone(sink), *level)),
            _ => None,
        })
        .collect()
}

/// Whether a `tracing` event at `level` goes to standard output (or, with
/// `stderr`, standard error) as part of the default channel.
pub(crate) fn stream_enabled(stderr: bool, level: &tracing::Level) -> bool {
    let (on, minimum) = if stderr {
        (&STDERR_ON, &STDERR_MIN)
    } else {
        (&STDOUT_ON, &STDOUT_MIN)
    };
    on.load(Ordering::Relaxed)
        && LogLevel::from(*level).severity() <= minimum.load(Ordering::Relaxed)
}

/// Check the log variables that are set, whether or not the default
/// channel uses them: every name `LOG_STACK` lists is a channel,
/// `LOG_SYSLOG_FACILITY` a facility, and `LOG_DAILY_DAYS` a number.
pub(crate) fn validate_environment() -> Result<(), FrameworkError> {
    let var = |key: &str| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    if let Some(stack) = var("LOG_STACK") {
        for name in stack
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let known = read().defined.contains_key(name) || built_in(name)?.is_some();
            if !known {
                return Err(FrameworkError::internal(format!(
                    "LOG_STACK lists '{name}': {}",
                    unknown(name)
                )));
            }
        }
    }
    if let Some(facility) = var("LOG_SYSLOG_FACILITY") {
        facility_number(&facility)
            .map_err(|error| FrameworkError::internal(format!("LOG_SYSLOG_FACILITY: {error}")))?;
    }
    if var("LOG_DAILY_DAYS").is_some() {
        built_in("daily")?;
    }
    Ok(())
}

/// Resolve the default channel again, after its definition or its sinks
/// changed. A default that no longer resolves keeps its sinks.
fn refresh_default() {
    let current = read().default.clone();
    if let Some(name) = current {
        let _ = set_default(&name);
    }
}

/// The channel `LOG_CHANNEL` names, `stdout` when it is not set.
pub(crate) fn configured_default() -> String {
    std::env::var("LOG_CHANNEL")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "stdout".to_owned())
}

/// The log channels, Laravel's `Log` facade.
///
/// `tracing`'s macros (`info!`, `error!` and the rest) write to the default
/// channel, the one `LOG_CHANNEL` names. `Log` reaches the others: a named
/// channel, a stack of them, or one built on the spot.
///
/// ```rust,no_run
/// use serde_json::json;
/// use suprnova::{Log, LogChannel};
///
/// // bootstrap.rs: a channel of the application's own.
/// Log::define("audit", LogChannel::daily("storage/logs/audit.log").days(90));
///
/// // anywhere
/// Log::channel("audit")?.info_with("user {id} signed in", json!({ "id": 42 }));
/// # Ok::<(), suprnova::FrameworkError>(())
/// ```
pub struct Log;

impl Log {
    /// Add a channel under `name`, or replace the one there. Call it in the
    /// bootstrap, before the server installs the default channel.
    pub fn define(name: &str, channel: LogChannel) {
        {
            let mut registry = write();
            registry.defined.insert(name.to_owned(), channel);
            // A stack that lists the name resolves it again.
            registry.resolved.clear();
        }
        // The default may be the channel, or a stack that lists it.
        refresh_default();
    }

    /// Add a driver: `factory` builds the sink of each channel defined with
    /// [`LogChannel::driver`] and this name. The way to add Slack, a log
    /// service, or anything else.
    pub fn extend<F>(driver: &str, factory: F)
    where
        F: Fn(&LogChannel) -> Result<Arc<dyn LogSink>, FrameworkError> + Send + Sync + 'static,
    {
        write().drivers.insert(driver.to_owned(), Arc::new(factory));
    }

    /// A logger that writes to the channel `name` only.
    ///
    /// # Errors
    ///
    /// When no channel has the name, or its sink cannot be built.
    pub fn channel(name: &str) -> Result<Logger, FrameworkError> {
        Ok(Logger {
            leaves: resolve_named(name, 0)?,
        })
    }

    /// A logger that writes to each channel named.
    ///
    /// # Errors
    ///
    /// As [`channel`](Self::channel), for any of them.
    pub fn stack(names: &[&str]) -> Result<Logger, FrameworkError> {
        let mut leaves = Vec::new();
        for name in names {
            leaves.extend(resolve_named(name, 0)?);
        }
        Ok(Logger { leaves })
    }

    /// A logger for a channel that has no name, built now.
    ///
    /// # Errors
    ///
    /// When its sink cannot be built: a syslog facility that is not one, a
    /// stack that lists an unknown channel, a driver that does not exist.
    pub fn build(channel: LogChannel) -> Result<Logger, FrameworkError> {
        Ok(Logger {
            leaves: build(&channel, 0)?,
        })
    }

    /// The names of the channels resolved so far.
    pub fn channels() -> Vec<String> {
        read().resolved.keys().cloned().collect()
    }

    /// Drop the resolved channel `name`, writing out what it buffered and
    /// closing its files. It is resolved again the next time it is used,
    /// the default channel by the next `tracing` event, which reopens its
    /// file, as Laravel's `forgetChannel` does after a file was rotated
    /// away. A stack that lists the channel is dropped with it, the
    /// default channel among them, so the stack reopens the file too
    /// rather than writing on into the rotated one. A [`Logger`] taken
    /// before keeps the sinks it holds.
    pub fn forget_channel(name: &str) {
        let (forgotten, default_affected) = {
            let mut registry = write();
            let forgotten: Vec<Arc<dyn LogSink>> = registry
                .resolved
                .remove(name)
                .into_iter()
                .flatten()
                .filter_map(|leaf| match leaf {
                    Leaf::Sink(sink, _) => Some(sink),
                    _ => None,
                })
                .collect();
            let shares_a_sink = |leaves: &[Leaf]| {
                leaves
                    .iter()
                    .any(|leaf| forgotten.iter().any(|sink| leaf.writes_to(sink)))
            };
            registry.resolved.retain(|_, leaves| !shares_a_sink(leaves));
            let default_affected = registry.default.as_deref() == Some(name)
                || shares_a_sink(&registry.default_leaves);
            (forgotten, default_affected)
        };
        // Flushed outside the registry lock: a driver's flush may log.
        for sink in &forgotten {
            // A failure is reported by the sink, or by its `ReportedSink`.
            let _ = sink.flush();
        }
        if default_affected {
            DEFAULT_FORGOTTEN.store(true, Ordering::Relaxed);
        }
    }

    /// The default channel: the one `tracing` events go to.
    pub fn default_channel() -> String {
        read().default.clone().unwrap_or_else(configured_default)
    }

    /// Make `name` the default channel; the events that follow go to it.
    ///
    /// # Errors
    ///
    /// When no channel has the name.
    pub fn set_default_channel(name: &str) -> Result<(), FrameworkError> {
        set_default(name)
    }

    /// Write out every buffered file channel. The server and the workers
    /// call it when they shut down.
    pub fn flush() {
        flush_all();
    }
}

/// A logger for one channel, a stack, or a built channel, from [`Log`].
#[derive(Clone)]
pub struct Logger {
    leaves: Vec<Leaf>,
}

impl Logger {
    /// Write `message` at `level`, with `context`: a JSON object whose
    /// values fill the message's `{key}` placeholders and are written
    /// beside it.
    pub fn log(&self, level: LogLevel, message: &str, context: serde_json::Value) {
        let context: Vec<(String, String)> = match context {
            serde_json::Value::Object(map) => map
                .into_iter()
                .map(|(key, value)| {
                    let text = match value {
                        serde_json::Value::String(text) => text,
                        other => other.to_string(),
                    };
                    (key, text)
                })
                .collect(),
            serde_json::Value::Null => Vec::new(),
            other => vec![("context".to_owned(), other.to_string())],
        };
        let record = LogRecord {
            time: crate::clock::now(),
            level,
            target: String::new(),
            message: replace_placeholders(message, &context),
            context,
        };
        for leaf in &self.leaves {
            // Every sink reports its own failure once on stderr, a driver's
            // through its `ReportedSink`, so the result is not needed here:
            // logging never fails its caller.
            let _ = match leaf {
                Leaf::Stdout(minimum) if level.passes(*minimum) => {
                    StreamSink::stdout().write(&record)
                }
                Leaf::Stderr(minimum) if level.passes(*minimum) => {
                    StreamSink::stderr().write(&record)
                }
                Leaf::Sink(sink, minimum) if level.passes(*minimum) => sink.write(&record),
                _ => Ok(()),
            };
        }
    }

    /// `emergency`: the system is unusable.
    pub fn emergency(&self, message: &str) {
        self.log(LogLevel::Emergency, message, serde_json::Value::Null);
    }

    /// `alert`: action must be taken at once.
    pub fn alert(&self, message: &str) {
        self.log(LogLevel::Alert, message, serde_json::Value::Null);
    }

    /// `critical`: a critical condition.
    pub fn critical(&self, message: &str) {
        self.log(LogLevel::Critical, message, serde_json::Value::Null);
    }

    /// `error`.
    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message, serde_json::Value::Null);
    }

    /// `warning`.
    pub fn warning(&self, message: &str) {
        self.log(LogLevel::Warning, message, serde_json::Value::Null);
    }

    /// `notice`: normal but significant.
    pub fn notice(&self, message: &str) {
        self.log(LogLevel::Notice, message, serde_json::Value::Null);
    }

    /// `info`.
    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message, serde_json::Value::Null);
    }

    /// `debug`.
    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message, serde_json::Value::Null);
    }

    /// `info` with a context.
    pub fn info_with(&self, message: &str, context: serde_json::Value) {
        self.log(LogLevel::Info, message, context);
    }

    /// `warning` with a context.
    pub fn warning_with(&self, message: &str, context: serde_json::Value) {
        self.log(LogLevel::Warning, message, context);
    }

    /// `error` with a context.
    pub fn error_with(&self, message: &str, context: serde_json::Value) {
        self.log(LogLevel::Error, message, context);
    }
}
