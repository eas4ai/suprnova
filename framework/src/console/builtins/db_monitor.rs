//! `db:monitor` - shows how many connections the server of each database
//! connection has, and with `--max` dispatches
//! [`DatabaseBusy`](crate::database::DatabaseBusy) for every server at or
//! over that number. Mirrors Laravel's `db:monitor`.
//!
//! The server counts, so the command gives the same answer from every
//! process that reaches the database, and it can run on the schedule:
//! `schedule.command("db:monitor --max 80").every_minute()`.

use async_trait::async_trait;
use suprnova_macros::Command;

use crate::console::{self, TypedCommand};
use crate::database::DB;
use crate::error::FrameworkError;

/// Show the connections the server of each database has
#[derive(clap::Parser, Debug, Command)]
#[console(name = "db:monitor")]
pub struct MonitorArgs {
    /// Dispatch DatabaseBusy for a server with this many connections,
    /// or more
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max: Option<u32>,
}

#[async_trait]
impl TypedCommand for MonitorArgs {
    async fn run(self) -> Result<(), FrameworkError> {
        let counts = DB::connection_counts().await?;
        if counts.is_empty() {
            // A monitor with nothing to look at must not look like one
            // that found nothing.
            return Err(FrameworkError::internal(
                "db:monitor: the application has no database connection",
            ));
        }

        // The counts that are shown are the counts that are judged.
        let busy = match self.max {
            Some(max) => DB::dispatch_busy(&counts, max).await?,
            None => Vec::new(),
        };
        for count in counts {
            let shown = match count.connections {
                Some(connections) => {
                    let alert = busy
                        .iter()
                        .any(|event| event.connection_name == count.connection_name);
                    let status = if alert { "ALERT" } else { "OK" };
                    format!("{connections} connections {status}")
                }
                None => "not counted, no server".to_owned(),
            };
            console::line(console::two_column_detail(&count.connection_name, &shown));
        }
        Ok(())
    }
}
