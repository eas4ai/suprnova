use suprnova::live::{LiveComponent, UploadPolicy};

fn policy() -> UploadPolicy {
    UploadPolicy::builder().build()
}

#[derive(LiveComponent)]
#[live(name = "upload.unknown", view = "live/example.html")]
pub struct UnknownUploadHelper {
    #[model]
    #[upload(unknown = policy)]
    value: String,
}

fn main() {}
