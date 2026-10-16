use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "accented", view = "live/counter.html")]
pub struct Accented {
    count: u64,
}

#[live]
impl Accented {
    #[action]
    pub fn rename(&mut self, café: String) {
        self.count = café.len() as u64;
    }
}

fn main() {}
