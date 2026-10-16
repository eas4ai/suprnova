//! `app.counter`: the smallest island, one public field and two actions.

use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }

    /// Resets the counter to zero.
    #[action]
    pub fn reset(&mut self) {
        self.count = 0;
    }
}
