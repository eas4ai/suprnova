use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "mount.destructured", view = "live/mount/destructured.html")]
pub struct MountDestructured {
    value: String,
}

#[live]
impl MountDestructured {
    #[mount]
    pub fn mount((value,): (String,)) -> Self {
        Self { value }
    }
}

fn main() {}
