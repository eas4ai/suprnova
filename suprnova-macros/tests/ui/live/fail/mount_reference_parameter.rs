use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "mount.reference", view = "live/mount/reference.html")]
pub struct MountReference {
    value: String,
}

#[live]
impl MountReference {
    #[mount]
    pub fn mount(value: &str) -> Self {
        Self {
            value: value.to_owned(),
        }
    }
}

fn main() {}
