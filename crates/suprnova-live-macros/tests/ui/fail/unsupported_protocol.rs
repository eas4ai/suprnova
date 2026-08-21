use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(
    name = "future.protocol",
    view = "live/example.html",
    minimum_protocol_version = 3
)]
pub struct UnsupportedProtocol {
    value: String,
}

fn main() {}
