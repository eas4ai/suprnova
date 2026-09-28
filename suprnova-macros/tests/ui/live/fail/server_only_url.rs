use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "server.url", view = "live/example.html")]
pub struct ServerOnlyUrl {
    #[server_only]
    #[url]
    identity: String,
}

fn main() {}
