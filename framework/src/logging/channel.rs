//! Log channels: where log records go, and the record a channel is given.

use crate::error::FrameworkError;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// The eight PSR-3 levels Laravel logs at, most severe first. `tracing`'s
/// five map onto them: `ERROR` to [`Error`](Self::Error), `WARN` to
/// [`Warning`](Self::Warning), `INFO` to [`Info`](Self::Info), and `DEBUG`
/// and `TRACE` to [`Debug`](Self::Debug).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LogLevel {
    /// The system is unusable.
    Emergency,
    /// Action must be taken at once.
    Alert,
    /// A critical condition.
    Critical,
    /// An error.
    Error,
    /// A warning.
    Warning,
    /// Normal but significant.
    Notice,
    /// Informational.
    Info,
    /// Detail for debugging.
    Debug,
}

impl LogLevel {
    /// The syslog severity: 0 for `Emergency` to 7 for `Debug`.
    pub fn severity(self) -> u8 {
        self as u8
    }

    /// The upper-case name Laravel writes: `EMERGENCY` to `DEBUG`.
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Emergency => "EMERGENCY",
            LogLevel::Alert => "ALERT",
            LogLevel::Critical => "CRITICAL",
            LogLevel::Error => "ERROR",
            LogLevel::Warning => "WARNING",
            LogLevel::Notice => "NOTICE",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        }
    }

    /// Whether a record at this level passes a channel whose lowest level
    /// is `minimum`.
    pub(crate) fn passes(self, minimum: Option<LogLevel>) -> bool {
        minimum.is_none_or(|minimum| self <= minimum)
    }
}

impl From<tracing::Level> for LogLevel {
    fn from(level: tracing::Level) -> Self {
        match level {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warning,
            tracing::Level::INFO => LogLevel::Info,
            _ => LogLevel::Debug,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One record, as a channel's sink receives it.
#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    /// When it was written, by the framework clock.
    pub time: DateTime<Utc>,
    /// Its level.
    pub level: LogLevel,
    /// The `tracing` target of an event; empty for a record written
    /// through [`Log`](super::Log).
    pub target: String,
    /// The message, with its `{key}` placeholders replaced from the context.
    pub message: String,
    /// The context: the event's fields, or the context given to the
    /// [`Logger`](super::Logger), each value as text.
    pub context: Vec<(String, String)>,
}

/// Where a channel writes. A driver added with [`Log::extend`](super::Log::extend)
/// returns one.
///
/// A write that fails is reported by the caller once and does not reach
/// the code that logged: logging never fails the request.
pub trait LogSink: Send + Sync {
    /// Write one record.
    ///
    /// # Errors
    ///
    /// When the record could not be written.
    fn write(&self, record: &LogRecord) -> std::io::Result<()>;

    /// Write out anything buffered. The default does nothing.
    ///
    /// # Errors
    ///
    /// When the buffered records could not be written.
    fn flush(&self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A channel's settings: what [`Log::define`](super::Log::define) names and
/// [`Log::build`](super::Log::build) builds.
#[derive(Debug, Clone)]
pub struct LogChannel {
    pub(crate) kind: ChannelKind,
    pub(crate) level: Option<LogLevel>,
    pub(crate) options: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) enum ChannelKind {
    Stdout,
    Stderr,
    Single(PathBuf),
    Daily {
        path: PathBuf,
        days: u32,
    },
    Monthly {
        path: PathBuf,
        months: u32,
    },
    Syslog {
        facility: Option<String>,
        socket: Option<PathBuf>,
    },
    Null,
    Stack(Vec<String>),
    Driver(String),
}

impl LogChannel {
    fn new(kind: ChannelKind) -> Self {
        Self {
            kind,
            level: None,
            options: BTreeMap::new(),
        }
    }

    /// Standard output, in the format `LOG_FORMAT` picks.
    pub fn stdout() -> Self {
        Self::new(ChannelKind::Stdout)
    }

    /// Standard error, in the format `LOG_FORMAT` picks.
    pub fn stderr() -> Self {
        Self::new(ChannelKind::Stderr)
    }

    /// One file, appended to. Its directories are created.
    pub fn single(path: impl AsRef<Path>) -> Self {
        Self::new(ChannelKind::Single(path.as_ref().to_path_buf()))
    }

    /// A file a day, named from `path` with the date before the extension
    /// (`app.log` becomes `app-2026-10-02.log`), keeping the newest 14
    /// unless [`days`](Self::days) says otherwise.
    pub fn daily(path: impl AsRef<Path>) -> Self {
        Self::new(ChannelKind::Daily {
            path: path.as_ref().to_path_buf(),
            days: 14,
        })
    }

    /// A file a month (`app-2026-10.log`), keeping the newest 3 unless
    /// [`months`](Self::months) says otherwise.
    pub fn monthly(path: impl AsRef<Path>) -> Self {
        Self::new(ChannelKind::Monthly {
            path: path.as_ref().to_path_buf(),
            months: 3,
        })
    }

    /// The local syslog, over its Unix socket.
    pub fn syslog() -> Self {
        Self::new(ChannelKind::Syslog {
            facility: None,
            socket: None,
        })
    }

    /// Nowhere.
    pub fn null() -> Self {
        Self::new(ChannelKind::Null)
    }

    /// Every channel named, each with its own settings.
    pub fn stack<I, S>(channels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(ChannelKind::Stack(
            channels.into_iter().map(Into::into).collect(),
        ))
    }

    /// A channel on a driver added with [`Log::extend`](super::Log::extend).
    pub fn driver(name: impl Into<String>) -> Self {
        Self::new(ChannelKind::Driver(name.into()))
    }

    /// Keep only records at `level` or more severe.
    pub fn level(mut self, level: LogLevel) -> Self {
        self.level = Some(level);
        self
    }

    /// How many daily files to keep, counting the one written to; 0 keeps
    /// every file, as Laravel's `max_files` of 0 does. Applies to a
    /// [`daily`](Self::daily) channel.
    pub fn days(mut self, keep: u32) -> Self {
        if let ChannelKind::Daily { days, .. } = &mut self.kind {
            *days = keep;
        }
        self
    }

    /// How many monthly files to keep, counting the one written to; 0 keeps
    /// every file. Applies to a [`monthly`](Self::monthly) channel.
    pub fn months(mut self, keep: u32) -> Self {
        if let ChannelKind::Monthly { months, .. } = &mut self.kind {
            *months = keep;
        }
        self
    }

    /// The syslog facility: a name (`user`, `daemon`, `local0` to `local7`
    /// and the rest) or its number. `user` unless set.
    pub fn facility(mut self, facility: &str) -> Self {
        if let ChannelKind::Syslog { facility: slot, .. } = &mut self.kind {
            *slot = Some(facility.to_owned());
        }
        self
    }

    /// The syslog socket. `/dev/log` unless set, or `/var/run/syslog` on
    /// macOS.
    pub fn socket(mut self, path: impl AsRef<Path>) -> Self {
        if let ChannelKind::Syslog { socket, .. } = &mut self.kind {
            *socket = Some(path.as_ref().to_path_buf());
        }
        self
    }

    /// A setting for a custom driver, which reads it with
    /// [`option_value`](Self::option_value).
    pub fn option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// A setting given with [`option`](Self::option).
    pub fn option_value(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(String::as_str)
    }

    /// The lowest level the channel keeps; `None` keeps every level.
    pub fn minimum_level(&self) -> Option<LogLevel> {
        self.level
    }
}

/// The syslog facility number for a name or a number.
pub(crate) fn facility_number(facility: &str) -> Result<u8, FrameworkError> {
    let named = match facility.trim().to_ascii_lowercase().as_str() {
        "kern" => Some(0),
        "user" => Some(1),
        "mail" => Some(2),
        "daemon" => Some(3),
        "auth" => Some(4),
        "syslog" => Some(5),
        "lpr" => Some(6),
        "news" => Some(7),
        "uucp" => Some(8),
        "cron" => Some(9),
        "authpriv" => Some(10),
        "ftp" => Some(11),
        "local0" => Some(16),
        "local1" => Some(17),
        "local2" => Some(18),
        "local3" => Some(19),
        "local4" => Some(20),
        "local5" => Some(21),
        "local6" => Some(22),
        "local7" => Some(23),
        other => other.parse::<u8>().ok().filter(|number| *number <= 23),
    };
    named.ok_or_else(|| {
        FrameworkError::internal(format!(
            "the syslog facility '{facility}' is not one: use user, daemon, local0 to local7, \
             another facility name, or a number from 0 to 23"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_order_from_most_severe() {
        assert!(LogLevel::Emergency < LogLevel::Debug);
        assert!(LogLevel::Error.passes(Some(LogLevel::Warning)));
        assert!(!LogLevel::Info.passes(Some(LogLevel::Warning)));
        assert!(LogLevel::Debug.passes(None));
        assert_eq!(LogLevel::Warning.severity(), 4);
    }

    #[test]
    fn facilities_by_name_and_number() {
        assert_eq!(facility_number("local3").unwrap(), 19);
        assert_eq!(facility_number("USER").unwrap(), 1);
        assert_eq!(facility_number("4").unwrap(), 4);
        assert!(facility_number("24").is_err());
        assert!(facility_number("nonsense").is_err());
    }
}
