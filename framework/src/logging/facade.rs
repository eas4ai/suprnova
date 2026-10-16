//! The `Log` facade and the channel registry behind it.

use super::channel::{ChannelKind, LogChannel, LogLevel, LogRecord, LogSink, facility_number};
use super::events::{self, MessageLogged};
use super::sinks::{
    FileSink, ReportedSink, Rotation, StreamSink, flush_all, register_flushable,
    replace_placeholders,
};
use crate::error::FrameworkError;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A driver added with [`Log::extend`].
type Factory = Arc<dyn Fn(&LogChannel) -> Result<Arc<dyn LogSink>, FrameworkError> + Send + Sync>;

/// One place a resolved channel writes, with the lowest level it keeps and
/// whether it replaces `{key}` placeholders. A stack's leaves each keep the
/// form their own channel chose.
#[derive(Clone)]
pub(crate) struct Leaf {
    to: LeafTo,
    level: Option<LogLevel>,
    replace: bool,
}

/// Where a leaf writes. The standard streams are kept apart because, as the
/// default channel, `tracing`'s own formatter writes them.
#[derive(Clone)]
enum LeafTo {
    Stdout,
    Stderr,
    Sink(Arc<dyn LogSink>),
}

impl Leaf {
    fn new(to: LeafTo, channel: &LogChannel) -> Self {
        Self {
            to,
            level: channel.level,
            replace: channel.replace_placeholders,
        }
    }

    /// The same leaf, keeping only what both its own lowest level and
    /// `outer`, the level of a stack that lists it, keep.
    fn within(self, outer: Option<LogLevel>) -> Self {
        let level = match (self.level, outer) {
            (Some(own), Some(outer)) => Some(own.min(outer)),
            (own, None) => own,
            (None, outer) => outer,
        };
        Self { level, ..self }
    }

    /// Whether this leaf writes to `sink`.
    fn writes_to(&self, sink: &Arc<dyn LogSink>) -> bool {
        matches!(&self.to, LeafTo::Sink(own) if Arc::ptr_eq(own, sink))
    }
}

/// A sink of the default channel other than the standard streams, as the
/// `tracing` layer writes to it.
pub(crate) struct ChannelSink {
    pub(crate) sink: Arc<dyn LogSink>,
    pub(crate) level: Option<LogLevel>,
    pub(crate) replace: bool,
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

/// Set once a default channel has been chosen, by the boot's
/// `check_channels`, an install, `Log::set_default_channel`, or the first
/// event that found none chosen and took `LOG_CHANNEL`.
static DEFAULT_CHOSEN: AtomicBool = AtomicBool::new(false);

/// Set by the first event that takes `LOG_CHANNEL` as the default, so that
/// neither another thread nor an event raised while resolving it starts a
/// second resolution.
static DEFAULT_TAKING: AtomicBool = AtomicBool::new(false);

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
const BUILT_IN: &str =
    "stdout, stderr, errorlog, single, daily, monthly, syslog, null, stack, custom";

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
    // The file and stream channels keep what `LOG_LEVEL`'s bare level keeps,
    // as Laravel's take `env('LOG_LEVEL', 'debug')`. `stdout`, the default
    // channel unless `LOG_CHANNEL` names another, keeps every level the
    // `tracing` filter lets through, so a target directive more verbose
    // than the bare level still reaches it.
    let level = super::config::channel_level;
    Ok(Some(match name {
        "stdout" => LogChannel::stdout(),
        "stderr" | "errorlog" => LogChannel::stderr().level(level()?),
        "single" => LogChannel::single(file()).level(level()?),
        "daily" => {
            let days = match var("LOG_DAILY_DAYS") {
                Some(days) => days.parse::<u32>().map_err(|_| {
                    FrameworkError::internal(format!(
                        "LOG_DAILY_DAYS is '{days}', and must be a number of days (0 keeps \
                         every file)"
                    ))
                })?,
                None => 7,
            };
            LogChannel::daily(file()).days(days).level(level()?)
        }
        "monthly" => LogChannel::monthly(file()).level(level()?),
        "syslog" => {
            let mut channel = LogChannel::syslog().level(level()?);
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
        "custom" => LogChannel::driver(var("LOG_CHANNEL_DRIVER").ok_or_else(|| {
            FrameworkError::internal(
                "the custom log channel needs LOG_CHANNEL_DRIVER to name a driver added with Log::extend",
            )
        })?),
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
        Leaf::new(LeafTo::Sink(sink), channel)
    };
    Ok(match &channel.kind {
        ChannelKind::Stdout => vec![Leaf::new(LeafTo::Stdout, channel)],
        ChannelKind::Stderr => vec![Leaf::new(LeafTo::Stderr, channel)],
        ChannelKind::Single(path) => vec![file(path, Rotation::None)],
        ChannelKind::Daily { path, days } => vec![file(path, Rotation::Daily(*days))],
        ChannelKind::Monthly { path, months } => vec![file(path, Rotation::Monthly(*months))],
        ChannelKind::Null => Vec::new(),
        ChannelKind::Syslog { facility, socket } => {
            let facility = facility_number(facility.as_deref().unwrap_or("user"))?;
            vec![Leaf::new(
                LeafTo::Sink(syslog(facility, socket.clone())?),
                channel,
            )]
        }
        ChannelKind::Stack(names) => {
            if depth > 8 {
                return Err(FrameworkError::internal(
                    "log stacks nest more than eight deep; a stack probably lists itself",
                ));
            }
            // The stack's own level applies on top of each channel's, so a
            // stack at `Warning` drops info even in a channel that keeps
            // every level. Each channel keeps its own placeholder form.
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
            vec![Leaf::new(LeafTo::Sink(sink), channel)]
        }
    })
}

#[cfg(unix)]
fn syslog(
    facility: u8,
    socket: Option<std::path::PathBuf>,
) -> Result<Arc<dyn LogSink>, FrameworkError> {
    use super::sinks::SyslogSink;
    let socket = socket.unwrap_or_else(SyslogSink::default_socket);
    let ident = std::env::var("APP_NAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "suprnova".to_owned());
    let sink = SyslogSink::new(socket, facility, ident).map_err(|error| {
        FrameworkError::internal(format!("cannot open a syslog socket: {error}"))
    })?;
    Ok(Arc::new(sink))
}

#[cfg(not(unix))]
fn syslog(
    _facility: u8,
    _socket: Option<std::path::PathBuf>,
) -> Result<Arc<dyn LogSink>, FrameworkError> {
    Err(FrameworkError::internal(
        "the syslog log channel needs a Unix system with a syslog socket",
    ))
}

fn level_code(level: Option<LogLevel>) -> u8 {
    level.map_or(8, LogLevel::severity)
}

/// The lowest level any of the standard-stream leaves keeps, as the
/// `tracing` formatter's threshold for that stream: `None` when no leaf is
/// that stream, `Some(None)` when one keeps every level.
///
/// `tracing` writes an event to a stream once however many channels in the
/// default stack name it, so the event goes when any of them keeps its
/// level, whichever comes first in the stack.
fn stream_minimum(
    leaves: &[Leaf],
    of_stream: impl Fn(&Leaf) -> Option<Option<LogLevel>>,
) -> Option<Option<LogLevel>> {
    leaves
        .iter()
        .filter_map(of_stream)
        .reduce(|kept, level| match (kept, level) {
            (Some(kept), Some(level)) => Some(kept.max(level)),
            _ => None,
        })
}

/// Make `name` the default channel: the one `tracing` events go to.
pub(crate) fn set_default(name: &str) -> Result<(), FrameworkError> {
    let leaves = resolve_named(name, 0)?;
    install_default(&mut write(), name, leaves);
    Ok(())
}

/// Whether `name` is a channel that can be the default, without making it so.
pub(crate) fn resolves(name: &str) -> Result<(), FrameworkError> {
    resolve_named(name, 0).map(|_| ())
}

fn install_default(registry: &mut Registry, name: &str, leaves: Vec<Leaf>) {
    let stdout = stream_minimum(&leaves, |leaf| match leaf.to {
        LeafTo::Stdout => Some(leaf.level),
        _ => None,
    });
    let stderr = stream_minimum(&leaves, |leaf| match leaf.to {
        LeafTo::Stderr => Some(leaf.level),
        _ => None,
    });
    STDOUT_ON.store(stdout.is_some(), Ordering::Relaxed);
    STDOUT_MIN.store(level_code(stdout.flatten()), Ordering::Relaxed);
    STDERR_ON.store(stderr.is_some(), Ordering::Relaxed);
    STDERR_MIN.store(level_code(stderr.flatten()), Ordering::Relaxed);
    registry.default = Some(name.to_owned());
    registry.default_leaves = leaves;
    DEFAULT_CHOSEN.store(true, Ordering::Release);
}

/// Make `LOG_CHANNEL` the default channel, or stdout when it does not
/// resolve, if no default has been chosen yet. [`Log::default_channel`]
/// already reports `LOG_CHANNEL` until one is, so a subscriber installed
/// without the boot sends its events where that says, while building the
/// subscriber itself chooses nothing.
fn take_default_if_unchosen() {
    if DEFAULT_CHOSEN.load(Ordering::Acquire) || DEFAULT_TAKING.swap(true, Ordering::AcqRel) {
        return;
    }
    let configured = configured_default();
    let taken = match resolve_named(&configured, 0) {
        Ok(leaves) => Some((configured, leaves)),
        Err(_) => resolve_named("stdout", 0)
            .ok()
            .map(|leaves| ("stdout".to_owned(), leaves)),
    };
    if let Some((name, leaves)) = taken {
        let mut registry = write();
        // A default chosen while this one resolved stands.
        if registry.default.is_none() {
            install_default(&mut registry, &name, leaves);
        }
    }
}

/// The sinks of the default channel that are not the standard streams.
pub(crate) fn default_sinks() -> Vec<ChannelSink> {
    take_default_if_unchosen();
    if DEFAULT_FORGOTTEN.swap(false, Ordering::Relaxed) {
        refresh_default();
    }
    read()
        .default_leaves
        .iter()
        .filter_map(|leaf| match &leaf.to {
            LeafTo::Sink(sink) => Some(ChannelSink {
                sink: Arc::clone(sink),
                level: leaf.level,
                replace: leaf.replace,
            }),
            _ => None,
        })
        .collect()
}

/// Whether a record at `level` goes to standard output (or, with `stderr`,
/// standard error) as part of the default channel.
pub(crate) fn stream_takes(stderr: bool, level: LogLevel) -> bool {
    take_default_if_unchosen();
    let (on, minimum) = if stderr {
        (&STDERR_ON, &STDERR_MIN)
    } else {
        (&STDOUT_ON, &STDOUT_MIN)
    };
    on.load(Ordering::Relaxed) && level.severity() <= minimum.load(Ordering::Relaxed)
}

/// Check the log variables that are set, whether or not the default
/// channel uses them: `LOG_LEVEL`'s bare level is a level, every name
/// `LOG_STACK` lists is a channel, `LOG_SYSLOG_FACILITY` a facility, and
/// `LOG_DAILY_DAYS` a number.
pub(crate) fn validate_environment() -> Result<(), FrameworkError> {
    super::config::channel_level()?;
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
    /// [`LogChannel::driver`] and this name, or the built-in `custom` channel
    /// when `LOG_CHANNEL_DRIVER` names it. The way to add Slack, a log
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
        Ok(Logger::on(resolve_named(name, 0)?))
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
        Ok(Logger::on(leaves))
    }

    /// A logger for a channel that has no name, built now.
    ///
    /// # Errors
    ///
    /// When its sink cannot be built: a syslog facility that is not one, a
    /// stack that lists an unknown channel, a driver that does not exist.
    pub fn build(channel: LogChannel) -> Result<Logger, FrameworkError> {
        Ok(Logger::on(build(&channel, 0)?))
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
                .filter_map(|leaf| match leaf.to {
                    LeafTo::Sink(sink) => Some(sink),
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

    /// Write `message` at `level` to the default channel, with `context`: a
    /// JSON object whose values fill the message's `{key}` placeholders and
    /// are written beside it. Laravel's `Log::log`.
    ///
    /// The write is a `tracing` event, so `LOG_LEVEL`'s filter and the
    /// request's span see it as they see `tracing`'s macros, and it carries
    /// its PSR-3 level in a field: a file or stack channel writes
    /// `CRITICAL` for [`critical`](Self::critical), where `tracing` knows
    /// only `ERROR`. Standard output, written by `tracing`'s own formatter,
    /// shows the event's `tracing` level with the PSR-3 level and the
    /// context as fields. With no subscriber installed the write goes
    /// nowhere, as a `tracing` macro's does.
    pub fn log(level: LogLevel, message: &str, context: Value) {
        let mut merged = shared_context();
        merge_into(&mut merged, context);
        let context = (!merged.is_empty()).then(|| Value::Object(merged).to_string());
        emit(level, message, context.as_deref());
    }

    /// `emergency` on the default channel: the system is unusable.
    pub fn emergency(message: &str) {
        Self::log(LogLevel::Emergency, message, Value::Null);
    }

    /// `alert` on the default channel: action must be taken at once.
    pub fn alert(message: &str) {
        Self::log(LogLevel::Alert, message, Value::Null);
    }

    /// `critical` on the default channel: a critical condition.
    pub fn critical(message: &str) {
        Self::log(LogLevel::Critical, message, Value::Null);
    }

    /// `error` on the default channel.
    pub fn error(message: &str) {
        Self::log(LogLevel::Error, message, Value::Null);
    }

    /// `warning` on the default channel.
    pub fn warning(message: &str) {
        Self::log(LogLevel::Warning, message, Value::Null);
    }

    /// `notice` on the default channel: normal but significant.
    pub fn notice(message: &str) {
        Self::log(LogLevel::Notice, message, Value::Null);
    }

    /// `info` on the default channel.
    pub fn info(message: &str) {
        Self::log(LogLevel::Info, message, Value::Null);
    }

    /// `debug` on the default channel.
    pub fn debug(message: &str) {
        Self::log(LogLevel::Debug, message, Value::Null);
    }

    /// `emergency` on the default channel, with a context.
    pub fn emergency_with(message: &str, context: Value) {
        Self::log(LogLevel::Emergency, message, context);
    }

    /// `alert` on the default channel, with a context.
    pub fn alert_with(message: &str, context: Value) {
        Self::log(LogLevel::Alert, message, context);
    }

    /// `critical` on the default channel, with a context.
    pub fn critical_with(message: &str, context: Value) {
        Self::log(LogLevel::Critical, message, context);
    }

    /// `error` on the default channel, with a context.
    pub fn error_with(message: &str, context: Value) {
        Self::log(LogLevel::Error, message, context);
    }

    /// `warning` on the default channel, with a context.
    pub fn warning_with(message: &str, context: Value) {
        Self::log(LogLevel::Warning, message, context);
    }

    /// `notice` on the default channel, with a context.
    pub fn notice_with(message: &str, context: Value) {
        Self::log(LogLevel::Notice, message, context);
    }

    /// `info` on the default channel, with a context.
    pub fn info_with(message: &str, context: Value) {
        Self::log(LogLevel::Info, message, context);
    }

    /// `debug` on the default channel, with a context.
    pub fn debug_with(message: &str, context: Value) {
        Self::log(LogLevel::Debug, message, context);
    }

    /// Add `context`, a JSON object, to every later write of the current
    /// [`Context`](crate::context::Context) scope, on every channel, under
    /// a [`Logger`]'s context and each call's own. Laravel's `shareContext`.
    ///
    /// The scope is the one each request and each queued job runs in, so
    /// one request's context never reaches the lines of another running at
    /// the same time. Outside a scope it shares nothing, as
    /// [`Context::add`](crate::context::Context::add) does. A key shared
    /// again takes the new value. Shared context stays with its scope: a job
    /// the request dispatches does not carry it.
    ///
    /// ```rust,no_run
    /// use serde_json::json;
    /// use suprnova::Log;
    ///
    /// // In a middleware, once the user is known.
    /// Log::share_context(json!({ "user_id": 42 }));
    /// // Every later line of this request carries user_id.
    /// Log::info("order placed");
    /// ```
    pub fn share_context(context: Value) {
        let mut shared = shared_context();
        merge_into(&mut shared, context);
        store_shared_context(shared);
    }

    /// The context the current scope shares, empty outside a scope.
    pub fn shared_context() -> Map<String, Value> {
        shared_context()
    }

    /// Stop sharing `keys`, or every key for `None`, in the current scope.
    pub fn without_context(keys: Option<&[&str]>) {
        let shared = match keys {
            Some(keys) => {
                let mut shared = shared_context();
                for key in keys {
                    shared.remove(*key);
                }
                shared
            }
            None => Map::new(),
        };
        store_shared_context(shared);
    }

    /// Stop sharing any context in the current scope. Laravel's
    /// `flushSharedContext`.
    pub fn flush_shared_context() {
        Self::without_context(None);
    }

    /// Call `callback` for each write that reaches a channel, with the
    /// write's level, message and context, as Laravel's `Log::listen`
    /// registers for `MessageLogged`.
    ///
    /// The callback runs on the thread that writes, before the write
    /// returns, so it should be quick. A callback that panics does not fail
    /// the write, and a write the callback makes itself is not reported to
    /// it again. Every write is also dispatched as a
    /// [`MessageLogged`] event; see there for how.
    pub fn listen(callback: impl Fn(&MessageLogged) + Send + Sync + 'static) {
        events::listen(Arc::new(callback));
    }
}

/// The hidden context key the shared log context lives under. Keeping it in
/// the current [`Context`](crate::context::Context) scope is what makes it
/// per request and per job; the hidden bag keeps it out of
/// `Context::all()`.
const SHARED_CONTEXT_KEY: &str = "suprnova.log.shared_context";

/// The context the current scope shares.
///
/// Read as any JSON value, which always deserializes: a typed read that
/// failed would log, and this runs inside the `tracing` layer, where that
/// line would come straight back here.
fn shared_context() -> Map<String, Value> {
    match crate::context::Context::hidden_get::<Value>(SHARED_CONTEXT_KEY) {
        Some(Value::Object(shared)) => shared,
        _ => Map::new(),
    }
}

/// Replace the context the current scope shares. Outside a scope the
/// context discards it.
fn store_shared_context(shared: Map<String, Value>) {
    // Shared log context belongs to the scope it was shared in, as Laravel's
    // belongs to its process: a job the scope dispatches starts without it.
    static NOT_CARRIED: std::sync::Once = std::sync::Once::new();
    NOT_CARRIED.call_once(|| {
        crate::context::Context::dehydrating(|snapshot| {
            snapshot.hidden.remove(SHARED_CONTEXT_KEY);
        });
    });
    crate::context::Context::hidden_add(SHARED_CONTEXT_KEY, Value::Object(shared));
}

/// Merge `context` into `into`, its keys winning: an object's entries, or a
/// value of another kind under `context`.
fn merge_into(into: &mut Map<String, Value>, context: Value) {
    match context {
        Value::Object(map) => into.extend(map),
        Value::Null => {}
        other => {
            into.insert("context".to_owned(), other);
        }
    }
}

/// A context as the record's text pairs: a string as it is, any other value
/// as JSON.
fn as_pairs(context: &Map<String, Value>) -> Vec<(String, String)> {
    context
        .iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (key.clone(), text)
        })
        .collect()
}

/// The `tracing` field a write through [`Log`] carries its PSR-3 level in.
pub(crate) const LEVEL_FIELD: &str = "suprnova.level";

/// The `tracing` field a write through [`Log`] carries its context in, as a
/// JSON object.
pub(crate) const CONTEXT_FIELD: &str = "suprnova.context";

/// One `tracing` event at `level`'s nearest `tracing` level, with the PSR-3
/// level and the context as fields. `tracing` needs each level at a callsite
/// of its own.
fn emit(level: LogLevel, message: &str, context: Option<&str>) {
    macro_rules! at {
        ($tracing:expr) => {
            match context {
                Some(context) => tracing::event!(
                    $tracing,
                    suprnova.level = level.psr_name(),
                    suprnova.context = context,
                    "{}",
                    message
                ),
                None => tracing::event!($tracing, suprnova.level = level.psr_name(), "{}", message),
            }
        };
    }
    match level {
        LogLevel::Emergency | LogLevel::Alert | LogLevel::Critical | LogLevel::Error => {
            at!(tracing::Level::ERROR)
        }
        LogLevel::Warning => at!(tracing::Level::WARN),
        LogLevel::Notice | LogLevel::Info => at!(tracing::Level::INFO),
        LogLevel::Debug => at!(tracing::Level::DEBUG),
    }
}

/// A logger for one channel, a stack, or a built channel, from [`Log`].
#[derive(Clone)]
pub struct Logger {
    leaves: Vec<Leaf>,
    context: Map<String, Value>,
}

impl Logger {
    fn on(leaves: Vec<Leaf>) -> Self {
        Self {
            leaves,
            context: Map::new(),
        }
    }

    /// A logger whose later writes carry `context`, a JSON object, under
    /// each call's own: a key the call names takes the call's value. The
    /// logger it came from is unchanged. Laravel's `Logger::withContext`.
    pub fn with_context(&self, context: Value) -> Logger {
        let mut logger = self.clone();
        merge_into(&mut logger.context, context);
        logger
    }

    /// A logger without the context `keys` name, or without any for `None`.
    /// Laravel's `Logger::withoutContext`.
    pub fn without_context(&self, keys: Option<&[&str]>) -> Logger {
        let mut logger = self.clone();
        match keys {
            Some(keys) => {
                for key in keys {
                    logger.context.remove(*key);
                }
            }
            None => logger.context.clear(),
        }
        logger
    }

    /// Write `message` at `level`, with `context`: a JSON object whose
    /// values fill the message's `{key}` placeholders, on each channel that
    /// replaces them, and are written beside it.
    ///
    /// The record carries the scope's shared context
    /// ([`Log::share_context`]), then the logger's
    /// ([`with_context`](Self::with_context)), then `context`, a later key
    /// winning. The write is reported to [`Log::listen`] and dispatched as
    /// [`MessageLogged`], once, whichever channels keep it.
    pub fn log(&self, level: LogLevel, message: &str, context: Value) {
        let mut merged = shared_context();
        merged.extend(self.context.clone());
        merge_into(&mut merged, context);
        let raw = LogRecord {
            time: crate::clock::now(),
            level,
            target: String::new(),
            message: message.to_owned(),
            context: as_pairs(&merged),
        };
        let mut replaced: Option<LogRecord> = None;
        for leaf in &self.leaves {
            if !level.passes(leaf.level) {
                continue;
            }
            let record = if leaf.replace {
                &*replaced.get_or_insert_with(|| LogRecord {
                    message: replace_placeholders(&raw.message, &raw.context),
                    ..raw.clone()
                })
            } else {
                &raw
            };
            // Every sink reports its own failure once on stderr, a driver's
            // through its `ReportedSink`, so the result is not needed here:
            // logging never fails its caller.
            let _ = match &leaf.to {
                LeafTo::Stdout => StreamSink::stdout().write(record),
                LeafTo::Stderr => StreamSink::stderr().write(record),
                LeafTo::Sink(sink) => sink.write(record),
            };
        }
        if events::observed() {
            events::report(MessageLogged::new(level, message.to_owned(), merged));
        }
    }

    /// `emergency`: the system is unusable.
    pub fn emergency(&self, message: &str) {
        self.log(LogLevel::Emergency, message, Value::Null);
    }

    /// `alert`: action must be taken at once.
    pub fn alert(&self, message: &str) {
        self.log(LogLevel::Alert, message, Value::Null);
    }

    /// `critical`: a critical condition.
    pub fn critical(&self, message: &str) {
        self.log(LogLevel::Critical, message, Value::Null);
    }

    /// `error`.
    pub fn error(&self, message: &str) {
        self.log(LogLevel::Error, message, Value::Null);
    }

    /// `warning`.
    pub fn warning(&self, message: &str) {
        self.log(LogLevel::Warning, message, Value::Null);
    }

    /// `notice`: normal but significant.
    pub fn notice(&self, message: &str) {
        self.log(LogLevel::Notice, message, Value::Null);
    }

    /// `info`.
    pub fn info(&self, message: &str) {
        self.log(LogLevel::Info, message, Value::Null);
    }

    /// `debug`.
    pub fn debug(&self, message: &str) {
        self.log(LogLevel::Debug, message, Value::Null);
    }

    /// `emergency` with a context.
    pub fn emergency_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Emergency, message, context);
    }

    /// `alert` with a context.
    pub fn alert_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Alert, message, context);
    }

    /// `critical` with a context.
    pub fn critical_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Critical, message, context);
    }

    /// `error` with a context.
    pub fn error_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Error, message, context);
    }

    /// `warning` with a context.
    pub fn warning_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Warning, message, context);
    }

    /// `notice` with a context.
    pub fn notice_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Notice, message, context);
    }

    /// `info` with a context.
    pub fn info_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Info, message, context);
    }

    /// `debug` with a context.
    pub fn debug_with(&self, message: &str, context: Value) {
        self.log(LogLevel::Debug, message, context);
    }
}

/// The context the current scope shares, for the `tracing` layer, which
/// adds it to every event's record.
pub(crate) fn scope_shared_context() -> Map<String, Value> {
    shared_context()
}
