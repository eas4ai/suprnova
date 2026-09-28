use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "session", view = "live/example.html")]
pub struct SessionDehydrated {
    #[session]
    #[public]
    locale: String,
}

fn main() {}
