//! The `{name}` listener.

use suprnova::{Event, FrameworkError, Listener, async_trait};

/// Handles every event it is registered for. Register it in
/// `src/bootstrap.rs`, once per event:
/// `EventFacade::listen::<UserRegistered, _>(Arc::new({name})).await;`.
pub struct {name};

#[async_trait]
impl<E: Event> Listener<E> for {name} {
    /// Handle the event. An `Err` stops a fail-fast dispatch.
    async fn handle(&self, _event: &E) -> Result<(), FrameworkError> {
        Ok(())
    }
}
