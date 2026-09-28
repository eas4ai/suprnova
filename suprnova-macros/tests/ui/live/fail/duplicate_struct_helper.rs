use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "first", name = "second", view = "live/example.html")]
pub struct DuplicateStructHelper {
    value: String,
}

fn main() {}
