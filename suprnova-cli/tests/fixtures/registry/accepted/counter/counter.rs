//! `Counter` Live component, as `live:make` scaffolds it.

use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `acme-ui/counter/counter.html`.
#[derive(LiveComponent)]
#[live(name = "acme.counter", view = "acme-ui/counter/counter.html")]
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
}
