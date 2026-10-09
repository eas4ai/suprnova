//! Reads a notes file through the files capability of Suprnova's re-exported
//! Tokio, so the plan must show `files`.

use suprnova::live::{LiveComponent, live};

/// The notes the application keeps on disk.
#[derive(LiveComponent)]
#[live(name = "acme.notes", view = "acme-ui/notes/notes.html")]
pub struct Notes {
    /// The file's text.
    #[public]
    text: String,
}

#[live]
impl Notes {
    /// Starts empty.
    #[mount]
    pub fn mount() -> Self {
        Self { text: String::new() }
    }

    /// Loads the notes file.
    #[action]
    pub async fn load(&mut self) {
        if let Ok(text) = suprnova::tokio::fs::read_to_string("notes.txt").await {
            self.text = text;
        }
    }
}
