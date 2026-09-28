use suprnova::live::{LiveComponent, UploadPolicy};

fn policy() -> UploadPolicy {
    UploadPolicy::builder().build()
}

#[derive(LiveComponent)]
#[live(name = "upload.duplicate", view = "live/example.html")]
pub struct DuplicateUploadHelper {
    #[model]
    #[upload(policy = policy)]
    #[upload(policy = policy)]
    value: String,
}

fn main() {}
