use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "secret.url", view = "live/example.html")]
pub struct SecretUrl {
    #[secret]
    #[url]
    token: String,
}

fn main() {}
