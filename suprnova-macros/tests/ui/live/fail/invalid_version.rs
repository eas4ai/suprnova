use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "invalid.version", view = "live/example.html", state_schema_version = 0)]
pub struct InvalidVersion {
    value: String,
}

fn main() {}
