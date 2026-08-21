use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "generic", view = "live/example.html")]
pub struct Generic<T> {
    value: T,
}

fn main() {}
