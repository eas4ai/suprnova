//! The `{name}` event.

use suprnova::Event;

/// Dispatch it with `suprnova::EventFacade::dispatch({name} {}).await?`.
/// Every listener of `{name}` receives its fields.
#[derive(Debug, Clone)]
pub struct {name} {}

impl Event for {name} {
    /// The name logs and fake assertions show.
    fn event_name() -> &'static str {
        "{name}"
    }
}
