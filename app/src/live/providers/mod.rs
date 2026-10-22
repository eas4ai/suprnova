//! Explicit Live providers: the upload finalizer, the tenant resolver, and the
//! gates that authorize the dogfood streams and uploads.

pub mod tenant;
pub mod upload_finalizer;

use suprnova::{Gate, LiveMountRequest, ToolCaller};

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
/// signed-in user may subscribe to the activity streams and manage their own
/// avatar, gallery avatar and attachment uploads. The route guard has already
/// required the principal.
///
/// An MCP client mounts `app.tally` from the properties it sends
/// (`add_to_tally`), which are untrusted input: Live asks
/// `live:app.tally.mount` before anything mounts, and refuses when no policy
/// answers. A signed-in user may start a tally at any count; a call whose
/// `start` the policy cannot read as a count is refused.
pub fn authorize_live() {
    Gate::define::<String, String>("live:topic.event-tally", |principal, _| {
        !principal.is_empty()
    });
    Gate::define::<ToolCaller, LiveMountRequest>("live:app.tally.mount", |caller, mount| {
        caller.is_user()
            && mount
                .property("start")
                .and_then(suprnova::serde_json::Value::as_u64)
                .is_some()
    });
    Gate::define::<String, String>("live:app.activity-feed.stream.activity", |_, _| true);
    Gate::define::<String, String>("live:app.event-tally.stream.changes", |_, _| true);
    Gate::define::<String, String>("live:app.live-native-gallery.stream.activity", |_, _| true);
    for control in UPLOAD_CONTROLS {
        Gate::define::<String, String>(
            &format!("live:app.avatar-uploader.upload.avatar.{control}"),
            |_, _| true,
        );
        Gate::define::<String, String>(
            &format!("live:app.live-native-gallery.upload.attachment.{control}"),
            |_, _| true,
        );
        Gate::define::<String, String>(
            &format!("live:app.form-gallery.upload.avatar.{control}"),
            |_, _| true,
        );
    }
}
