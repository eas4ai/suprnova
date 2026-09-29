//! The failed-job console commands: `queue:failed`, `queue:retry`,
//! `queue:forget`, `queue:flush` and `queue:prune-failed`.
//!
//! Each one is a thin wrapper over the [`FailedJobStore`] and
//! [`Queue::retry_failed`]. They live here, not in the application's command
//! dispatch, so that a command *returns* what it would print. The dispatch
//! arm prints the [`Report`] and sets the exit code; a test reads the same
//! value without a process to spawn.
//!
//! [`FailedJobStore`]: crate::queue::FailedJobStore

use crate::error::FrameworkError;
use crate::queue::Queue;
use crate::queue::failed::{self, FailedJob, FailedJobStore};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

/// Widest error text `queue:failed` prints for one job. The record keeps the
/// whole text; the listing is for finding the job, not for reading a trace.
const ERROR_COLUMN_WIDTH: usize = 80;

/// One failed-job command with its arguments as the operator typed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// `queue:failed`: list every failed job.
    Failed,
    /// `queue:retry <id>...` or `queue:retry all`.
    Retry(Vec<String>),
    /// `queue:forget <id>`.
    Forget(String),
    /// `queue:flush [--hours N]`.
    Flush {
        /// Only delete jobs that failed more than this many hours ago.
        hours: Option<u64>,
    },
    /// `queue:prune-failed [--hours N]`.
    PruneFailed {
        /// Delete jobs that failed more than this many hours ago.
        hours: u64,
    },
}

impl Command {
    /// The name the operator typed, for the error line.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Failed => "queue:failed",
            Self::Retry(_) => "queue:retry",
            Self::Forget(_) => "queue:forget",
            Self::Flush { .. } => "queue:flush",
            Self::PruneFailed { .. } => "queue:prune-failed",
        }
    }
}

/// What a command has to say, and whether it did what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Report {
    /// Lines for standard output, in order.
    pub(crate) lines: Vec<String>,
    /// `false` when part of the request could not be met, for example an id
    /// that names no failed job. The process then exits non-zero, so a
    /// script that retries a job finds out that nothing was retried.
    pub(crate) succeeded: bool,
}

/// Run one failed-job command against the configured store.
pub(crate) async fn run(command: Command) -> Result<Report, FrameworkError> {
    let store = store()?;
    match command {
        Command::Failed => list(store.as_ref()).await,
        Command::Retry(targets) => retry(&targets).await,
        Command::Forget(id) => forget(store.as_ref(), &id).await,
        Command::Flush { hours } => flush(store.as_ref(), hours).await,
        Command::PruneFailed { hours } => flush(store.as_ref(), Some(hours)).await,
    }
}

fn store() -> Result<Arc<dyn FailedJobStore>, FrameworkError> {
    failed::current().ok_or_else(|| {
        FrameworkError::internal(
            "no failed-job store is configured. QUEUE_DRIVER=database brings one; \
             with any other driver, call Queue::set_failed_store(...) in bootstrap",
        )
    })
}

async fn list(store: &dyn FailedJobStore) -> Result<Report, FrameworkError> {
    let records = store.all().await?;
    let lines = if records.is_empty() {
        vec!["No failed jobs found.".to_owned()]
    } else {
        format_listing(&records)
    };
    Ok(Report {
        lines,
        succeeded: true,
    })
}

/// What `queue:retry` was asked to retry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RetryTarget {
    All,
    Ids(Vec<Uuid>),
}

/// Read the arguments of `queue:retry`. `all` anywhere in the list means
/// every failed job, as it does for `php artisan queue:retry`.
fn parse_retry_targets(targets: &[String]) -> Result<RetryTarget, FrameworkError> {
    if targets.iter().any(|t| t.trim().eq_ignore_ascii_case("all")) {
        return Ok(RetryTarget::All);
    }
    if targets.is_empty() {
        return Err(FrameworkError::internal(
            "name the failed jobs to retry by id, or pass `all`",
        ));
    }
    targets
        .iter()
        .map(|t| parse_id(t))
        .collect::<Result<Vec<_>, _>>()
        .map(RetryTarget::Ids)
}

fn parse_id(raw: &str) -> Result<Uuid, FrameworkError> {
    Uuid::parse_str(raw.trim()).map_err(|_| {
        FrameworkError::internal(format!(
            "`{raw}` is not a failed-job id. `queue:failed` lists the ids"
        ))
    })
}

async fn retry(targets: &[String]) -> Result<Report, FrameworkError> {
    match parse_retry_targets(targets)? {
        RetryTarget::All => {
            let retried = Queue::retry_all_failed(None).await?;
            Ok(Report {
                lines: vec![match retried {
                    0 => "No failed jobs found.".to_owned(),
                    n => format!("{n} failed job(s) pushed back onto the queue."),
                }],
                succeeded: true,
            })
        }
        RetryTarget::Ids(ids) => {
            let mut lines = Vec::with_capacity(ids.len());
            let mut succeeded = true;
            for id in ids {
                if Queue::retry_failed(id).await? {
                    lines.push(format!(
                        "The failed job [{id}] has been pushed back onto the queue."
                    ));
                } else {
                    lines.push(format!("No failed job matches the id [{id}]."));
                    succeeded = false;
                }
            }
            Ok(Report { lines, succeeded })
        }
    }
}

async fn forget(store: &dyn FailedJobStore, raw: &str) -> Result<Report, FrameworkError> {
    let id = parse_id(raw)?;
    let forgotten = store.forget(id).await?;
    Ok(Report {
        lines: vec![if forgotten {
            format!("The failed job [{id}] has been deleted.")
        } else {
            format!("No failed job matches the id [{id}].")
        }],
        succeeded: forgotten,
    })
}

async fn flush(store: &dyn FailedJobStore, hours: Option<u64>) -> Result<Report, FrameworkError> {
    let cutoff = hours
        .map(|h| cutoff_before(crate::clock::now(), h))
        .transpose()?;
    let deleted = store.flush(cutoff).await?;
    let line = match hours {
        Some(h) => format!("{deleted} failed job(s) older than {h} hour(s) deleted."),
        None => format!("{deleted} failed job(s) deleted."),
    };
    Ok(Report {
        lines: vec![line],
        succeeded: true,
    })
}

/// The instant `hours` before `now`. An `hours` that reaches past what a
/// timestamp can hold is refused: treating it as "everything" would delete
/// on a typing mistake.
fn cutoff_before(now: DateTime<Utc>, hours: u64) -> Result<DateTime<Utc>, FrameworkError> {
    i64::try_from(hours)
        .ok()
        .and_then(chrono::Duration::try_hours)
        .and_then(|span| now.checked_sub_signed(span))
        .ok_or_else(|| FrameworkError::internal(format!("--hours={hours} is out of range")))
}

/// The `queue:failed` table: a header, then one line per job, newest first
/// as the store returns them.
fn format_listing(records: &[FailedJob]) -> Vec<String> {
    let rows: Vec<[String; 6]> = records
        .iter()
        .map(|r| {
            [
                r.id.to_string(),
                r.connection.clone(),
                r.queue.clone(),
                r.job_name.clone(),
                r.failed_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                first_line(&r.exception),
            ]
        })
        .collect();
    let header = ["ID", "Connection", "Queue", "Job", "Failed at", "Error"];
    let widths: Vec<usize> = (0..header.len())
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain(std::iter::once(header[column].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let render = |cells: &[&str]| {
        cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    std::iter::once(render(&header))
        .chain(rows.iter().map(|row| {
            let cells: Vec<&str> = row.iter().map(String::as_str).collect();
            render(&cells)
        }))
        .collect()
}

/// The first line of an error, cut to [`ERROR_COLUMN_WIDTH`] characters.
/// Counted in characters, so a cut never lands inside one.
fn first_line(exception: &str) -> String {
    let line = exception.lines().next().unwrap_or("").trim();
    if line.chars().count() <= ERROR_COLUMN_WIDTH {
        return line.to_owned();
    }
    let kept: String = line.chars().take(ERROR_COLUMN_WIDTH - 3).collect();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::envelope::Envelope;
    use crate::queue::{MemoryFailedJobStore, MemoryQueueDriver, QueueDriver};
    use serial_test::serial;

    fn envelope(job_name: &str) -> Envelope {
        let mut env = crate::queue::chain::ChainLink {
            job_name: job_name.to_owned(),
            payload: serde_json::json!({}),
            max_tries: 1,
            timeout_secs: None,
            fail_on_timeout: false,
            backoff: Default::default(),
            queue: None,
            delay_secs: None,
        }
        .to_envelope();
        env.attempts = 1;
        env
    }

    /// A fresh failed-job store and queue driver, installed as the
    /// process-wide ones. The caller holds `#[serial]`.
    fn install() -> (Arc<MemoryFailedJobStore>, Arc<MemoryQueueDriver>) {
        let store = Arc::new(MemoryFailedJobStore::new());
        Queue::set_failed_store(store.clone());
        let driver = Arc::new(MemoryQueueDriver::new());
        Queue::set_driver(driver.clone());
        (store, driver)
    }

    async fn fail(store: &MemoryFailedJobStore, job_name: &str, error: &str) -> Uuid {
        store
            .log("database", "default", &envelope(job_name), error)
            .await
            .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn failed_lists_every_job_with_the_first_line_of_its_error() {
        let (store, _driver) = install();
        let id = fail(
            &store,
            "SendInvoice",
            "smtp refused\n  at send()\n  at run()",
        )
        .await;

        let report = run(Command::Failed).await.unwrap();

        assert!(report.succeeded);
        assert_eq!(report.lines.len(), 2, "a header and one job: {report:?}");
        assert!(report.lines[0].starts_with("ID"));
        for column in ["Connection", "Queue", "Job", "Failed at", "Error"] {
            assert!(report.lines[0].contains(column), "{column} is a column");
        }
        let row = &report.lines[1];
        assert!(row.starts_with(&id.to_string()));
        assert!(row.contains("database"));
        assert!(row.contains("default"));
        assert!(row.contains("SendInvoice"));
        assert!(row.ends_with("smtp refused"), "only the first line: {row}");
    }

    #[tokio::test]
    #[serial]
    async fn failed_says_so_when_there_is_nothing_to_list() {
        let _installed = install();
        let report = run(Command::Failed).await.unwrap();
        assert_eq!(report.lines, ["No failed jobs found."]);
        assert!(report.succeeded);
    }

    #[tokio::test]
    #[serial]
    async fn retry_pushes_the_named_job_back_and_deletes_its_record() {
        let (store, driver) = install();
        let kept = fail(&store, "SendInvoice", "boom").await;
        let retried = fail(&store, "SendReceipt", "boom").await;

        let report = run(Command::Retry(vec![retried.to_string()]))
            .await
            .unwrap();

        assert!(report.succeeded);
        assert_eq!(
            report.lines,
            [format!(
                "The failed job [{retried}] has been pushed back onto the queue."
            )]
        );
        assert_eq!(driver.size().await.unwrap(), 1);
        assert_eq!(store.ids().await.unwrap(), [kept]);
    }

    #[tokio::test]
    #[serial]
    async fn retry_reports_an_id_that_names_no_job_and_still_retries_the_rest() {
        let (store, driver) = install();
        let known = fail(&store, "SendInvoice", "boom").await;
        let unknown = Uuid::new_v4();

        let report = run(Command::Retry(vec![unknown.to_string(), known.to_string()]))
            .await
            .unwrap();

        assert!(!report.succeeded, "one id was not retried");
        assert_eq!(
            report.lines,
            [
                format!("No failed job matches the id [{unknown}]."),
                format!("The failed job [{known}] has been pushed back onto the queue."),
            ]
        );
        assert_eq!(driver.size().await.unwrap(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn retry_all_pushes_every_job_back() {
        let (store, driver) = install();
        fail(&store, "SendInvoice", "boom").await;
        fail(&store, "SendReceipt", "boom").await;

        let report = run(Command::Retry(vec!["all".into()])).await.unwrap();

        assert_eq!(
            report.lines,
            ["2 failed job(s) pushed back onto the queue."]
        );
        assert_eq!(driver.size().await.unwrap(), 2);
        assert_eq!(store.count().await.unwrap(), 0);
    }

    #[tokio::test]
    #[serial]
    async fn retry_refuses_an_argument_that_is_not_an_id() {
        let (store, driver) = install();
        fail(&store, "SendInvoice", "boom").await;

        let error = run(Command::Retry(vec!["42".into()])).await.unwrap_err();

        assert!(error.to_string().contains("`42` is not a failed-job id"));
        assert_eq!(driver.size().await.unwrap(), 0, "nothing was retried");
        assert_eq!(store.count().await.unwrap(), 1);
    }

    #[tokio::test]
    #[serial]
    async fn forget_deletes_one_record_and_fails_for_an_unknown_id() {
        let (store, driver) = install();
        let id = fail(&store, "SendInvoice", "boom").await;

        let report = run(Command::Forget(id.to_string())).await.unwrap();
        assert!(report.succeeded);
        assert_eq!(
            report.lines,
            [format!("The failed job [{id}] has been deleted.")]
        );
        assert_eq!(store.count().await.unwrap(), 0);
        assert_eq!(driver.size().await.unwrap(), 0, "forget never pushes");

        let again = run(Command::Forget(id.to_string())).await.unwrap();
        assert!(!again.succeeded);
        assert_eq!(
            again.lines,
            [format!("No failed job matches the id [{id}].")]
        );
    }

    #[tokio::test]
    #[serial]
    async fn flush_deletes_everything_and_prune_keeps_recent_failures() {
        let (store, _driver) = install();
        fail(&store, "SendInvoice", "boom").await;
        fail(&store, "SendReceipt", "boom").await;

        // Both failed a moment ago, so a one-hour cutoff keeps them.
        let pruned = run(Command::PruneFailed { hours: 1 }).await.unwrap();
        assert_eq!(
            pruned.lines,
            ["0 failed job(s) older than 1 hour(s) deleted."]
        );
        assert_eq!(store.count().await.unwrap(), 2);

        // A cutoff of zero hours is "now": both are older than that.
        let pruned = run(Command::Flush { hours: Some(0) }).await.unwrap();
        assert_eq!(
            pruned.lines,
            ["2 failed job(s) older than 0 hour(s) deleted."]
        );

        fail(&store, "SendInvoice", "boom").await;
        let flushed = run(Command::Flush { hours: None }).await.unwrap();
        assert_eq!(flushed.lines, ["1 failed job(s) deleted."]);
        assert_eq!(store.count().await.unwrap(), 0);
    }

    #[test]
    fn a_cutoff_past_the_calendar_is_refused() {
        let error = cutoff_before(crate::clock::now(), u64::MAX).unwrap_err();
        assert!(error.to_string().contains("is out of range"));
    }

    #[test]
    fn a_long_error_is_cut_on_a_character_boundary() {
        let long = "é".repeat(200);
        let cut = first_line(&long);
        assert_eq!(cut.chars().count(), ERROR_COLUMN_WIDTH);
        assert!(cut.ends_with("..."));
        assert_eq!(first_line("short\nsecond line"), "short");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn all_anywhere_in_the_arguments_means_every_job() {
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            parse_retry_targets(&[id.clone(), "ALL".into()]).unwrap(),
            RetryTarget::All
        );
        assert!(matches!(
            parse_retry_targets(&[id]).unwrap(),
            RetryTarget::Ids(ids) if ids.len() == 1
        ));
        assert!(parse_retry_targets(&[]).is_err());
    }
}
