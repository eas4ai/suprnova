//! `VerifyEmail`: the notice on `/verify-email` and its resend button.
//!
//! Registration mails a verification link. This island names the address
//! the link went to and mails a fresh one on request. Tokens live in the
//! `auth_flow_tokens` table, are single-use, and expire after 24 hours.

use suprnova::MustVerifyEmail;
use suprnova::auth_flows::EmailVerification;
use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};

use super::{current_user, redirect};
use crate::controllers::email_verification::verification_link;
use crate::flash::Toast;

/// The notice and resend button, rendered by `live/verify_email.html`.
#[derive(LiveComponent)]
#[live(name = "{package_name}.verify-email", view = "live/verify_email.html")]
pub struct VerifyEmail {
    /// The address the link went to, so the notice can name it.
    email: String,
}

#[live]
impl VerifyEmail {
    /// Names the signed-in user's address.
    #[mount]
    pub async fn mount() -> Result<Self, ComponentError> {
        let user = current_user().await?;
        Ok(Self { email: user.email })
    }

    /// Mails a fresh link and returns to the notice. A verified account
    /// gets no mail: the notice sends it on to the dashboard.
    #[action]
    pub async fn resend(&mut self) -> Result<ActionResult, ComponentError> {
        let user = current_user().await?;
        if user.is_email_verified() {
            return redirect("dashboard", None);
        }
        let sent = EmailVerification::send_link(&user, &verification_link()).await;
        let toast = match sent {
            Ok(_) => Toast::info("A new verification link is on its way."),
            Err(error) => {
                tracing::error!(error = %error, "sending a verification link failed");
                Toast::error("The verification email could not be sent. Try again later.")
            }
        };
        redirect("verification.notice", Some(toast))
    }
}
