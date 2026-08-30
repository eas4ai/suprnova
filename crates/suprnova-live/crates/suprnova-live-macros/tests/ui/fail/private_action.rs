use suprnova_live_macros::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "private.action", view = "live/example.html")]
pub struct PrivateAction {
    value: String,
}

#[live]
impl PrivateAction {
    #[action]
    async fn save(&mut self) {}
}

fn main() {}
