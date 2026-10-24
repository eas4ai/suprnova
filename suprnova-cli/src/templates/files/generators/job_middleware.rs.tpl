//! The `{name}` job middleware.

use suprnova::{
    Envelope, FrameworkError, JobMiddleware, JobMiddlewareNext, JobOutcome, async_trait,
};

/// Wraps the handler of a queued job. Return it from the job's
/// `middleware()`: `vec![Arc::new({name})]`.
pub struct {name};

#[async_trait]
impl JobMiddleware for {name} {
    /// Process the job. `next(envelope).await` runs the rest of the
    /// pipeline and the handler; returning a `JobOutcome` instead settles
    /// the job without running it.
    async fn handle(
        &self,
        envelope: Envelope,
        next: JobMiddlewareNext,
    ) -> Result<JobOutcome, FrameworkError> {
        next(envelope).await
    }
}
