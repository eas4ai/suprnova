use suprnova::live::{LiveComponent, UploadPolicy, live};

fn policy() -> UploadPolicy {
    UploadPolicy::builder().build()
}

#[derive(LiveComponent)]
#[live(name = "upload.misplaced", view = "live/example.html")]
pub struct MisplacedUploadHelper {
    #[model]
    value: String,
}

#[live]
impl MisplacedUploadHelper {
    #[upload(policy = policy)]
    pub fn wrong_place(&mut self) {}
}

fn main() {}
