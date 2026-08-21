use suprnova_live_macros::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "bad.action", view = "live/example.html")]
pub struct InvalidAction {
    value: String,
}

#[live]
impl InvalidAction {
    #[action]
    pub async fn save() {}
}

fn main() {}
