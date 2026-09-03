//! `app.avatar-uploader`: a single-file PNG upload bound to a model field,
//! finalized by `save_avatar` through the application's upload finalizer.

use suprnova::live::{
    LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live,
};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

/// An avatar picker rendered by `live/avatar-uploader.html`.
#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    /// The pending upload handle the browser proposes; finalized by `save_avatar`.
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    /// Finalizes the pending avatar upload through the application finalizer.
    #[action]
    pub fn save_avatar(&mut self) {}
}
