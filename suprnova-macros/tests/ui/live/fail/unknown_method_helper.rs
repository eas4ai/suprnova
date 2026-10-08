use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "unknown.method", view = "live/example.html")]
pub struct UnknownMethodHelper {
    value: String,
}

#[live]
impl UnknownMethodHelper {
    #[surprise]
    pub fn surprise(&mut self) {}
}

fn main() {}
