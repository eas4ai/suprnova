//! Workflow configuration

use crate::config::env;
use crate::error::FrameworkError;

/// Minimum allowed `concurrency`. Zero would make the worker semaphore
/// permanently saturated (`acquire_owned()` parks forever); the worker
/// would never process anything.
pub const MIN_CONCURRENCY: usize = 1;

/// Minimum allowed workflow lease duration in seconds. Zero expires at the
/// claim timestamp, allowing another worker to reclaim the same workflow.
pub const MIN_LOCK_TIMEOUT_SECS: u64 = 1;

/// Minimum allowed `max_attempts`. A value below 1 prevents any attempt
/// from running (`attempts < max_attempts` is never true after the
/// first claim increments `attempts` to 1).
pub const MIN_MAX_ATTEMPTS: i32 = 1;

/// Workflow configuration
///
/// # Environment Variables
///
/// - `WORKFLOW_POLL_INTERVAL_MS` - Worker poll interval in milliseconds (default: 1000)
/// - `WORKFLOW_CONCURRENCY` - Number of workflows to process concurrently (default: 4, min: 1)
/// - `WORKFLOW_LOCK_TIMEOUT_SECS` - Lease duration in seconds (default: 30, min: 1)
/// - `WORKFLOW_MAX_ATTEMPTS` - Max workflow attempts (default: 3, min: 1)
/// - `WORKFLOW_RETRY_BACKOFF_SECS` - Linear backoff seconds (default: 5, min: 0)
///
/// # Validation
///
/// Out-of-range environment values are clamped (with a structured warning
/// emitted via `tracing`) so a typo in `.env` cannot brick a worker.
/// Use [`WorkflowConfig::validate`] for fail-fast checks on programmatic
/// configs supplied through the typed config registry.
#[derive(Debug, Clone)]
pub struct WorkflowConfig {
    /// Worker poll interval in milliseconds
    pub poll_interval_ms: u64,
    /// Max concurrent workflows processed by a worker
    pub concurrency: usize,
    /// Lease duration in seconds (minimum: 1)
    pub lock_timeout_secs: u64,
    /// Max attempts per workflow
    pub max_attempts: i32,
    /// Linear backoff seconds per attempt
    pub retry_backoff_secs: i64,
}

impl WorkflowConfig {
    /// Build config from environment variables.
    ///
    /// Out-of-range values are clamped to safe minimums rather than honoured
    /// blindly. Returning a "load-but-useless" config (e.g. concurrency=0)
    /// would deadlock the worker the first time it ran - clamping plus a
    /// structured warning makes the misconfiguration visible while keeping
    /// the worker functional.
    pub fn from_env() -> Self {
        let raw_concurrency = env("WORKFLOW_CONCURRENCY", 4usize);
        let concurrency = if raw_concurrency < MIN_CONCURRENCY {
            tracing::warn!(
                env = "WORKFLOW_CONCURRENCY",
                value = raw_concurrency,
                clamped_to = MIN_CONCURRENCY,
                "WORKFLOW_CONCURRENCY below minimum; clamping (0 would park the worker semaphore forever)"
            );
            MIN_CONCURRENCY
        } else {
            raw_concurrency
        };

        let raw_max_attempts = env("WORKFLOW_MAX_ATTEMPTS", 3i32);
        let max_attempts = if raw_max_attempts < MIN_MAX_ATTEMPTS {
            tracing::warn!(
                env = "WORKFLOW_MAX_ATTEMPTS",
                value = raw_max_attempts,
                clamped_to = MIN_MAX_ATTEMPTS,
                "WORKFLOW_MAX_ATTEMPTS below minimum; clamping (a row with attempts >= max_attempts is failed before its first run)"
            );
            MIN_MAX_ATTEMPTS
        } else {
            raw_max_attempts
        };

        let raw_backoff = env("WORKFLOW_RETRY_BACKOFF_SECS", 5i64);
        let retry_backoff_secs = if raw_backoff < 0 {
            tracing::warn!(
                env = "WORKFLOW_RETRY_BACKOFF_SECS",
                value = raw_backoff,
                clamped_to = 0i64,
                "WORKFLOW_RETRY_BACKOFF_SECS is negative; clamping (negative backoff schedules retries in the past, causing instant tight-loop reclaim)"
            );
            0
        } else {
            raw_backoff
        };

        let raw_lock_timeout = env("WORKFLOW_LOCK_TIMEOUT_SECS", 30u64);
        let lock_timeout_secs = if raw_lock_timeout < MIN_LOCK_TIMEOUT_SECS {
            tracing::warn!(
                env = "WORKFLOW_LOCK_TIMEOUT_SECS",
                value = raw_lock_timeout,
                clamped_to = MIN_LOCK_TIMEOUT_SECS,
                "WORKFLOW_LOCK_TIMEOUT_SECS below minimum; clamping (a zero-second lease is immediately reclaimable)"
            );
            MIN_LOCK_TIMEOUT_SECS
        } else {
            raw_lock_timeout
        };

        Self {
            poll_interval_ms: env("WORKFLOW_POLL_INTERVAL_MS", 1000u64),
            concurrency,
            lock_timeout_secs,
            max_attempts,
            retry_backoff_secs,
        }
    }

    /// Validate a programmatic config. Returns `Err` for any value the
    /// worker cannot honour. Use this when constructing a `WorkflowConfig`
    /// in code (not from env) to fail fast at boot instead of failing
    /// silently at runtime.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        if self.concurrency < MIN_CONCURRENCY {
            return Err(FrameworkError::internal(format!(
                "WorkflowConfig.concurrency must be >= {MIN_CONCURRENCY}; got {}. \
                 Zero concurrency makes the worker semaphore park forever.",
                self.concurrency
            )));
        }
        if self.max_attempts < MIN_MAX_ATTEMPTS {
            return Err(FrameworkError::internal(format!(
                "WorkflowConfig.max_attempts must be >= {MIN_MAX_ATTEMPTS}; got {}. \
                 The first claim increments attempts to 1, so max_attempts < 1 fails \
                 every workflow before its body runs.",
                self.max_attempts
            )));
        }
        if self.retry_backoff_secs < 0 {
            return Err(FrameworkError::internal(format!(
                "WorkflowConfig.retry_backoff_secs must be >= 0; got {}. \
                 Negative backoff schedules retries in the past, producing tight-loop \
                 reclaim instead of backoff.",
                self.retry_backoff_secs
            )));
        }
        if self.lock_timeout_secs < MIN_LOCK_TIMEOUT_SECS {
            return Err(FrameworkError::internal(format!(
                "WorkflowConfig.lock_timeout_secs must be >= {MIN_LOCK_TIMEOUT_SECS}; got {}. \
                 A zero-second lease expires at claim time, allowing concurrent workers \
                 to reclaim the same workflow.",
                self.lock_timeout_secs
            )));
        }
        if self.lock_timeout_secs > i64::MAX as u64 {
            return Err(FrameworkError::internal(format!(
                "WorkflowConfig.lock_timeout_secs must be <= {} (i64::MAX); got {}. \
                 Values above this wrap to a negative chrono duration, making every \
                 workflow lease appear expired and causing reclaim thrashing.",
                i64::MAX,
                self.lock_timeout_secs
            )));
        }
        Ok(())
    }
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const ENV_PROBE: &str = "SUPRNOVA_WORKFLOW_CONFIG_TEST_PROBE";
    const ENV_PROBE_TEST: &str = "workflow::config::tests::from_env_child_probe";

    fn run_from_env_probe(scenario: &str, key: &str, value: &str) {
        let output = Command::new(std::env::current_exe().expect("resolve current test binary"))
            .args(["--exact", ENV_PROBE_TEST, "--nocapture"])
            .env(ENV_PROBE, scenario)
            .env("WORKFLOW_POLL_INTERVAL_MS", "1000")
            .env("WORKFLOW_CONCURRENCY", "4")
            .env("WORKFLOW_LOCK_TIMEOUT_SECS", "30")
            .env("WORKFLOW_MAX_ATTEMPTS", "3")
            .env("WORKFLOW_RETRY_BACKOFF_SECS", "5")
            .env(key, value)
            .output()
            .expect("run isolated workflow config probe");

        assert!(
            output.status.success(),
            "isolated workflow config probe {scenario:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[test]
    fn from_env_child_probe() {
        let Ok(scenario) = std::env::var(ENV_PROBE) else {
            return;
        };

        let cfg = WorkflowConfig::from_env();
        match scenario.as_str() {
            "zero-concurrency" => assert!(
                cfg.concurrency >= MIN_CONCURRENCY,
                "concurrency=0 must be clamped to >= {MIN_CONCURRENCY}, got {}",
                cfg.concurrency,
            ),
            "negative-max-attempts" => assert!(
                cfg.max_attempts >= MIN_MAX_ATTEMPTS,
                "max_attempts=-3 must be clamped to >= {MIN_MAX_ATTEMPTS}, got {}",
                cfg.max_attempts,
            ),
            "negative-backoff" => assert!(
                cfg.retry_backoff_secs >= 0,
                "retry_backoff_secs=-7 must be clamped to >= 0, got {}",
                cfg.retry_backoff_secs,
            ),
            "zero-lock-timeout" => assert!(
                cfg.lock_timeout_secs >= MIN_LOCK_TIMEOUT_SECS,
                "lock_timeout_secs=0 must be clamped to >= {MIN_LOCK_TIMEOUT_SECS}, got {}",
                cfg.lock_timeout_secs,
            ),
            "positive-lock-timeout" => assert_eq!(cfg.lock_timeout_secs, 7),
            other => panic!("unknown workflow config probe scenario: {other}"),
        }
    }

    #[test]
    fn from_env_clamps_zero_concurrency_to_min() {
        run_from_env_probe("zero-concurrency", "WORKFLOW_CONCURRENCY", "0");
    }

    #[test]
    fn from_env_clamps_negative_max_attempts_to_min() {
        run_from_env_probe("negative-max-attempts", "WORKFLOW_MAX_ATTEMPTS", "-3");
    }

    #[test]
    fn from_env_clamps_negative_backoff_to_zero() {
        run_from_env_probe("negative-backoff", "WORKFLOW_RETRY_BACKOFF_SECS", "-7");
    }

    #[test]
    fn from_env_clamps_zero_lock_timeout_to_positive_minimum() {
        run_from_env_probe("zero-lock-timeout", "WORKFLOW_LOCK_TIMEOUT_SECS", "0");
    }

    #[test]
    fn from_env_preserves_positive_lock_timeout() {
        run_from_env_probe("positive-lock-timeout", "WORKFLOW_LOCK_TIMEOUT_SECS", "7");
    }

    #[test]
    fn validate_rejects_zero_concurrency() {
        let cfg = WorkflowConfig {
            poll_interval_ms: 1000,
            concurrency: 0,
            lock_timeout_secs: 30,
            max_attempts: 3,
            retry_backoff_secs: 5,
        };
        let err = cfg.validate().expect_err("zero concurrency must error");
        assert!(
            err.to_string().contains("concurrency"),
            "error must mention concurrency, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_negative_backoff() {
        let cfg = WorkflowConfig {
            poll_interval_ms: 1000,
            concurrency: 4,
            lock_timeout_secs: 30,
            max_attempts: 3,
            retry_backoff_secs: -1,
        };
        let err = cfg.validate().expect_err("negative backoff must error");
        assert!(
            err.to_string().contains("retry_backoff_secs"),
            "error must mention retry_backoff_secs, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_max_attempts() {
        let cfg = WorkflowConfig {
            poll_interval_ms: 1000,
            concurrency: 4,
            lock_timeout_secs: 30,
            max_attempts: 0,
            retry_backoff_secs: 5,
        };
        let err = cfg.validate().expect_err("zero max_attempts must error");
        assert!(
            err.to_string().contains("max_attempts"),
            "error must mention max_attempts, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_lock_timeout() {
        let cfg = WorkflowConfig {
            poll_interval_ms: 1000,
            concurrency: 4,
            lock_timeout_secs: 0,
            max_attempts: 3,
            retry_backoff_secs: 5,
        };
        let err = cfg.validate().expect_err("zero lock timeout must error");
        assert!(
            err.to_string().contains("lock_timeout_secs"),
            "error must name lock_timeout_secs, got: {err}",
        );
    }

    #[test]
    fn validate_passes_for_sane_defaults() {
        let cfg = WorkflowConfig {
            poll_interval_ms: 1000,
            concurrency: 4,
            lock_timeout_secs: 30,
            max_attempts: 3,
            retry_backoff_secs: 5,
        };
        cfg.validate().expect("default config must validate");
    }
}
