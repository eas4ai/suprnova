use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(
    name = "refresh",
    view = "live/example.html",
    minimum_protocol_version = 1,
    refresh_on_promote
)]
pub struct RefreshRequiresV2 {
    value: String,
}

fn main() {}
