//! The `{name}` service registrations. Suprnova has no provider class: a
//! `register` function that the bootstrap calls plays its part.

/// Register the services of `{name}`: container bindings, event
/// listeners, observers and seeders. Call it from `register` in
/// `src/bootstrap.rs`: `crate::{module}::register().await;`.
pub async fn register() {
    // `App::singleton`, `App::bind` and `EventFacade::listen` calls go here.
}
