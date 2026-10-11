use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "borrowed", view = "live/example.html")]
pub struct Borrowed {
    value: &'static str,
}

fn main() {}
