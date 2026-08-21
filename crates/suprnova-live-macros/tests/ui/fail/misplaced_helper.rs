use suprnova_live_macros::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "misplaced", view = "live/example.html")]
pub struct MisplacedHelper {
    value: String,
}

#[live]
impl MisplacedHelper {
    #[model]
    pub fn wrong_place(&mut self) {}
}

fn main() {}
