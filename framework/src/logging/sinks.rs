//! The built-in sinks: files that rotate, the standard streams, syslog;
//! the line they write; and the flushing of buffered files.

use super::channel::{LogLevel, LogRecord, LogSink};
use super::config::LogFormat;
use chrono::{DateTime, NaiveDate, Utc};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::Duration;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The format of the lines the sinks write for [`Log`](super::Log):
/// `LOG_FORMAT`, set when the framework installs its subscriber and read
/// from the environment before that.
static FORMAT: RwLock<Option<LogFormat>> = RwLock::new(None);

thread_local! {
    /// The format of the subscriber whose event this thread is writing, so
    /// its lines match its own standard-stream lines. Each subscriber
    /// carries its format rather than setting `FORMAT`, so building one
    /// changes nothing for the subscriber already installed.
    static WRITING_FOR: std::cell::Cell<Option<LogFormat>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn set_format(format: LogFormat) {
    *FORMAT
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(format);
}

/// Run `write` with the lines it writes in `format`, the format of the
/// subscriber the event came through.
pub(crate) fn write_in<R>(format: LogFormat, write: impl FnOnce() -> R) -> R {
    /// Puts back the format of an outer write, however `write` ends.
    struct Restore(Option<LogFormat>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WRITING_FOR.with(|cell| cell.set(self.0));
        }
    }
    let _restore = Restore(WRITING_FOR.with(|cell| cell.replace(Some(format))));
    write()
}

fn format() -> LogFormat {
    if let Some(format) = WRITING_FOR.with(std::cell::Cell::get) {
        return format;
    }
    FORMAT
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .unwrap_or_else(|| super::config::LogConfig::from_env().format)
}

/// Replace each `{key}` in `message` with the context's value under `key`,
/// for any key the context holds, as Laravel's `replace_placeholders`
/// does: in one pass, so a value that holds a placeholder is not replaced
/// again. A placeholder with no value in the context is left as it is.
pub(crate) fn replace_placeholders(message: &str, context: &[(String, String)]) -> String {
    if !message.contains('{') || context.is_empty() {
        return message.to_owned();
    }
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let found = after.find('}').and_then(|close| {
            let key = &after[..close];
            context
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| (value, close))
        });
        match found {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One record as a line: text for `pretty`, a JSON object for `json`.
pub(crate) fn line(record: &LogRecord) -> String {
    match format() {
        LogFormat::Json => {
            let context: serde_json::Map<String, serde_json::Value> = record
                .context
                .iter()
                .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                .collect();
            let mut value = serde_json::json!({
                "time": record.time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "level": record.level.name(),
                "message": record.message,
                "context": context,
            });
            if !record.target.is_empty() {
                value["target"] = serde_json::Value::String(record.target.clone());
            }
            format!("{value}\n")
        }
        LogFormat::Pretty => {
            let mut text = format!(
                "[{}] {}",
                record.time.format("%Y-%m-%d %H:%M:%S%.3f UTC"),
                record.level.name()
            );
            if !record.target.is_empty() {
                text.push(' ');
                text.push_str(&record.target);
            }
            text.push_str(": ");
            text.push_str(&record.message);
            if !record.context.is_empty() {
                let context: serde_json::Map<String, serde_json::Value> = record
                    .context
                    .iter()
                    .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                    .collect();
                text.push(' ');
                text.push_str(&serde_json::Value::Object(context).to_string());
            }
            text.push('\n');
            text
        }
    }
}

/// Report a sink that cannot write, once, on stderr: a log call never
/// fails its caller, and a log about the log could loop.
fn report_once(reported: &AtomicBool, what: &str, error: &std::io::Error) {
    if !reported.swap(true, Ordering::Relaxed) {
        eprintln!("suprnova: a log channel cannot write {what}: {error}");
    }
}

/// A sink an application's driver built, with its failures reported.
///
/// [`LogSink`] leaves reporting a failed write to its caller, so a driver
/// is not expected to report its own. Every write and flush of a driver's
/// sink goes through this, the logger and the flusher alike, and the
/// first failure is reported once on stderr, as the built-in sinks report
/// theirs.
pub(crate) struct ReportedSink {
    inner: Arc<dyn LogSink>,
    driver: String,
    reported: AtomicBool,
}

impl ReportedSink {
    pub(crate) fn new(inner: Arc<dyn LogSink>, driver: &str) -> Self {
        Self {
            inner,
            driver: driver.to_owned(),
            reported: AtomicBool::new(false),
        }
    }

    fn reported(&self, outcome: std::io::Result<()>) -> std::io::Result<()> {
        if let Err(error) = &outcome {
            report_once(
                &self.reported,
                &format!("through the '{}' driver", self.driver),
                error,
            );
        }
        outcome
    }
}

impl LogSink for ReportedSink {
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        self.reported(self.inner.write(record))
    }

    fn flush(&self) -> std::io::Result<()> {
        self.reported(self.inner.flush())
    }
}

// Flushing.

/// Every file sink alive, for [`flush_all`] and the flusher thread.
static FLUSHABLE: Mutex<Vec<Weak<dyn LogSink>>> = Mutex::new(Vec::new());

/// How often the flusher writes out buffered records.
const FLUSH_EVERY: Duration = Duration::from_millis(500);

/// Register a sink whose records are buffered, starting the flusher thread
/// the first time.
pub(crate) fn register_flushable(sink: &Arc<dyn LogSink>) {
    lock(&FLUSHABLE).push(Arc::downgrade(sink));
    static FLUSHER: OnceLock<()> = OnceLock::new();
    FLUSHER.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("suprnova-log-flush".into())
            .spawn(|| {
                loop {
                    std::thread::sleep(FLUSH_EVERY);
                    flush_all();
                }
            });
        if let Err(error) = spawned {
            eprintln!(
                "suprnova: could not start the log flusher ({error}); file channels flush \
                 on errors, Log::flush and shutdown only"
            );
        }
    });
}

/// Write out every buffered file channel.
pub(crate) fn flush_all() {
    let sinks: Vec<Arc<dyn LogSink>> = {
        let mut flushable = lock(&FLUSHABLE);
        flushable.retain(|sink| sink.strong_count() > 0);
        flushable.iter().filter_map(Weak::upgrade).collect()
    };
    for sink in sinks {
        let _ = sink.flush();
    }
}

// Files.

/// How a file sink splits its records into files.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rotation {
    /// One file.
    None,
    /// A file a day, keeping this many.
    Daily(u32),
    /// A file a month, keeping this many.
    Monthly(u32),
}

/// A file, or a file a day or a month, buffered.
pub(crate) struct FileSink {
    base: PathBuf,
    rotation: Rotation,
    state: Mutex<FileState>,
    reported: AtomicBool,
}

#[derive(Default)]
struct FileState {
    path: Option<PathBuf>,
    writer: Option<BufWriter<File>>,
}

impl FileSink {
    pub(crate) fn new(base: PathBuf, rotation: Rotation) -> Self {
        Self {
            base,
            rotation,
            state: Mutex::new(FileState::default()),
            reported: AtomicBool::new(false),
        }
    }

    fn stem_and_extension(&self) -> (String, Option<String>) {
        let stem = self
            .base
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "suprnova".to_owned());
        let extension = self
            .base
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned());
        (stem, extension)
    }

    /// The file a record written at `time` goes to.
    fn path_at(&self, time: DateTime<Utc>) -> PathBuf {
        let period = match self.rotation {
            Rotation::None => return self.base.clone(),
            Rotation::Daily(_) => time.format("%Y-%m-%d").to_string(),
            Rotation::Monthly(_) => time.format("%Y-%m").to_string(),
        };
        let (stem, extension) = self.stem_and_extension();
        let name = match extension {
            Some(extension) => format!("{stem}-{period}.{extension}"),
            None => format!("{stem}-{period}"),
        };
        self.base.with_file_name(name)
    }

    /// Delete the dated files beyond the number kept, newest kept first. A
    /// count of 0 keeps every file, as Monolog's `max_files` of 0 does.
    fn prune(&self) {
        let (keep, monthly) = match self.rotation {
            Rotation::None => return,
            Rotation::Daily(keep) => (keep, false),
            Rotation::Monthly(keep) => (keep, true),
        };
        if keep == 0 {
            return;
        }
        let Some(dir) = self.base.parent() else {
            return;
        };
        let (stem, extension) = self.stem_and_extension();
        let prefix = format!("{stem}-");
        let suffix = extension.map(|extension| format!(".{extension}"));
        let Ok(entries) = std::fs::read_dir(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        }) else {
            return;
        };
        let mut dated: Vec<(NaiveDate, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let middle = name.strip_prefix(&prefix)?;
                let middle = match &suffix {
                    Some(suffix) => middle.strip_suffix(suffix.as_str())?,
                    None => middle,
                };
                let date = if monthly {
                    NaiveDate::parse_from_str(&format!("{middle}-01"), "%Y-%m-%d").ok()?
                } else {
                    NaiveDate::parse_from_str(middle, "%Y-%m-%d").ok()?
                };
                let shape_matches = if monthly {
                    middle.len() == 7
                } else {
                    middle.len() == 10
                };
                shape_matches.then(|| (date, entry.path()))
            })
            .collect();
        dated.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
        for (_, path) in dated.into_iter().skip(keep as usize) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The writer for `path`, opening it when the record goes to another
    /// file than the last one did.
    fn writer_for<'a>(
        &self,
        state: &'a mut FileState,
        path: PathBuf,
    ) -> std::io::Result<&'a mut BufWriter<File>> {
        if state.path.as_deref() != Some(path.as_path()) || state.writer.is_none() {
            if let Some(mut old) = state.writer.take()
                && let Err(error) = old.flush()
            {
                // The records still buffered for the previous file are lost.
                report_once(&self.reported, &self.base.display().to_string(), &error);
            }
            if let Some(dir) = path.parent()
                && !dir.as_os_str().is_empty()
            {
                std::fs::create_dir_all(dir)?;
            }
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            state.writer = Some(BufWriter::new(file));
            state.path = Some(path);
            self.prune();
        }
        state
            .writer
            .as_mut()
            .ok_or_else(|| std::io::Error::other("no open log file"))
    }
}

impl LogSink for FileSink {
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        let path = self.path_at(record.time);
        let mut state = lock(&self.state);
        let written = self.writer_for(&mut state, path).and_then(|writer| {
            writer.write_all(line(record).as_bytes())?;
            if record.level <= LogLevel::Error {
                writer.flush()?;
            }
            Ok(())
        });
        if let Err(error) = &written {
            report_once(&self.reported, &self.base.display().to_string(), error);
        }
        written
    }

    /// Reports a failure itself, as `write` does: the flusher thread and
    /// `Log::flush` have no caller to tell, and the records left in the
    /// buffer are the ones a failed flush loses.
    fn flush(&self) -> std::io::Result<()> {
        let flushed = match lock(&self.state).writer.as_mut() {
            Some(writer) => writer.flush(),
            None => Ok(()),
        };
        if let Err(error) = &flushed {
            report_once(&self.reported, &self.base.display().to_string(), error);
        }
        flushed
    }
}

// The standard streams, for the records written through `Log`.

/// Standard output or standard error, written a line at a time.
pub(crate) struct StreamSink {
    stderr: bool,
}

impl StreamSink {
    pub(crate) fn stdout() -> Self {
        Self { stderr: false }
    }

    pub(crate) fn stderr() -> Self {
        Self { stderr: true }
    }
}

/// Set once a record written through `Log` could not reach standard
/// output, so the failure is reported once.
static STDOUT_REPORTED: AtomicBool = AtomicBool::new(false);

impl LogSink for StreamSink {
    /// A failure on standard output is reported once on standard error. One
    /// on standard error has nowhere left to be reported.
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        let text = line(record);
        if self.stderr {
            std::io::stderr().lock().write_all(text.as_bytes())
        } else {
            let mut out = std::io::stdout().lock();
            let written = out.write_all(text.as_bytes()).and_then(|()| out.flush());
            if let Err(error) = &written {
                report_once(&STDOUT_REPORTED, "to standard output", error);
            }
            written
        }
    }
}

// Syslog.

/// The local syslog, an RFC 3164 datagram a record.
#[cfg(unix)]
pub(crate) struct SyslogSink {
    socket: PathBuf,
    facility: u8,
    connection: std::os::unix::net::UnixDatagram,
    reported: AtomicBool,
}

#[cfg(unix)]
impl SyslogSink {
    pub(crate) fn new(socket: PathBuf, facility: u8) -> std::io::Result<Self> {
        Ok(Self {
            socket,
            facility,
            connection: std::os::unix::net::UnixDatagram::unbound()?,
            reported: AtomicBool::new(false),
        })
    }

    /// The socket syslog listens on here.
    pub(crate) fn default_socket() -> PathBuf {
        if cfg!(target_os = "macos") {
            PathBuf::from("/var/run/syslog")
        } else {
            PathBuf::from("/dev/log")
        }
    }
}

#[cfg(unix)]
impl LogSink for SyslogSink {
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        let priority = u16::from(self.facility) * 8 + u16::from(record.level.severity());
        let mut message = record.message.clone();
        if !record.context.is_empty() {
            let context: serde_json::Map<String, serde_json::Value> = record
                .context
                .iter()
                .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                .collect();
            message.push(' ');
            message.push_str(&serde_json::Value::Object(context).to_string());
        }
        let datagram = format!(
            "<{priority}>{} suprnova[{}]: {message}",
            record.time.format("%b %e %H:%M:%S"),
            std::process::id()
        );
        let sent = self
            .connection
            .send_to(datagram.as_bytes(), &self.socket)
            .map(|_| ());
        if let Err(error) = &sent {
            report_once(
                &self.reported,
                &format!("to syslog at {}", self.socket.display()),
                error,
            );
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        context(pairs)
    }

    fn context(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn placeholders_take_the_context_and_leave_the_rest() {
        let context = context(&[("id", "7"), ("user.name", "ada")]);
        assert_eq!(
            replace_placeholders("user {id} {user.name} {nope} {} {id", &context),
            "user 7 ada {nope} {} {id"
        );
        let odd = context_of(&[("user-id", "9"), ("a b", "{id}"), ("id", "7")]);
        assert_eq!(
            replace_placeholders("{user-id} {a b} {id}", &odd),
            "9 {id} 7",
            "any key, and one pass"
        );
        assert_eq!(replace_placeholders("no braces", &context), "no braces");
    }

    #[test]
    fn dated_names_keep_the_extension() {
        let sink = FileSink::new(PathBuf::from("/tmp/logs/app.log"), Rotation::Daily(14));
        let time = DateTime::parse_from_rfc3339("2026-10-02T23:59:59Z")
            .unwrap()
            .to_utc();
        assert_eq!(
            sink.path_at(time),
            PathBuf::from("/tmp/logs/app-2026-10-02.log")
        );
        let monthly = FileSink::new(PathBuf::from("/tmp/logs/app"), Rotation::Monthly(3));
        assert_eq!(
            monthly.path_at(time),
            PathBuf::from("/tmp/logs/app-2026-10")
        );
    }
}
