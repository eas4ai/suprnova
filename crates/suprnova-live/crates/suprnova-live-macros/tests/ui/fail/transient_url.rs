use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "transient.url", view = "live/example.html")]
pub struct TransientUrl {
    #[model(transient)]
    #[url]
    query: String,
}

fn main() {}
