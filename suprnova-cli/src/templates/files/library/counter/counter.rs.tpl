//! `{namespace}.counter`: a count kept on the server. A press of the button
//! runs the `increment` action, and the view renders the new count.

use suprnova::live::{LiveComponent, live};

/// A counter rendered by `{namespace}-ui/counter/counter.html`.
#[derive(LiveComponent)]
#[live(name = "{namespace}.counter", view = "{namespace}-ui/counter/counter.html")]
pub struct Counter {
    /// The current count, rendered by the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Adds one in answer to `live:click="increment"`, stopping at the
    /// largest count rather than wrapping to zero.
    #[action]
    pub fn increment(&mut self) {
        self.count = self.count.saturating_add(1);
    }
}
