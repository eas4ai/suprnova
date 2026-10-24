//! `app.tally`: a count an MCP client keeps for a signed-in user, with no
//! browser. Its one action, `add`, is the headless Live tool `add_to_tally`
//! (`src/agents.rs`).
//!
//! The headless tool has its own component rather than an action on
//! `app.counter`. A Live tool narrows who may run its action, and the
//! component's islands list its tools for each viewer. A user-only tool on
//! the counter would refuse a guest's click on the public page's counter,
//! and make that page read its viewer, so RenderCache could no longer keep
//! the one copy it shares with everybody.
//!
//! An agent starts a tally from `start` and gets back a handle, Live's
//! sealed snapshot, which carries the count to its next call. The handle is
//! bound to the caller, so no one else can act on it, and it expires as a
//! browser snapshot does.

use suprnova::live::{LiveComponent, live};

/// A count rendered by `live/tally.html`. The agent reads the count, and
/// nothing else is stored.
#[derive(LiveComponent)]
#[live(name = "app.tally", view = "live/tally.html")]
pub struct Tally {
    /// The count so far.
    #[agent_visible]
    count: u64,
}

#[live]
impl Tally {
    /// Starts a tally at `start`. An agent sends it, so the mount policy in
    /// `live::providers` checks it before anything mounts.
    #[mount]
    pub fn mount(#[doc = "The initial tally count."] start: u64) -> Self {
        Self { count: start }
    }

    /// Adds `amount` to the count, stopping at the largest count rather
    /// than wrapping around.
    #[tool(
        name = "add_to_tally",
        title = "Add to a tally",
        description = "Add an amount to a tally kept for the signed-in user, and read the new count. Start a tally with `mount.start`, or continue one with the `handle` the last call answered.",
        callers = [user],
        arguments(amount = "How much to add to the tally."),
        headless,
    )]
    #[action]
    pub fn add(&mut self, amount: u64) {
        self.count = self.count.saturating_add(amount);
    }
}
