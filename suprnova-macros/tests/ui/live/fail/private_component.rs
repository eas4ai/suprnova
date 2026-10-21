use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "private", view = "live/example.html")]
struct PrivateComponent {
    value: String,
}

fn main() {}
