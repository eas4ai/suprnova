use suprnova::live::{LiveComponent, UploadPolicy};

fn policy(_field: &str) -> UploadPolicy {
    UploadPolicy::builder().build()
}

#[derive(LiveComponent)]
#[live(name = "upload.signature", view = "live/example.html")]
pub struct WrongPolicySignature {
    #[model]
    #[upload(policy = policy)]
    value: String,
}

fn main() {}
