use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "session", view = "live/example.html")]
pub struct SessionDehydrated {
    #[session]
    #[public]
    locale: String,
}

fn main() {}
