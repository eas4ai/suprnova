//! Durable domain histories, aggregate snapshots and replayable read models.

mod aggregate;
mod commands;
mod migrations;
mod projections;
mod replay;
mod store;

pub use aggregate::{Aggregate, AggregateEntry, AggregateRoot};
pub use migrations::{CreateEventSourcingTables, migrations};
pub use store::{DomainEvent, DomainEventEntry, EventStore, Fact, Snapshot, StoredEvent};

use crate::FrameworkError;

fn error(message: impl Into<String>) -> FrameworkError {
    FrameworkError::database(message)
}

pub use projections::{Projector, ProjectorEntry, QueuedProjector, Reactor, ReactorEntry};
pub use replay::{Replay, ReplayFinished, ReplayStarted};
