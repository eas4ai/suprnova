//! Workflow persistence helpers

use crate::database::DB;
use crate::error::FrameworkError;
use crate::workflow::config::WorkflowConfig;
use crate::workflow::context::WorkflowContext;
use crate::workflow::entities::{workflow_steps, workflows};
use crate::workflow::types::{ClaimedWorkflow, StepStatus, WorkflowHandle, WorkflowStatus};
use chrono::{Duration as ChronoDuration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, QueryFilter,
    QuerySelect, Set, TransactionTrait,
};
use sea_orm::{ConnectionTrait, Statement};
use std::time::Duration;

const ATTEMPT_BUDGET_EXHAUSTED_ERROR: &str = "Workflow attempt budget exhausted before claim";

/// Insert a new workflow row (pending)
pub async fn insert_workflow(
    name: &str,
    input: &str,
    max_attempts: i32,
) -> Result<WorkflowHandle, FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let model = workflows::ActiveModel {
        name: Set(name.to_string()),
        status: Set(WorkflowStatus::Pending.as_str().to_string()),
        input: Set(input.to_string()),
        output: Set(None),
        error: Set(None),
        attempts: Set(0),
        max_attempts: Set(max_attempts),
        next_run_at: Set(None),
        locked_until: Set(None),
        worker_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        started_at: Set(None),
        completed_at: Set(None),
        ..Default::default()
    };

    let inserted = model
        .insert(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    Ok(WorkflowHandle::new(inserted.id))
}

/// Get workflow status
pub async fn get_workflow_status(id: i64) -> Result<WorkflowStatus, FrameworkError> {
    let db = DB::connection()?;
    let model = workflows::Entity::find_by_id(id)
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Workflow not found"))?;

    WorkflowStatus::from_str(&model.status)
        .ok_or_else(|| FrameworkError::internal("Invalid workflow status"))
}

/// Get workflow output JSON
pub async fn get_workflow_output(id: i64) -> Result<Option<String>, FrameworkError> {
    let db = DB::connection()?;
    let model = workflows::Entity::find_by_id(id)
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Workflow not found"))?;
    Ok(model.output)
}

/// Load workflow record by id
pub async fn get_workflow_record(id: i64) -> Result<workflows::Model, FrameworkError> {
    let db = DB::connection()?;
    workflows::Entity::find_by_id(id)
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Workflow not found"))
}

/// Mark workflow as running (used for tests or manual claim)
pub async fn mark_running(
    id: i64,
    worker_id: &str,
    lock_timeout: Duration,
) -> Result<ClaimedWorkflow, FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();
    let lock_until =
        now + ChronoDuration::seconds(i64::try_from(lock_timeout.as_secs()).unwrap_or(i64::MAX));

    let model = workflows::Entity::find_by_id(id)
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Workflow not found"))?;

    let attempts = model.attempts + 1;
    let started_at = model.started_at.unwrap_or(now);
    let mut active: workflows::ActiveModel = model.into();
    active.attempts = Set(attempts);
    active.status = Set(WorkflowStatus::Running.as_str().to_string());
    active.locked_until = Set(Some(lock_until));
    active.worker_id = Set(Some(worker_id.to_string()));
    active.started_at = Set(Some(started_at));
    active.updated_at = Set(now);

    let updated = active
        .update(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    Ok(ClaimedWorkflow {
        id: updated.id,
        name: updated.name,
        input: updated.input,
        attempts: updated.attempts,
        max_attempts: updated.max_attempts,
        worker_id: updated.worker_id.unwrap_or_else(|| worker_id.to_string()),
    })
}

/// Claim the next workflow to run (Postgres only)
pub async fn claim_next_workflow(
    worker_id: &str,
    config: &WorkflowConfig,
) -> Result<Option<ClaimedWorkflow>, FrameworkError> {
    let db = DB::connection()?;
    let backend = db.inner().get_database_backend();
    if backend != DatabaseBackend::Postgres {
        return Err(FrameworkError::internal(
            "Workflow worker requires a Postgres database",
        ));
    }

    // The initial expiry is computed by the database (`NOW() + $1`),
    // not from a client-side timestamp taken before the round trip.
    // This matches the reclaim clock and avoids worker clock skew. A slow
    // response can still consume the lease, so execution must refresh and
    // verify ownership before entering user code.
    let lock_timeout_secs = i64::try_from(config.lock_timeout_secs).unwrap_or(i64::MAX);

    // Eligible-row predicate covers two cases:
    //   1. status='pending' rows ready to run (next_run_at elapsed, no live lock).
    //   2. status='running' rows whose `locked_until` lease has expired - the
    //      worker that owned them is presumed dead (process crash, hard kill,
    //      or panic that escaped the spawn boundary before our catch_unwind
    //      net was added). Reclaiming these is what turns `locked_until` from
    //      a hint into actual crash recovery; without it, a single crashed
    //      worker strands its in-flight row forever.
    // An eligible row whose budget is already exhausted cannot be left in
    // pending/running forever, but it must not be claimed again either. The
    // first two CTEs terminalize at most one such row per poll. The cleanup
    // and claim predicates are disjoint (`>=` versus `<`), so both updates
    // can safely execute in this one statement. Bounded cleanup avoids a
    // large abandoned backlog turning one worker poll into an unbounded
    // write.
    //
    // FOR UPDATE SKIP LOCKED keeps concurrent workers from racing on either
    // candidate. The outer UPDATE increments attempts, so a reclaimed crash
    // below budget counts the same way as a returned error. Applying the
    // budget predicate before that increment preserves the final legal
    // attempt (`max_attempts - 1` becomes `max_attempts`).
    let sql = r#"
        WITH exhausted_candidate AS (
            SELECT id
            FROM workflows
            WHERE attempts >= max_attempts
              AND (
                  (status = 'pending'
                   AND (next_run_at IS NULL OR next_run_at <= NOW())
                   AND (locked_until IS NULL OR locked_until <= NOW()))
                  OR
                  (status = 'running'
                   AND locked_until IS NOT NULL
                   AND locked_until <= NOW())
              )
            ORDER BY id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        ),
        terminalized AS (
            UPDATE workflows
            SET status = 'failed',
                error = $3,
                completed_at = COALESCE(completed_at, NOW()),
                locked_until = NULL,
                worker_id = NULL,
                updated_at = NOW()
            WHERE id = (SELECT id FROM exhausted_candidate)
              AND attempts >= max_attempts
            RETURNING id
        ),
        claimable_candidate AS (
            SELECT id
            FROM workflows
            WHERE attempts < max_attempts
              AND (
                  (status = 'pending'
                   AND (next_run_at IS NULL OR next_run_at <= NOW())
                   AND (locked_until IS NULL OR locked_until <= NOW()))
                  OR
                  (status = 'running'
                   AND locked_until IS NOT NULL
                   AND locked_until <= NOW())
              )
            ORDER BY id
            FOR UPDATE SKIP LOCKED
            LIMIT 1
        )
        UPDATE workflows
        SET status = 'running',
            attempts = attempts + 1,
            locked_until = NOW() + ($1 * INTERVAL '1 second'),
            worker_id = $2,
            started_at = COALESCE(started_at, NOW()),
            updated_at = NOW()
        WHERE id = (
            SELECT id FROM claimable_candidate
        )
        RETURNING id, name, input, attempts, max_attempts, worker_id
    "#;

    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        vec![
            lock_timeout_secs.into(),
            worker_id.into(),
            ATTEMPT_BUDGET_EXHAUSTED_ERROR.into(),
        ],
    );

    let row = db
        .inner()
        .query_one_raw(stmt)
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    if let Some(row) = row {
        let id: i64 = row
            .try_get("", "id")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let name: String = row
            .try_get("", "name")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let input: String = row
            .try_get("", "input")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let attempts: i32 = row
            .try_get("", "attempts")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let max_attempts: i32 = row
            .try_get("", "max_attempts")
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let worker_id: String = row
            .try_get("", "worker_id")
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        Ok(Some(ClaimedWorkflow {
            id,
            name,
            input,
            attempts,
            max_attempts,
            worker_id,
        }))
    } else {
        Ok(None)
    }
}

/// Refresh a workflow lock lease.
///
/// The update is fenced by `worker_id` and `attempts`. For compatibility,
/// losing ownership remains a successful no-op for direct callers; workflow
/// execution uses a strict crate-private variant that reports lease loss.
pub async fn refresh_lock(
    id: i64,
    lock_timeout: Duration,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    if !refresh_lock_if_owned(id, lock_timeout, worker_id, attempts).await? {
        tracing::warn!(
            workflow_id = id,
            worker_id,
            attempts,
            "workflow lock refresh rejected because another claim owns the row"
        );
    }

    Ok(())
}

pub(crate) async fn refresh_lock_owned(
    id: i64,
    lock_timeout: Duration,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    if refresh_lock_if_owned(id, lock_timeout, worker_id, attempts).await? {
        Ok(())
    } else {
        Err(workflow_lease_lost(id, worker_id, attempts))
    }
}

pub(crate) async fn refresh_lock_if_owned(
    id: i64,
    lock_timeout: Duration,
    worker_id: &str,
    attempts: i32,
) -> Result<bool, FrameworkError> {
    let now = Utc::now().naive_utc();
    refresh_lock_if_owned_at(id, lock_timeout, worker_id, attempts, now).await
}

pub(crate) async fn refresh_lock_if_owned_at(
    id: i64,
    lock_timeout: Duration,
    worker_id: &str,
    attempts: i32,
    now: chrono::NaiveDateTime,
) -> Result<bool, FrameworkError> {
    let db = DB::connection()?;
    let seconds = i64::try_from(lock_timeout.as_secs()).unwrap_or(i64::MAX);
    // PostgreSQL claims and reclaim checks use the database clock. Using a
    // worker timestamp here could immediately expire a successfully renewed
    // lease. The supplied clock remains the deterministic non-Postgres path.
    let (lock_until, updated_at) = if db.inner().get_database_backend() == DatabaseBackend::Postgres
    {
        (
            Expr::cust_with_values("NOW() + ($1 * INTERVAL '1 second')", [seconds]),
            Expr::cust("NOW()"),
        )
    } else {
        (
            Expr::value(Some(now + ChronoDuration::seconds(seconds))),
            Expr::value(now),
        )
    };

    let result = workflows::Entity::update_many()
        .col_expr(workflows::Column::LockedUntil, lock_until)
        .col_expr(workflows::Column::UpdatedAt, updated_at)
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::Status.eq(WorkflowStatus::Running.as_str()))
        .filter(workflows::Column::WorkerId.eq(worker_id))
        .filter(workflows::Column::Attempts.eq(attempts))
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    if result.rows_affected > 0 {
        return Ok(true);
    }

    // MySQL reports changed rows rather than matched rows by default. With
    // second-precision timestamp columns, two refreshes in the same second
    // can therefore report zero even though this token still owns the row.
    // Read the ownership tuple back before declaring the lease lost.
    workflows::Entity::find_by_id(id)
        .filter(workflows::Column::Status.eq(WorkflowStatus::Running.as_str()))
        .filter(workflows::Column::WorkerId.eq(worker_id))
        .filter(workflows::Column::Attempts.eq(attempts))
        .one(db.inner())
        .await
        .map(|row| row.is_some())
        .map_err(|error| FrameworkError::database(error.to_string()))
}

fn workflow_lease_lost(id: i64, worker_id: &str, attempts: i32) -> FrameworkError {
    FrameworkError::internal(format!(
        "Workflow lease lost for workflow {id} (worker_id={worker_id}, attempts={attempts})"
    ))
}

async fn lock_workflow_for_step(
    transaction: &DatabaseTransaction,
    workflow_id: i64,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    let owns_workflow = if transaction.get_database_backend() == DatabaseBackend::Sqlite {
        workflows::Entity::update_many()
            .col_expr(
                workflows::Column::UpdatedAt,
                Expr::value(Utc::now().naive_utc()),
            )
            .filter(workflows::Column::Id.eq(workflow_id))
            .filter(workflows::Column::Status.eq(WorkflowStatus::Running.as_str()))
            .filter(workflows::Column::WorkerId.eq(worker_id))
            .filter(workflows::Column::Attempts.eq(attempts))
            .exec(transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?
            .rows_affected
            == 1
    } else {
        workflows::Entity::find_by_id(workflow_id)
            .filter(workflows::Column::Status.eq(WorkflowStatus::Running.as_str()))
            .filter(workflows::Column::WorkerId.eq(worker_id))
            .filter(workflows::Column::Attempts.eq(attempts))
            .lock_exclusive()
            .one(transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?
            .is_some()
    };

    if owns_workflow {
        Ok(())
    } else {
        Err(workflow_lease_lost(workflow_id, worker_id, attempts))
    }
}

async fn finish_step_write(
    transaction: DatabaseTransaction,
    result: Result<(), FrameworkError>,
) -> Result<(), FrameworkError> {
    match result {
        Ok(()) => transaction
            .commit()
            .await
            .map_err(|error| FrameworkError::database(error.to_string())),
        Err(error) => match transaction.rollback().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(FrameworkError::database(format!(
                "workflow step mutation failed: {error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

/// Mark workflow as succeeded
///
/// Fenced by `worker_id` + `attempts` - see [`refresh_lock`] for the
/// mechanism. A caller whose lease was reclaimed by another worker affects
/// zero rows here; that is logged at `warn` and treated as success from
/// this caller's point of view (`Ok(())`), because the reclaiming worker
/// now owns the outcome and will settle the row itself. Never overwrites a
/// row it no longer owns.
pub async fn mark_succeeded(
    id: i64,
    output: &str,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let result = workflows::Entity::update_many()
        .col_expr(
            workflows::Column::Status,
            Expr::value(WorkflowStatus::Succeeded.as_str()),
        )
        .col_expr(
            workflows::Column::Output,
            Expr::value(Some(output.to_string())),
        )
        .col_expr(
            workflows::Column::Error,
            Expr::value(Option::<String>::None),
        )
        .col_expr(workflows::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(
            workflows::Column::LockedUntil,
            Expr::value(Option::<chrono::NaiveDateTime>::None),
        )
        .col_expr(
            workflows::Column::WorkerId,
            Expr::value(Option::<String>::None),
        )
        .col_expr(workflows::Column::UpdatedAt, Expr::value(now))
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::WorkerId.eq(worker_id))
        .filter(workflows::Column::Attempts.eq(attempts))
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    if result.rows_affected == 0 {
        tracing::warn!(
            workflow_id = id,
            worker_id,
            attempts,
            "workflow settlement (mark_succeeded): fencing check failed - another worker owns \
             this row now; dropping this write, the new owner is authoritative"
        );
    }

    Ok(())
}

/// Requeue workflow for retry
///
/// Fenced by `worker_id` + `attempts` - see [`refresh_lock`] for the
/// mechanism and [`mark_succeeded`] for the lease-lost handling this
/// mirrors.
pub async fn requeue(
    id: i64,
    error: &str,
    next_run_at: chrono::NaiveDateTime,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let result = workflows::Entity::update_many()
        .col_expr(
            workflows::Column::Status,
            Expr::value(WorkflowStatus::Pending.as_str()),
        )
        .col_expr(
            workflows::Column::Error,
            Expr::value(Some(error.to_string())),
        )
        .col_expr(workflows::Column::NextRunAt, Expr::value(Some(next_run_at)))
        .col_expr(
            workflows::Column::LockedUntil,
            Expr::value(Option::<chrono::NaiveDateTime>::None),
        )
        .col_expr(
            workflows::Column::WorkerId,
            Expr::value(Option::<String>::None),
        )
        .col_expr(workflows::Column::UpdatedAt, Expr::value(now))
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::WorkerId.eq(worker_id))
        .filter(workflows::Column::Attempts.eq(attempts))
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    if result.rows_affected == 0 {
        tracing::warn!(
            workflow_id = id,
            worker_id,
            attempts,
            "workflow settlement (requeue): fencing check failed - another worker owns this row \
             now; dropping this write, the new owner is authoritative"
        );
    }

    Ok(())
}

/// Mark workflow as failed
///
/// Fenced by `worker_id` + `attempts` - see [`refresh_lock`] for the
/// mechanism and [`mark_succeeded`] for the lease-lost handling this
/// mirrors.
pub async fn mark_failed(
    id: i64,
    error: &str,
    worker_id: &str,
    attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let result = workflows::Entity::update_many()
        .col_expr(
            workflows::Column::Status,
            Expr::value(WorkflowStatus::Failed.as_str()),
        )
        .col_expr(
            workflows::Column::Error,
            Expr::value(Some(error.to_string())),
        )
        .col_expr(workflows::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(
            workflows::Column::LockedUntil,
            Expr::value(Option::<chrono::NaiveDateTime>::None),
        )
        .col_expr(
            workflows::Column::WorkerId,
            Expr::value(Option::<String>::None),
        )
        .col_expr(workflows::Column::UpdatedAt, Expr::value(now))
        .filter(workflows::Column::Id.eq(id))
        .filter(workflows::Column::WorkerId.eq(worker_id))
        .filter(workflows::Column::Attempts.eq(attempts))
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;

    if result.rows_affected == 0 {
        tracing::warn!(
            workflow_id = id,
            worker_id,
            attempts,
            "workflow settlement (mark_failed): fencing check failed - another worker owns this \
             row now; dropping this write, the new owner is authoritative"
        );
    }

    Ok(())
}

/// Load a step by workflow + index
pub async fn load_step(
    workflow_id: i64,
    step_index: i32,
    step_name: &str,
) -> Result<Option<workflow_steps::Model>, FrameworkError> {
    let db = DB::connection()?;
    workflow_steps::Entity::find()
        .filter(workflow_steps::Column::WorkflowId.eq(workflow_id))
        .filter(workflow_steps::Column::StepIndex.eq(step_index))
        .filter(workflow_steps::Column::StepName.eq(step_name))
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))
}

/// Load any step by workflow + index (used to detect mismatches)
pub async fn load_step_by_index(
    workflow_id: i64,
    step_index: i32,
) -> Result<Option<workflow_steps::Model>, FrameworkError> {
    let db = DB::connection()?;
    workflow_steps::Entity::find()
        .filter(workflow_steps::Column::WorkflowId.eq(workflow_id))
        .filter(workflow_steps::Column::StepIndex.eq(step_index))
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))
}

/// Insert a running step for the active workflow context.
///
/// The active context must belong to `workflow_id`; its claim token fences the
/// write against a concurrent reclaim.
pub async fn insert_step_running(
    workflow_id: i64,
    step_index: i32,
    step_name: &str,
    input: &str,
) -> Result<(), FrameworkError> {
    let (worker_id, workflow_attempts) = WorkflowContext::current_claim_for(workflow_id)?;
    insert_step_running_owned(
        workflow_id,
        step_index,
        step_name,
        input,
        &worker_id,
        workflow_attempts,
    )
    .await
}

pub(crate) async fn insert_step_running_owned(
    workflow_id: i64,
    step_index: i32,
    step_name: &str,
    input: &str,
    worker_id: &str,
    workflow_attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let model = workflow_steps::ActiveModel {
        workflow_id: Set(workflow_id),
        step_index: Set(step_index),
        step_name: Set(step_name.to_string()),
        status: Set(StepStatus::Running.as_str().to_string()),
        input: Set(input.to_string()),
        output: Set(None),
        error: Set(None),
        attempts: Set(1),
        created_at: Set(now),
        updated_at: Set(now),
        started_at: Set(Some(now)),
        completed_at: Set(None),
        ..Default::default()
    };

    let transaction = db
        .inner()
        .begin()
        .await
        .map_err(|error| FrameworkError::database(error.to_string()))?;
    let result = async {
        lock_workflow_for_step(&transaction, workflow_id, worker_id, workflow_attempts).await?;
        model
            .insert(&transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?;
        Ok(())
    }
    .await;

    finish_step_write(transaction, result).await
}

/// Update a step to running for the active workflow context.
///
/// The active context must belong to the step's workflow; its claim token
/// fences the write against a concurrent reclaim.
pub async fn update_step_running(
    step: workflow_steps::Model,
    input: &str,
) -> Result<(), FrameworkError> {
    let (worker_id, workflow_attempts) = WorkflowContext::current_claim_for(step.workflow_id)?;
    update_step_running_owned(step, input, &worker_id, workflow_attempts).await
}

pub(crate) async fn update_step_running_owned(
    step: workflow_steps::Model,
    input: &str,
    worker_id: &str,
    workflow_attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let workflow_id = step.workflow_id;
    let attempts = step.attempts + 1;
    let mut active: workflow_steps::ActiveModel = step.into();
    active.status = Set(StepStatus::Running.as_str().to_string());
    active.input = Set(input.to_string());
    active.attempts = Set(attempts);
    active.updated_at = Set(now);
    active.started_at = Set(Some(now));

    let transaction = db
        .inner()
        .begin()
        .await
        .map_err(|error| FrameworkError::database(error.to_string()))?;
    let result = async {
        lock_workflow_for_step(&transaction, workflow_id, worker_id, workflow_attempts).await?;
        active
            .update(&transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?;
        Ok(())
    }
    .await;

    finish_step_write(transaction, result).await
}

/// Mark a step succeeded for the active workflow context.
///
/// The active context's claim token fences the write against a concurrent
/// reclaim of the step's parent workflow.
pub async fn mark_step_succeeded(step_id: i64, output: &str) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let workflow_id = workflow_steps::Entity::find_by_id(step_id)
        .one(db.inner())
        .await
        .map_err(|error| FrameworkError::database(error.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Step not found"))?
        .workflow_id;
    let (worker_id, workflow_attempts) = WorkflowContext::current_claim_for(workflow_id)?;
    mark_step_succeeded_owned(workflow_id, step_id, output, &worker_id, workflow_attempts).await
}

pub(crate) async fn mark_step_succeeded_owned(
    workflow_id: i64,
    step_id: i64,
    output: &str,
    worker_id: &str,
    workflow_attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let transaction = db
        .inner()
        .begin()
        .await
        .map_err(|error| FrameworkError::database(error.to_string()))?;
    let result = async {
        lock_workflow_for_step(&transaction, workflow_id, worker_id, workflow_attempts).await?;
        let mut active: workflow_steps::ActiveModel = workflow_steps::Entity::find_by_id(step_id)
            .filter(workflow_steps::Column::WorkflowId.eq(workflow_id))
            .one(&transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?
            .ok_or_else(|| FrameworkError::internal("Step not found"))?
            .into();

        active.status = Set(StepStatus::Succeeded.as_str().to_string());
        active.output = Set(Some(output.to_string()));
        active.error = Set(None);
        active.updated_at = Set(now);
        active.completed_at = Set(Some(now));

        active
            .update(&transaction)
            .await
            .map_err(|error| FrameworkError::database(error.to_string()))?;
        Ok(())
    }
    .await;

    finish_step_write(transaction, result).await
}

/// Mark a step failed for the active workflow context.
///
/// The active context's claim token fences the write against a concurrent
/// reclaim of the step's parent workflow.
pub async fn mark_step_failed(step_id: i64, error: &str) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let workflow_id = workflow_steps::Entity::find_by_id(step_id)
        .one(db.inner())
        .await
        .map_err(|database_error| FrameworkError::database(database_error.to_string()))?
        .ok_or_else(|| FrameworkError::internal("Step not found"))?
        .workflow_id;
    let (worker_id, workflow_attempts) = WorkflowContext::current_claim_for(workflow_id)?;
    mark_step_failed_owned(workflow_id, step_id, error, &worker_id, workflow_attempts).await
}

pub(crate) async fn mark_step_failed_owned(
    workflow_id: i64,
    step_id: i64,
    error: &str,
    worker_id: &str,
    workflow_attempts: i32,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let now = Utc::now().naive_utc();

    let transaction = db
        .inner()
        .begin()
        .await
        .map_err(|database_error| FrameworkError::database(database_error.to_string()))?;
    let result = async {
        lock_workflow_for_step(&transaction, workflow_id, worker_id, workflow_attempts).await?;
        let mut active: workflow_steps::ActiveModel = workflow_steps::Entity::find_by_id(step_id)
            .filter(workflow_steps::Column::WorkflowId.eq(workflow_id))
            .one(&transaction)
            .await
            .map_err(|database_error| FrameworkError::database(database_error.to_string()))?
            .ok_or_else(|| FrameworkError::internal("Step not found"))?
            .into();

        active.status = Set(StepStatus::Failed.as_str().to_string());
        active.error = Set(Some(error.to_string()));
        active.updated_at = Set(now);
        active.completed_at = Set(Some(now));

        active
            .update(&transaction)
            .await
            .map_err(|database_error| FrameworkError::database(database_error.to_string()))?;
        Ok(())
    }
    .await;

    finish_step_write(transaction, result).await
}
