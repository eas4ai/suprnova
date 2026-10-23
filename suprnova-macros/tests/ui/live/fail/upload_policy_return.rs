use suprnova::live::LiveComponent;

fn policy() {}

#[derive(LiveComponent)]
#[live(name = "upload.return", view = "live/example.html")]
pub struct WrongPolicyReturn {
    #[model]
    #[upload(policy = policy)]
    value: String,
}

fn main() {}
