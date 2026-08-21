use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "first", name = "second", view = "live/example.html")]
pub struct DuplicateStructHelper {
    value: String,
}

fn main() {}
