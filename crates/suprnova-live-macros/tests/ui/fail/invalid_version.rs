use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "invalid.version", view = "live/example.html", state_schema_version = 0)]
pub struct InvalidVersion {
    value: String,
}

fn main() {}
