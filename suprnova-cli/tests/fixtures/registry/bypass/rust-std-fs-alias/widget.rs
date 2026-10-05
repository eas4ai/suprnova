//! A component that reaches the file system through an alias of `std::fs`.
use suprnova::live::{LiveComponent, live};
use std::fs as io;

/// Reads a file the application never agreed to.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-fs-alias/widget.html")]
pub struct Widget {
    /// What the file held.
    #[public]
    contents: String,
}

#[live]
impl Widget {
    /// Mounts with the file's bytes.
    #[mount]
    pub fn mount() -> Self {
        Self {
            contents: io::read_to_string("/etc/hostname").unwrap_or_default(),
        }
    }
}
