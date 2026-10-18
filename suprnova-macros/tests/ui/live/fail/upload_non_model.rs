use suprnova::live::{LiveComponent, UploadPolicy};

fn policy() -> UploadPolicy {
    UploadPolicy::builder().build()
}

#[derive(LiveComponent)]
#[live(name = "upload.non-model", view = "live/example.html")]
pub struct NonModelUpload {
    #[upload(policy = policy)]
    value: String,
}

fn main() {}
