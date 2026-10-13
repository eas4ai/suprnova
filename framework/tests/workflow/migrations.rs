//! The framework's workflow migrations over tables that already exist.
//!
//! An app whose database already holds `workflows` and `workflow_steps`
//! and their indexes (created by an earlier run, or by the scaffolder's
//! own copy of these migrations) runs `up` again. sea-query drops `IF NOT
//! EXISTS` from `CREATE INDEX` on MySQL, so before the index guard that
//! run failed there on the first existing index (error 1061). The
//! migrations now check for each index the same way on every backend, so
//! this SQLite test covers the path MySQL takes.

use sea_orm::Database;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::workflow::migrations::{CreateWorkflowStepsTable, CreateWorkflowsTable};

#[tokio::test]
async fn up_over_existing_workflow_tables_and_indexes_succeeds() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    CreateWorkflowsTable.up(&manager).await.unwrap();
    CreateWorkflowStepsTable.up(&manager).await.unwrap();

    CreateWorkflowsTable
        .up(&manager)
        .await
        .expect("up over an existing workflows table and its indexes");
    CreateWorkflowStepsTable
        .up(&manager)
        .await
        .expect("up over an existing workflow_steps table and its indexes");

    for (table, index) in [
        ("workflows", "idx_workflows_status"),
        ("workflows", "idx_workflows_next_run_at"),
        ("workflows", "idx_workflows_locked_until"),
        ("workflow_steps", "idx_workflow_steps_workflow_id"),
        ("workflow_steps", "idx_workflow_steps_unique"),
    ] {
        assert!(
            manager.has_index(table, index).await.unwrap(),
            "{table}.{index} is still there"
        );
    }
}
