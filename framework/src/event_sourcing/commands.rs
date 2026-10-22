//! Maintenance commands validate every argument before mutating projection state.
use super::{AggregateEntry, EventStore, Replay, error};
use crate::{FrameworkError, command};

#[command(
    name = "event-sourcing:replay",
    description = "Replay all or named projectors from a global event position"
)]
async fn replay(args: Vec<String>) -> Result<(), FrameworkError> {
    let mut names = Vec::new();
    let mut from = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--projector" => names.push(
                args.next()
                    .filter(|name| !name.is_empty() && !name.starts_with('-'))
                    .ok_or_else(|| error("event-sourcing:replay --projector requires a name"))?,
            ),
            "--from" => {
                if from.is_some() {
                    return Err(error("event-sourcing:replay --from supplied twice"));
                }
                let value = args
                    .next()
                    .ok_or_else(|| error("event-sourcing:replay --from requires a position"))?;
                let value: i64 = value
                    .parse()
                    .map_err(|_| error(format!("invalid replay position '{value}'")))?;
                if value < 0 {
                    return Err(error("replay position cannot be negative"));
                }
                from = Some(value);
            }
            _ => {
                return Err(error(format!(
                    "event-sourcing:replay unknown argument '{argument}'"
                )));
            }
        }
    }
    let replay = if names.is_empty() {
        Replay::all()
    } else {
        Replay::projectors(names)
    };
    let result = replay.from(from.unwrap_or(0)).run().await?;
    crate::console::line(format!("Replayed {} stored events", result.events));
    Ok(())
}

fn aggregate_uuid(command: &str, args: Vec<String>) -> Result<String, FrameworkError> {
    if args.len() != 1 {
        return Err(error(format!("{command} requires one aggregate UUID")));
    }
    EventStore::uuid(&args[0])
}

#[command(
    name = "event-sourcing:events",
    description = "List an aggregate's stored events as JSON lines"
)]
async fn events(args: Vec<String>) -> Result<(), FrameworkError> {
    let uuid = aggregate_uuid("event-sourcing:events", args)?;
    let mut after = 0;
    loop {
        let page = EventStore::aggregate_page(&uuid, after, 256).await?;
        if page.is_empty() {
            break;
        }
        for event in page {
            after = event.aggregate_version;
            crate::console::line(serde_json::to_string(&event).map_err(|e| error(e.to_string()))?);
        }
    }
    Ok(())
}

#[command(
    name = "event-sourcing:snapshot",
    description = "Rebuild and snapshot a registered aggregate"
)]
async fn snapshot(args: Vec<String>) -> Result<(), FrameworkError> {
    let uuid = aggregate_uuid("event-sourcing:snapshot", args)?;
    let first = EventStore::aggregate_page(&uuid, 0, 1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| error(format!("aggregate '{uuid}' has no stored history")))?;
    let name = first
        .metadata
        .get("aggregate_type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(format!("aggregate '{uuid}' has no aggregate_type metadata")))?;
    let mut entries = inventory::iter::<AggregateEntry>
        .into_iter()
        .filter(|entry| entry.name == name);
    let entry = entries
        .next()
        .ok_or_else(|| error(format!("unknown aggregate type '{name}'")))?;
    if entries.next().is_some() {
        return Err(error(format!("duplicate aggregate type '{name}'")));
    }
    let snapshot = (entry.snapshot)(uuid).await?;
    crate::console::line(serde_json::to_string(&snapshot).map_err(|e| error(e.to_string()))?);
    Ok(())
}
