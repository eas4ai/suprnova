use super::{LogReporter, NullReporter, Reporter};
use crate::{FrameworkError, LogLevel};
use std::sync::Arc;
use std::time::Duration;

/// Driver names are validated at boot so a typo cannot silently lose reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReportingDriver {
    /// The standard envelope protocol reaches Rustrak, Sentry and GlitchTip.
    #[default]
    Sentry,
    /// Keep reports in the application's existing log channels.
    Log,
    /// Disable reporting explicitly for an application or environment.
    Null,
}

impl ReportingDriver {
    /// Reject an unknown driver instead of silently choosing a fallback.
    pub fn parse(name: &str) -> Result<Self, FrameworkError> {
        match name.trim().to_ascii_lowercase().as_str() {
            "sentry" => Ok(Self::Sentry),
            "log" => Ok(Self::Log),
            "null" => Ok(Self::Null),
            _ => Err(FrameworkError::internal(
                "REPORTING_DRIVER must be sentry, log or null",
            )),
        }
    }
}

/// Programmatic settings let applications configure reporting without env mutation.
#[derive(Clone)]
pub struct ReportingConfig {
    pub(crate) driver: ReportingDriver,
    pub(crate) dsn: Option<String>,
    pub(crate) release: Option<String>,
    pub(crate) environment: String,
    pub(crate) send_pii: bool,
    pub(crate) logs: Option<LogLevel>,
    pub(crate) traces_sample_rate: f32,
    pub(crate) queue_capacity: usize,
    pub(crate) timeout: Duration,
    pub(crate) duplicate_window: Duration,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            driver: ReportingDriver::Sentry,
            dsn: None,
            release: None,
            environment: std::env::var("APP_ENV").unwrap_or_else(|_| "production".into()),
            send_pii: false,
            logs: None,
            traces_sample_rate: 0.0,
            queue_capacity: 128,
            timeout: Duration::from_secs(2),
            duplicate_window: Duration::from_secs(1),
        }
    }
}

impl ReportingConfig {
    /// Read only the reporting subsystem's environment settings.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let driver = std::env::var("REPORTING_DRIVER").unwrap_or_else(|_| "sentry".into());
        let logs = match std::env::var("REPORTING_LOGS").ok().as_deref() {
            None | Some("") | Some("off") => None,
            Some(level) => Some(LogLevel::parse(level)?),
        };
        let rate = std::env::var("REPORTING_TRACES_SAMPLE_RATE")
            .unwrap_or_else(|_| "0".into())
            .parse()
            .map_err(|_| {
                FrameworkError::internal("REPORTING_TRACES_SAMPLE_RATE must be between 0 and 1")
            })?;
        let send_pii = match std::env::var("REPORTING_SEND_PII").ok().as_deref() {
            None | Some("") | Some("false") => false,
            Some("true") => true,
            _ => {
                return Err(FrameworkError::internal(
                    "REPORTING_SEND_PII must be true or false",
                ));
            }
        };
        let config = Self {
            driver: ReportingDriver::parse(&driver)?,
            dsn: std::env::var("REPORTING_DSN")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            release: std::env::var("REPORTING_RELEASE")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            logs,
            traces_sample_rate: rate,
            send_pii,
            ..Self::default()
        };
        config.validate()?;
        Ok(config)
    }

    /// Choose a driver without changing process-wide environment variables.
    pub fn driver(mut self, driver: ReportingDriver) -> Self {
        self.driver = driver;
        self
    }

    /// Keep the server credential separate from the public browser DSN.
    pub fn dsn(mut self, dsn: impl Into<String>) -> Self {
        self.dsn = Some(dsn.into()).filter(|s| !s.trim().is_empty());
        self
    }

    /// Identify the application's release independently of the framework version.
    pub fn release(mut self, release: impl Into<String>) -> Self {
        self.release = Some(release.into());
        self
    }

    /// Label reports from a deployment without exposing its process environment.
    pub fn environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = environment.into();
        self
    }

    /// Opt in before including user email or client IP in any event.
    pub fn send_pii(mut self, enabled: bool) -> Self {
        self.send_pii = enabled;
        self
    }

    /// Select the lowest log level to send; `None` keeps logs local.
    pub fn logs(mut self, level: Option<LogLevel>) -> Self {
        self.logs = level;
        self
    }

    /// Sample request and job transactions; build rejects values outside 0..=1.
    pub fn traces_sample_rate(mut self, rate: f32) -> Self {
        self.traces_sample_rate = rate;
        self
    }

    /// Bound retained envelopes so an outage cannot exhaust application memory.
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.queue_capacity = capacity;
        self
    }

    /// Bound each send independently of the process shutdown deadline.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Collapse repeated incidents inside this window; zero disables throttling.
    pub fn duplicate_window(mut self, window: Duration) -> Self {
        self.duplicate_window = window;
        self
    }

    fn validate(&self) -> Result<(), FrameworkError> {
        if !self.traces_sample_rate.is_finite() || !(0.0..=1.0).contains(&self.traces_sample_rate) {
            return Err(FrameworkError::internal(
                "REPORTING_TRACES_SAMPLE_RATE must be between 0 and 1",
            ));
        }
        if self.queue_capacity == 0 || self.queue_capacity > 65_536 || self.timeout.is_zero() {
            return Err(FrameworkError::internal(
                "reporting needs a queue capacity from 1 to 65536 and a positive send timeout",
            ));
        }
        Ok(())
    }

    /// Validate settings before starting a driver or binding it to the container.
    pub fn build(&self) -> Result<Arc<dyn Reporter>, FrameworkError> {
        self.validate()?;
        match self.driver {
            ReportingDriver::Null => Ok(Arc::new(NullReporter)),
            ReportingDriver::Log => Ok(Arc::new(LogReporter)),
            ReportingDriver::Sentry if self.dsn.is_none() => Ok(Arc::new(NullReporter)),
            #[cfg(feature = "reporting-sentry")]
            ReportingDriver::Sentry => Ok(Arc::new(super::SentryReporter::new(self)?)),
            #[cfg(not(feature = "reporting-sentry"))]
            ReportingDriver::Sentry => Err(FrameworkError::internal(
                "REPORTING_DRIVER=sentry needs the reporting-sentry feature",
            )),
        }
    }
}
