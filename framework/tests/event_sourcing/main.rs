//! Domain history proofs run on isolated SQLite databases.
mod aggregates;
mod migrations;

use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde::{Deserialize, Serialize};
use suprnova::testing::TestDatabase;
use suprnova::{AggregateRoot, FrameworkError, StoredEvent, domain_event};

#[domain_event(name = "es.added", version = 2, upgrade = "upgrade_added")]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct RenamedAddition {
    amount: i64,
}
fn upgrade_added(
    version: i32,
    mut value: serde_json::Value,
) -> Result<serde_json::Value, FrameworkError> {
    if version == 1 {
        value["amount"] = value["old_amount"].take();
    }
    Ok(value)
}

#[suprnova::aggregate]
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
struct Tally {
    total: i64,
}
impl AggregateRoot for Tally {
    const NAME: &'static str = "es.tally";
    fn apply(&mut self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(event) = event.decode::<RenamedAddition>()? {
            self.total += event.amount;
        }
        Ok(())
    }
}

async fn database() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    suprnova::CreateEventSourcingTables
        .up(&SchemaManager::new(db.conn()))
        .await
        .unwrap();
    db
}
fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
