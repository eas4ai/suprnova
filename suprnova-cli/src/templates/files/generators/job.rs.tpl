//! The `{name}` queued job.

use serde::{Deserialize, Serialize};
use suprnova::{FrameworkError, Job, async_trait};

/// A job a queue worker runs. Its fields go into the queue payload, so keep
/// them to the data the job needs. Register it at boot with
/// `suprnova::queue::worker::register_job::<{name}>();` and push it with
/// `suprnova::Queue::push({name} {}).await?`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct {name} {}

#[async_trait]
impl Job for {name} {
    /// The name the queue payload records. Renaming it strands the jobs
    /// already queued under the old name.
    fn job_name() -> &'static str {
        "{name}"
    }

    /// Run the job. An `Err` retries it, up to `max_tries`.
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
