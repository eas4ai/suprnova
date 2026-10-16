#![allow(dead_code)]

use suprnova::live::{
    LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadScanFailure, UploadType,
    live,
};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(2)
        .maximum_file_bytes(4 * 1024 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .accept(UploadType::Jpeg)
        .accept_application("application/pdf", &["pdf"])
        .dimensions(4096, 4096, 16_777_216)
        .scan(UploadScan::Required {
            on_timeout: UploadScanFailure::Retry,
            on_unavailable: UploadScanFailure::Reject,
        })
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "profile.avatar", view = "live/upload.html")]
pub struct Avatar {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl Avatar {
    #[action]
    pub fn save_avatar(&mut self) {}
}

fn main() {
    let descriptor =
        <Avatar as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
            .expect("generated upload metadata must be valid");
    let policy = descriptor.metadata().fields()[0]
        .upload_policy()
        .expect("upload policy");
    assert_eq!(policy.maximum_files(), 2);
    assert_eq!(policy.maximum_file_bytes(), 4 * 1024 * 1024);
    assert_eq!(policy.finalize_action().as_str(), "save_avatar");
}
