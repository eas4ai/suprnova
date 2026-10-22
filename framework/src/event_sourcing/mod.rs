//! Durable domain histories, aggregate snapshots and replayable read models.

mod aggregate;
mod migrations;
mod store;

pub use aggregate::{Aggregate, AggregateEntry, AggregateRoot};
pub use migrations::{CreateEventSourcingTables, migrations};
pub use store::{DomainEvent, DomainEventEntry, EventStore, Snapshot, StoredEvent};

use crate::FrameworkError;

fn error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::database(message)
}
