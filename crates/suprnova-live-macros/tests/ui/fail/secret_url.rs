use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "secret.url", view = "live/example.html")]
pub struct SecretUrl {
    #[secret]
    #[url]
    token: String,
}

fn main() {}
