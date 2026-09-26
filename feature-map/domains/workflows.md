# Feature map: `manual/workflows.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 81 checked.

## Command line

### App runner (`suprnova::Application`; the project binary)

- [ ] command `app workflow:work` · framework/src/app/mod.rs:163
  - Run the workflow worker daemon

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova workflow:work` · suprnova-cli/src/main.rs:320
  - Start the workflow worker daemon
- [ ] command `suprnova workflow:install` · suprnova-cli/src/main.rs:323
  - Install workflow migrations

## Endpoints and tables

### framework migration `CreateWorkflowStepsTable`

- [ ] table `workflow_steps (framework)` · framework/src/workflow/migrations/m_create_workflow_steps_table.rs:105

### framework migration `CreateWorkflowsTable`

- [ ] table `workflows (framework)` · framework/src/workflow/migrations/m_create_workflows_table.rs:109

## Rust API: suprnova

### `suprnova::workflow::config`

- [ ] struct `suprnova::WorkflowConfig` · framework/src/workflow/config.rs:40 (also `suprnova::workflow::WorkflowConfig`, `suprnova::workflow::config::WorkflowConfig`)
  - Public fields: `poll_interval_ms`, `concurrency`, `lock_timeout_secs`, `max_attempts`, `retry_backoff_secs`
  - [ ] fn `suprnova::WorkflowConfig::from_env` · framework/src/workflow/config.rs:61
  - [ ] fn `suprnova::WorkflowConfig::validate` · framework/src/workflow/config.rs:127
- [ ] const `suprnova::workflow::config::MIN_CONCURRENCY` · framework/src/workflow/config.rs:9
- [ ] const `suprnova::workflow::config::MIN_LOCK_TIMEOUT_SECS` · framework/src/workflow/config.rs:16
- [ ] const `suprnova::workflow::config::MIN_MAX_ATTEMPTS` · framework/src/workflow/config.rs:21

### `suprnova::workflow::context`

- [ ] struct `suprnova::WorkflowContext` · framework/src/workflow/context.rs:18 (also `suprnova::workflow::WorkflowContext`, `suprnova::workflow::context::WorkflowContext`)
  - [ ] fn `suprnova::WorkflowContext::enter` · framework/src/workflow/context.rs:57
  - [ ] fn `suprnova::WorkflowContext::current` · framework/src/workflow/context.rs:65
  - [ ] fn `suprnova::WorkflowContext::is_active` · framework/src/workflow/context.rs:70
  - [ ] fn `suprnova::WorkflowContext::run_step_with_input` · framework/src/workflow/context.rs:90
  - [ ] fn `suprnova::WorkflowContext::run_step` · framework/src/workflow/context.rs:228

### `suprnova::workflow::entities::workflow_steps`

- [ ] struct `suprnova::workflow::entities::workflow_steps::ActiveModel` · framework/src/workflow/entities.rs:66
  - Public fields: `id`, `workflow_id`, `step_index`, `step_name`, `status`, `input`, `output`, `error`, `attempts`, `created_at`, `updated_at`, `started_at`, `completed_at`
- [ ] struct `suprnova::workflow::entities::workflow_steps::ColumnIter` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::Entity` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::Model` · framework/src/workflow/entities.rs:68
  - Public fields: `id`, `workflow_id`, `step_index`, `step_name`, `status`, `input`, `output`, `error`, `attempts`, `created_at`, `updated_at`, `started_at`, `completed_at`
  - [ ] fn `suprnova::workflow::entities::workflow_steps::Model::into_ex` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::PrimaryKeyIter` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::RelationIter` · framework/src/workflow/entities.rs:102
- [ ] enum `suprnova::workflow::entities::workflow_steps::Column` · framework/src/workflow/entities.rs:66
  - Variants: `Id`, `WorkflowId`, `StepIndex`, `StepName`, `Status`, `Input`, `Output`, `Error`, `Attempts`, `CreatedAt`, `UpdatedAt`, `StartedAt`, `CompletedAt`
- [ ] enum `suprnova::workflow::entities::workflow_steps::PrimaryKey` · framework/src/workflow/entities.rs:66
  - Variants: `Id`
- [ ] enum `suprnova::workflow::entities::workflow_steps::Relation` · framework/src/workflow/entities.rs:103

### `suprnova::workflow::entities::workflows`

- [ ] struct `suprnova::workflow::entities::workflows::ActiveModel` · framework/src/workflow/entities.rs:12
  - Public fields: `id`, `name`, `status`, `input`, `output`, `error`, `attempts`, `max_attempts`, `next_run_at`, `locked_until`, `worker_id`, `created_at`, `updated_at`, `started_at`, `completed_at`
- [ ] struct `suprnova::workflow::entities::workflows::ColumnIter` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::Entity` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::Model` · framework/src/workflow/entities.rs:14
  - Public fields: `id`, `name`, `status`, `input`, `output`, `error`, `attempts`, `max_attempts`, `next_run_at`, `locked_until`, `worker_id`, `created_at`, `updated_at`, `started_at`, `completed_at`
  - [ ] fn `suprnova::workflow::entities::workflows::Model::into_ex` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::PrimaryKeyIter` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::RelationIter` · framework/src/workflow/entities.rs:52
- [ ] enum `suprnova::workflow::entities::workflows::Column` · framework/src/workflow/entities.rs:12
  - Variants: `Id`, `Name`, `Status`, `Input`, `Output`, `Error`, `Attempts`, `MaxAttempts`, `NextRunAt`, `LockedUntil`, `WorkerId`, `CreatedAt`, `UpdatedAt`, `StartedAt`, `CompletedAt`
- [ ] enum `suprnova::workflow::entities::workflows::PrimaryKey` · framework/src/workflow/entities.rs:12
  - Variants: `Id`
- [ ] enum `suprnova::workflow::entities::workflows::Relation` · framework/src/workflow/entities.rs:53

### `suprnova::workflow::migrations::m_create_workflow_steps_table`

- [ ] struct `suprnova::workflow::migrations::CreateWorkflowStepsTable` · framework/src/workflow/migrations/m_create_workflow_steps_table.rs:11 (also `suprnova::workflow::migrations::m_create_workflow_steps_table::Migration`)

### `suprnova::workflow::migrations::m_create_workflows_table`

- [ ] struct `suprnova::workflow::migrations::CreateWorkflowsTable` · framework/src/workflow/migrations/m_create_workflows_table.rs:11 (also `suprnova::workflow::migrations::m_create_workflows_table::Migration`)

### `suprnova::workflow::migrations::m_normalize_mysql_datetime_columns`

- [ ] struct `suprnova::workflow::migrations::NormalizeWorkflowDateTimesForMysql` · framework/src/workflow/migrations/m_normalize_mysql_datetime_columns.rs:33 (also `suprnova::workflow::migrations::m_normalize_mysql_datetime_columns::Migration`)

### `suprnova::workflow::store`

- [ ] fn `suprnova::workflow::store::claim_next_workflow` · framework/src/workflow/store.rs:132
- [ ] fn `suprnova::workflow::store::get_workflow_output` · framework/src/workflow/store.rs:69
- [ ] fn `suprnova::workflow::store::get_workflow_record` · framework/src/workflow/store.rs:80
- [ ] fn `suprnova::workflow::store::get_workflow_status` · framework/src/workflow/store.rs:56
- [ ] fn `suprnova::workflow::store::insert_step_running` · framework/src/workflow/store.rs:646
- [ ] fn `suprnova::workflow::store::insert_workflow` · framework/src/workflow/store.rs:21
- [ ] fn `suprnova::workflow::store::load_step` · framework/src/workflow/store.rs:613
- [ ] fn `suprnova::workflow::store::load_step_by_index` · framework/src/workflow/store.rs:629
- [ ] fn `suprnova::workflow::store::mark_failed` · framework/src/workflow/store.rs:564
- [ ] fn `suprnova::workflow::store::mark_running` · framework/src/workflow/store.rs:90
- [ ] fn `suprnova::workflow::store::mark_step_failed` · framework/src/workflow/store.rs:819
- [ ] fn `suprnova::workflow::store::mark_step_succeeded` · framework/src/workflow/store.rs:761
- [ ] fn `suprnova::workflow::store::mark_succeeded` · framework/src/workflow/store.rs:453
- [ ] fn `suprnova::workflow::store::refresh_lock` · framework/src/workflow/store.rs:286
- [ ] fn `suprnova::workflow::store::requeue` · framework/src/workflow/store.rs:510
- [ ] fn `suprnova::workflow::store::update_step_running` · framework/src/workflow/store.rs:713

### `suprnova::workflow::types`

- [ ] struct `suprnova::workflow::types::ClaimedWorkflow` · framework/src/workflow/types.rs:194
  - Public fields: `id`, `name`, `input`, `attempts`, `max_attempts`, `worker_id`
- [ ] struct `suprnova::WorkflowHandle` · framework/src/workflow/types.rs:83 (also `suprnova::workflow::WorkflowHandle`, `suprnova::workflow::types::WorkflowHandle`)
  - [ ] fn `suprnova::WorkflowHandle::id` · framework/src/workflow/types.rs:93
  - [ ] fn `suprnova::WorkflowHandle::status` · framework/src/workflow/types.rs:98
  - [ ] fn `suprnova::WorkflowHandle::wait` · framework/src/workflow/types.rs:105
  - [ ] fn `suprnova::WorkflowHandle::wait_with_timeout` · framework/src/workflow/types.rs:118
  - [ ] fn `suprnova::WorkflowHandle::wait_with_options` · framework/src/workflow/types.rs:130
  - [ ] fn `suprnova::WorkflowHandle::output_raw` · framework/src/workflow/types.rs:174
  - [ ] fn `suprnova::WorkflowHandle::output` · framework/src/workflow/types.rs:179
- [ ] enum `suprnova::StepStatus` · framework/src/workflow/types.rs:49 (also `suprnova::workflow::StepStatus`, `suprnova::workflow::types::StepStatus`)
  - Variants: `Running`, `Succeeded`, `Failed`
  - [ ] fn `suprnova::StepStatus::as_str` · framework/src/workflow/types.rs:60
  - [ ] fn `suprnova::StepStatus::from_str` · framework/src/workflow/types.rs:71
- [ ] enum `suprnova::WorkflowStatus` · framework/src/workflow/types.rs:11 (also `suprnova::workflow::WorkflowStatus`, `suprnova::workflow::types::WorkflowStatus`)
  - Variants: `Pending`, `Running`, `Succeeded`, `Failed`
  - [ ] fn `suprnova::WorkflowStatus::as_str` · framework/src/workflow/types.rs:24
  - [ ] fn `suprnova::WorkflowStatus::from_str` · framework/src/workflow/types.rs:36

### `suprnova::workflow`

- [ ] fn `suprnova::start_named` · framework/src/workflow/mod.rs:192 (also `suprnova::workflow::start_named`)
- [ ] struct `suprnova::WorkflowWorker` · framework/src/workflow/mod.rs:205 (also `suprnova::workflow::WorkflowWorker`)
  - [ ] fn `suprnova::WorkflowWorker::new` · framework/src/workflow/mod.rs:227
  - [ ] fn `suprnova::WorkflowWorker::with_config` · framework/src/workflow/mod.rs:248
  - [ ] fn `suprnova::WorkflowWorker::worker_id` · framework/src/workflow/mod.rs:258
  - [ ] fn `suprnova::WorkflowWorker::work_loop` · framework/src/workflow/mod.rs:267
  - [ ] fn `suprnova::WorkflowWorker::run_with_cancel` · framework/src/workflow/mod.rs:278

### `suprnova`

- [ ] macro `suprnova::start_workflow` · framework/src/workflow/mod.rs:539

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::workflow` · suprnova-macros/src/lib.rs:569 (re-exported as `suprnova::workflow`)
  - Form: attribute `#[workflow]`
- [ ] proc macro `suprnova_macros::workflow_step` · suprnova-macros/src/lib.rs:575 (re-exported as `suprnova::workflow_step`)
  - Form: attribute `#[workflow_step]`
