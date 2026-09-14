//! Session lifecycle hooks that end asynchronous Live memberships.
//!
//! LIVE-019 (decided 2026-09-13): a membership never outlives the session
//! that opened it on the node that destroys the session. The session
//! middleware calls [`session_destroyed`] when it destroys a session's store
//! row, and the "log out everywhere" path calls
//! [`principal_sessions_destroyed`]; both retire the matching memberships
//! through the same path a denied Gate takes, so the transport learns of
//! it and no later event reaches the old stream.

use suprnova_live::host::SessionFingerprint;

use super::attestation::{SecurityCheck, purpose_fingerprint};
use super::runtime::LiveRuntime;
use crate::container::App;

/// Retires every membership issued under the session whose id is
/// `session_id`, on this node.
///
/// A process without a bound Live runtime has no memberships to retire, so
/// the call is a no-op there rather than assembling a runtime.
pub(crate) async fn session_destroyed(session_id: &[u8]) {
    let Ok(runtime) = App::resolve::<LiveRuntime>() else {
        return;
    };
    let digest = purpose_fingerprint(SecurityCheck::Session, session_id);
    let Ok(session) = SessionFingerprint::from_bytes(&digest) else {
        return;
    };
    runtime.async_state().revoke_session(&session).await;
}

/// Retires every membership issued to `user_id`, on this node, when all of
/// that user's sessions are destroyed at once.
pub(crate) async fn principal_sessions_destroyed(user_id: &str) {
    let Ok(runtime) = App::resolve::<LiveRuntime>() else {
        return;
    };
    runtime.async_state().revoke_principal(user_id).await;
}
