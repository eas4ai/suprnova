use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "conflict", view = "live/example.html")]
pub struct ModelLocked {
    #[model]
    #[locked]
    value: String,
}

fn main() {}
