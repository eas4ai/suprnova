//! Explicit Live providers: the upload finalizer, the tenant resolver, and the
//! gates that authorize the dogfood streams and uploads.

pub mod tenant;
pub mod upload_finalizer;

use suprnova::Gate;

/// Upload controls the engine authorizes per field through the application's gate.
const UPLOAD_CONTROLS: [&str; 14] = [
    "Create",
    "Reacquire",
    "Status",
    "Queue",
    "BeginTransfer",
    "PutChunk",
    "Complete",
    "Accept",
    "BeginFinalize",
    "CommitFinalize",
    "Cancel",
    "Reject",
    "Expire",
    "Fail",
];

/// Defines the abilities Live consults for the dogfood components: any
/// signed-in user may subscribe to the activity stream and manage their own
/// avatar upload. The route guard has already required the principal.
pub fn authorize_live() {
    Gate::define::<String, String>("live:app.activity-feed.stream.activity", |_, _| true);
    for control in UPLOAD_CONTROLS {
        Gate::define::<String, String>(
            &format!("live:app.avatar-uploader.upload.avatar.{control}"),
            |_, _| true,
        );
    }
}
