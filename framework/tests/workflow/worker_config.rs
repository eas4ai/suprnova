//! `WorkflowWorker::try_with_config`: the constructor for a config of
//! the application's own that checks what `WorkflowWorker::new` checks.
//!
//! This binary registers no workflow twice, so the registry passes here.
//! The refusal of a registry with a duplicate is tested beside the code,
//! in a binary that has one.

use suprnova::workflow::assert_no_duplicates;
use suprnova::{WorkflowConfig, WorkflowWorker};

#[test]
fn the_check_of_the_registry_is_reached_from_the_workflow_module() {
    assert_no_duplicates().expect("this binary registers no workflow twice");
}

#[test]
fn a_config_that_passes_builds_a_worker() {
    let worker = WorkflowWorker::try_with_config(WorkflowConfig::default())
        .expect("the default config is a config");
    assert!(!worker.worker_id().is_empty());
}

#[test]
fn a_config_that_is_none_is_the_error_and_no_panic() {
    let config = WorkflowConfig {
        concurrency: 0,
        ..WorkflowConfig::default()
    };

    let error = WorkflowWorker::try_with_config(config)
        .err()
        .expect("no worker runs nothing at a time");
    assert!(error.to_string().contains("concurrency"), "{error}");
}
