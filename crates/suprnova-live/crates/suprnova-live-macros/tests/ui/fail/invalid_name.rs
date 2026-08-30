use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "not allowed!", view = "live/example.html")]
pub struct InvalidName {
    value: String,
}

fn main() {}
