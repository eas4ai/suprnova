//! The `{name}` mailable.

use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::Mailable;

/// A mail message. Its fields are the templates' context: `{{ field }}`
/// reads the field of that name. Send it with
/// `suprnova::mail::Mail::to(address).send({name} {}).await?`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct {name} {}

#[async_trait]
impl Mailable for {name} {
    /// The name a queued send records. Renaming it strands the mail
    /// already queued under the old name.
    fn mailable_name() -> &'static str {
        "{name}"
    }

    /// The subject line.
    fn subject(&self) -> String {
        "{title}".to_string()
    }

    /// The HTML body, a Tera template.
    fn html_template_source(&self) -> Option<String> {
        Some("<p>{title}</p>".to_string())
    }

    /// The plain-text body, a Tera template.
    fn text_template_source(&self) -> Option<String> {
        Some("{title}".to_string())
    }
}
