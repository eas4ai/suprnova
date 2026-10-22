//! Replay holds the append lock so live persistence and queued workers cannot interleave.
use super::{EventStore, error, projections};
use crate::{DB, EventFacade, FrameworkError};

tokio::task_local! { static REPLAYING: bool; }

/// Announces the replay boundary before reset and the first projection write.
#[derive(Clone, Debug)]
pub struct ReplayStarted {
    /// Stable names selected for replay.
    pub projectors: Vec<String>,
    /// Exclusive lower global event position.
    pub from: i64,
    /// Inclusive upper position captured under the append lock.
    pub through: i64,
}
impl crate::Event for ReplayStarted {
    fn event_name() -> &'static str {
        "suprnova.event_sourcing.replay.started"
    }
}
/// Announces successful completion of the selected projector writes.
#[derive(Clone, Debug)]
pub struct ReplayFinished {
    /// Number of stored facts read, independent of the projector count.
    pub events: u64,
}
impl crate::Event for ReplayFinished {
    fn event_name() -> &'static str {
        "suprnova.event_sourcing.replay.finished"
    }
}

/// A bounded ordered rebuild that invokes projectors and excludes reactors.
#[derive(Clone, Debug)]
pub struct Replay {
    names: Option<Vec<String>>,
    from: i64,
    page_size: u64,
}
impl Replay {
    /// Selects every registered projector, including queued projectors.
    pub fn all() -> Self {
        Self {
            names: None,
            from: 0,
            page_size: 256,
        }
    }
    /// Selects projectors by stable name. An empty selection fails rather than replaying all.
    pub fn projectors(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            names: Some(names.into_iter().map(Into::into).collect()),
            ..Self::all()
        }
    }
    /// Resumes strictly after this global position without resetting read models.
    pub fn from(mut self, position: i64) -> Self {
        self.from = position;
        self
    }
    /// Sets the read page size. Run accepts values from one through ten thousand.
    pub fn page_size(mut self, size: u64) -> Self {
        self.page_size = size;
        self
    }
    /// Reports whether this task is inside a replay lifecycle.
    pub fn is_running() -> bool {
        REPLAYING.try_with(|value| *value).unwrap_or(false)
    }
    /// Resets full replays and commits every selected projector and checkpoint together.
    pub async fn run(self) -> Result<ReplayFinished, FrameworkError> {
        if self.from < 0 || !(1..=10_000).contains(&self.page_size) {
            return Err(error("invalid replay position or page size"));
        }
        if Self::is_running() || DB::transaction_level() > 0 {
            return Err(error("replay must own its transaction"));
        }
        let mut entries = projections::projectors()?;
        if let Some(names) = &self.names {
            if names.is_empty() {
                return Err(error("replay requires at least one projector name"));
            }
            for name in names {
                if !entries.iter().any(|entry| entry.projector.name() == name) {
                    return Err(error(format!("unknown projector '{name}'")));
                }
            }
            entries.retain(|entry| names.iter().any(|name| name == entry.projector.name()));
        }
        let finished =
            crate::live::topics::replay_scope(|| self.run_transaction(entries)).await??;
        EventFacade::dispatch(finished.clone()).await?;
        Ok(finished)
    }

    // Erase the transaction future before the generic buffering scope proves Send.
    fn run_transaction(
        self,
        entries: Vec<projections::Registration>,
    ) -> futures::future::BoxFuture<'static, Result<ReplayFinished, FrameworkError>> {
        Box::pin(REPLAYING.scope(
            true,
            DB::transaction(move |_tx| {
                Box::pin(async move {
                    EventStore::lock().await?;
                    let through = EventStore::last_position().await?;
                    let start = ReplayStarted {
                        projectors: entries
                            .iter()
                            .map(|entry| entry.projector.name().to_owned())
                            .collect(),
                        from: self.from,
                        through,
                    };
                    EventFacade::dispatch(start).await?;
                    for entry in &entries {
                        entry.projector.replay_started().await?;
                        projections::checkpoint(entry.projector.name()).await?;
                        if self.from == 0 {
                            entry.projector.reset().await?;
                            projections::set_checkpoint(entry.projector.name(), 0).await?;
                        }
                    }
                    let mut after = self.from;
                    let mut count = 0;
                    loop {
                        let page = EventStore::page(after, through, self.page_size).await?;
                        if page.is_empty() {
                            break;
                        }
                        for event in page {
                            for entry in &entries {
                                projections::apply(entry.projector.as_ref(), &event).await?;
                            }
                            after = event.id;
                            count += 1;
                        }
                    }
                    for entry in &entries {
                        entry.projector.replay_finished().await?;
                    }
                    let finished = ReplayFinished { events: count };
                    Ok(finished)
                })
            }),
        ))
    }
}
