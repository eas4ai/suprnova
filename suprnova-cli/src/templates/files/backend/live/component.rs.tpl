//! `{pascal}` Live component.
//!
//! Server state lives in this struct; the browser runtime sends typed actions
//! back over the Live protocol and morphs the re-rendered view in place.

use suprnova::live::{LiveComponent, live};

/// A counter island rendered by `{view}`.
#[derive(LiveComponent)]
#[live(name = "{component_name}", view = "{view}")]
pub struct {pascal} {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl {pascal} {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
