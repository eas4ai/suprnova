use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "duplicate.action", view = "live/example.html")]
pub struct DuplicateActionName {
    value: String,
}

#[live]
impl DuplicateActionName {
    #[action(name = "save")]
    pub async fn save_primary(&mut self) {}

    #[action(name = "save")]
    pub async fn save_secondary(&mut self) {}
}

fn main() {}
