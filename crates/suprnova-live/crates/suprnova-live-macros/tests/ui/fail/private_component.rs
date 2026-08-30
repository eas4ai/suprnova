use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "private", view = "live/example.html")]
struct PrivateComponent {
    value: String,
}

fn main() {}
