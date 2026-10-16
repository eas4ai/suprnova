use crate::{Exceptions, FrameworkError};
use std::future::Future;

/// Record a handled failure through the same callbacks as an unhandled one.
pub fn report(error: &FrameworkError) -> Result<(), FrameworkError> {
    Exceptions::try_report(error)
}

/// Keep conditional reporting on the normal exception path.
pub fn report_if(condition: bool, error: &FrameworkError) -> Result<(), FrameworkError> {
    if condition {
        report(error)?;
    }
    Ok(())
}

/// Report a handled failure unless the caller's suppression condition holds.
pub fn report_unless(condition: bool, error: &FrameworkError) -> Result<(), FrameworkError> {
    report_if(!condition, error)
}

/// Report a rescued failure before returning the application's fallback value.
pub async fn rescue<T, F, D>(future: F, fallback: D) -> T
where
    F: Future<Output = Result<T, FrameworkError>>,
    D: FnOnce(FrameworkError) -> T,
{
    match future.await {
        Ok(value) => value,
        Err(error) => {
            Exceptions::report(&error);
            fallback(error)
        }
    }
}
