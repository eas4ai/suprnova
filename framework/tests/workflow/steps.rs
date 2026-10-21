//! `#[workflow_step]` and `#[workflow]` on functions whose bodies use their
//! arguments.
//!
//! The step macro hands its body to the workflow context as a `'static`
//! closure. Written as a borrowing closure, a step that took a `Copy`
//! argument such as an `i64` did not compile, because the closure held a
//! reference to it; this binary failing to build is that regression.

use suprnova::{FrameworkError, workflow, workflow_step};

#[workflow_step]
async fn double(value: i64) -> Result<i64, FrameworkError> {
    Ok(value * 2)
}

#[workflow_step]
async fn repeat(text: String, times: u32) -> Result<String, FrameworkError> {
    Ok(text.repeat(times as usize))
}

#[workflow]
async fn doubled_then_repeated(value: i64, text: String) -> Result<(), FrameworkError> {
    let doubled = double(value).await?;
    let times = u32::try_from(doubled)
        .map_err(|_| FrameworkError::internal("doubled value out of range"))?;
    repeat(text, times).await?;
    Ok(())
}

#[tokio::test]
async fn a_step_called_outside_a_workflow_runs_its_body_with_its_arguments() {
    assert_eq!(double(21).await.expect("the step runs"), 42);
    assert_eq!(
        repeat("ab".to_string(), 3).await.expect("the step runs"),
        "ababab"
    );
}
