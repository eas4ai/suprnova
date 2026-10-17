//! The starter kit's scheduled retention cleanup.

use suprnova::Schedule;

/// Keeps retained search activity bounded when the application runs its scheduler.
pub fn register(schedule: &mut Schedule) {
    schedule.add(schedule.command("search:analytics-prune").daily());
}
