use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "borrowed", view = "live/example.html")]
pub struct Borrowed {
    value: &'static str,
}

fn main() {}
