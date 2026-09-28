use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "unknown", view = "live/example.html", surprise = 1)]
pub struct UnknownStructHelper {
    value: String,
}

fn main() {}
