use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "conflict", view = "live/example.html")]
pub struct ModelLocked {
    #[model]
    #[locked]
    value: String,
}

fn main() {}
