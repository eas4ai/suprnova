use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "missing.validation", view = "live/example.html")]
pub struct MissingValidationHook {
    email: String,
}

#[live]
impl MissingValidationHook {
    #[action(validate = "whole")]
    pub fn save(&mut self) {}
}

fn main() {}
