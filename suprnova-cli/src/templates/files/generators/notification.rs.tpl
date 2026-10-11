//! The `{name}` notification.

use serde::{Deserialize, Serialize};
use suprnova::serde_json;
use suprnova::{Notification, NotificationMailable};

/// A notification, sent on each channel `channels` names. Register its
/// mail renderer at boot with `suprnova::register_mail_renderer::<{name}>()?;`
/// and send it with `suprnova::Notify::send(&user, &{name} {}).await?`.
#[derive(Debug, Clone, Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "{title}",
    html = "<p>The introduction to the notification.</p><p>Thank you for using our application!</p>",
    text = "The introduction to the notification.\nThank you for using our application!"
)]
pub struct {name} {}

impl Notification for {name} {
    /// The name the database channel stores and a queued send records.
    fn notification_name() -> &'static str {
        "{name}"
    }

    /// The channels it is sent on.
    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }

    /// The payload the channels deliver or store.
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}
