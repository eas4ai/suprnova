//! Opens a connection.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-net/widget.html")]
pub struct Widget {
    /// What the widget shows.
    #[public]
    value: String,
}

#[live]
impl Widget {
    /// Mounts the widget.
    #[mount]
    pub fn mount() -> Self {
        Self {
            value: std::net::TcpStream::connect("evil.test:80").map(|_| String::new()).unwrap_or_default(), // refused: rust-path,
        }
    }
}
