//! `ProfileName`: the display-name form on the dashboard.
//!
//! An identity-bound island: it belongs to the signed-in user. The action
//! checks the name, saves it, and reloads the dashboard with a toast, so the
//! navigation and the welcome show the new name too. An empty name, or one
//! longer than 255 characters, stays on the form with its message.

use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Model, Validate, attrs};

use super::{FieldErrors, current_user, failed, redirect};
use crate::flash::Toast;

/// The rule the name must pass once trimmed.
#[derive(Validate)]
struct NameInput {
    #[validate(length(min = 1, max = 255, message = "Enter a name of 1 to 255 characters."))]
    name: String,
}

/// The display-name form, rendered by `live/profile_name.html`.
#[derive(LiveComponent)]
#[live(name = "{package_name}.profile-name", view = "live/profile_name.html")]
pub struct ProfileName {
    /// The name being edited, bound to the input.
    #[model]
    name: String,
    /// The message under the control when the last save failed.
    errors: FieldErrors,
}

#[live]
impl ProfileName {
    /// Starts from the name the user has now.
    #[mount]
    pub async fn mount() -> Result<Self, ComponentError> {
        let user = current_user().await?;
        Ok(Self {
            name: user.name,
            errors: FieldErrors::default(),
        })
    }

    /// Saves the new name, or shows why it was not saved.
    #[action]
    pub async fn save(&mut self) -> Result<ActionResult, ComponentError> {
        let input = NameInput {
            name: self.name.trim().to_owned(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        let user = current_user().await?;
        user.update(attrs! { name: input.name })
            .await
            .map_err(|error| failed("saving a display name", error))?;
        redirect("dashboard", Some(Toast::success("Your name was saved.")))
    }
}
