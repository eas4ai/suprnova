//! `app.account-menu`: the signed-in principal's menu, a stitch slot under
//! RenderCache (Cairn NAV-005). The dashboard mounts it as its own
//! identity-bound island, so the stitched document carries one slot for it
//! and the shared shell never holds the principal's name.

use suprnova::live::{LiveComponent, live};

/// The account menu rendered by `live/account-menu.html`.
#[derive(LiveComponent)]
#[live(name = "app.account-menu", view = "live/account-menu.html")]
pub struct AccountMenu {
    /// The principal's display name.
    #[public]
    name: String,
    /// The principal's initials for the avatar.
    #[public]
    initials: String,
    /// The session's CSRF token for the sign-out form.
    #[public]
    csrf: String,
}

#[live]
impl AccountMenu {
    /// Mounts for the dogfood principal.
    #[mount]
    pub fn mount() -> Self {
        Self {
            name: "Ada Lovelace".to_owned(),
            initials: "AL".to_owned(),
            csrf: suprnova::csrf_token().unwrap_or_default(),
        }
    }

    /// Re-renders the menu; the entries are anchors and a form, so nothing
    /// else needs an action.
    #[action]
    pub fn refresh(&mut self) {}
}
