use suprnova::live::LiveComponent;

struct UndeclaredPayload;

#[derive(LiveComponent)]
#[live(
    name = "missing.event.contract",
    view = "live/example.html",
    events(UndeclaredPayload)
)]
pub struct MissingEventContract {
    value: String,
}

fn main() {}
