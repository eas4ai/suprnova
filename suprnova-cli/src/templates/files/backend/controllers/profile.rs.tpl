//! Profile: the signed-in user changes their display name.
//!
//! The dashboard's name form posts JSON here with `useHttp`, outside an
//! Inertia visit, so the answer is JSON too: `200 {"user": {..}}` with the
//! saved name, or the framework's `422 {"message", "errors"}` when the name
//! is empty or longer than 255 characters. The page shows the new name
//! before this answer arrives, and puts the old one back on a `422`.

use serde::Deserialize;
use suprnova::{
    Auth, FormRequest, FrameworkError, HttpResponse, Model, Response, Validate, ValidationErrors,
    attrs, handler,
};

use crate::models::user::User;
use crate::props::shared::UserInfo;

/// The name form's one field. A missing field reads as empty, so it fails
/// the rule below rather than the parse.
#[derive(Deserialize, Validate)]
pub struct UpdateNameRequest {
    #[serde(default)]
    #[validate(length(min = 1, max = 255, message = "Enter a name of 1 to 255 characters."))]
    pub name: String,
}

impl FormRequest for UpdateNameRequest {
    /// A name of spaces alone is no name. Laravel trims input before
    /// `required` sees it; this is the same check.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.name.trim().is_empty() {
            let mut errs = ValidationErrors::new();
            errs.add("name", "Enter a name of 1 to 255 characters.");
            return Err(errs);
        }
        Ok(())
    }
}

/// `POST /profile/name` - save the signed-in user's new name and answer
/// the user as the server now has them.
#[handler]
pub async fn update_name(form: UpdateNameRequest) -> Response {
    // Behind the `auth` middleware, so `None` means the session lost its
    // user between the guard and the handler; refusing is the safe answer.
    let user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let user = user.update(attrs! { name: form.name.trim() }).await?;

    Ok(HttpResponse::json(suprnova::serde_json::json!({
        "user": UserInfo::from(user),
    })))
}
