//! `app.overlay-gallery`: mounts every overlay and disclosure component the
//! library ships, so `live:check`, the document tests and the browser
//! matrix exercise the real set (Cairn OVL-001 to OVL-006).

use suprnova::live::{LiveComponent, live};

/// A page that uses each shipped overlay once, rendered by
/// `live/overlay-gallery.html`. Open state lives in the browser; the
/// actions own only their effect on the notes and the deletion flag.
#[derive(LiveComponent)]
#[live(name = "app.overlay-gallery", view = "live/overlay-gallery.html")]
pub struct OverlayGallery {
    /// Notes shown inside the collapsible, so a morph reaches an open
    /// disclosure without touching its open state.
    #[public]
    notes: Vec<String>,
    /// Set by the dialog's confirm action, the one server effect an overlay
    /// invokes.
    #[public]
    deleted: bool,
}

#[live]
impl OverlayGallery {
    /// Starts with one note and nothing deleted.
    #[mount]
    pub fn mount() -> Self {
        Self {
            notes: vec!["Note 1".to_owned()],
            deleted: false,
        }
    }

    /// Appends a note; invoked from the menu's action item.
    #[action]
    pub fn add_note(&mut self) {
        self.notes.push(format!("Note {}", self.notes.len() + 1));
    }

    /// Confirms the deletion the dialog asks about.
    #[action]
    pub fn confirm_delete(&mut self) {
        self.deleted = true;
    }

    /// Restores the mounted state.
    #[action]
    pub fn reset(&mut self) {
        let fresh = Self::mount();
        self.notes = fresh.notes;
        self.deleted = fresh.deleted;
    }
}
