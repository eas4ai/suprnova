//! Mail notification channel - delivers via the bound mail transport.
//!
//! [`MailChannel`] is stateless. Each notification that wants to be
//! delivered via mail opts in by implementing [`NotificationMailable`]
//! and registering its renderer once at boot via
//! [`register_mail_renderer`]. At dispatch time, the channel looks up
//! the renderer by `Notification::notification_name()` and invokes
//! `N::to_mail(&self)` on the notification itself to produce a
//! [`MailRendering`]. On the queued path that is the notification the
//! worker rebuilt from its full serialized form; `data()`, the public
//! payload other channels persist, is not what `to_mail` reads. The channel
//! then assembles an `OutgoingMessage` addressed to the route
//! returned by the `Notifiable` and dispatches it through
//! `Mail::current_transport`.
//!
//! Empty-body guard: if the renderer returns a rendering with neither
//! `html` nor `text`, delivery fails fast with a clear error. This
//! mirrors `MailBuilder::send`'s upstream check on the `Mailable` path -
//! we refuse to silently dispatch blank emails through any code path.
//!
//! Why a per-Notification trait rather than a single factory closure
//! at construction time: the closure approach centralized rendering
//! logic in one match-on-name and lost type safety on the JSON
//! payload. The trait gives each notification ownership of its own
//! mail representation, matches Laravel's `toMail()` idiom, and hands
//! the renderer a serde-deserialized concrete type instead of raw
//! JSON.

use crate::error::FrameworkError;
use crate::lock;
use crate::mail::transport::OutgoingMessage;
use crate::mail::{Address, Attachment, Mail};
use crate::notifications::{Channel, DynNotification, Notification};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

/// What a per-notification renderer must produce - enough to assemble
/// an outgoing message. At least one of `html` / `text` must be `Some` or
/// delivery will fail. `from` is optional and falls back to
/// `noreply@localhost` to match `MailBuilder::send`. An empty `subject`
/// falls back to the notification's name in title case (`InvoicePaid`
/// gives `Invoice Paid`), as Laravel's mail channel falls back to the
/// class name.
///
/// `cc`, `bcc`, `reply_to`, `attachments` and `attachment_paths` are
/// optional and default to empty. Use `..Default::default()` in the struct
/// literal to skip any field you don't need, and the `attach*` methods to
/// add files:
///
/// ```rust,no_run
/// # use suprnova::notifications::channels::mail::MailRendering;
/// # fn ex() -> MailRendering {
/// MailRendering {
///     subject: "Order shipped".into(),
///     text: Some("Tracking: 1Z999".into()),
///     ..Default::default()
/// }
/// # }
/// ```
///
/// # Forward-compat contract
///
/// `#[derive(NotificationMailable)]` populates every field listed below
/// explicitly *plus* a trailing `..Default::default()`. **Adding a new
/// field here must keep `MailRendering: Default`** so the derive's
/// generated code stays valid without a macro change. Reorder freely;
/// breaking the `Default` impl is the only thing to avoid.
#[derive(Default)]
pub struct MailRendering {
    /// Email subject line.
    pub subject: String,
    /// Optional HTML body part.
    pub html: Option<String>,
    /// Optional plain-text body part.
    pub text: Option<String>,
    /// Override the configured default `From:` address.
    pub from: Option<Address>,
    /// Carbon-copy recipients.
    pub cc: Vec<Address>,
    /// Blind-carbon-copy recipients.
    pub bcc: Vec<Address>,
    /// `Reply-To:` addresses.
    pub reply_to: Vec<Address>,
    /// File attachments included with the message.
    pub attachments: Vec<Attachment>,
    /// Files attached by path, read when the mail is delivered and sent
    /// after `attachments`. Fill it with [`MailRendering::attach_path`] and
    /// [`MailRendering::attach_many`].
    pub attachment_paths: Vec<PathAttachment>,
}

/// The name and content type of a file attached by path. Mirrors the
/// `as` and `mime` options of Laravel's `MailMessage::attach`.
///
/// A `None` name is the file's own name; a `None` content type follows the
/// file's extension, `application/octet-stream` when the extension is
/// unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttachOptions {
    /// The file name the recipient sees.
    pub name: Option<String>,
    /// The MIME content type, such as `application/pdf`.
    pub content_type: Option<String>,
}

/// One file attached by path to a [`MailRendering`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathAttachment {
    /// The file to read when the mail is delivered.
    pub path: PathBuf,
    /// Its name and content type.
    pub options: AttachOptions,
}

impl MailRendering {
    /// Attach `attachment`. Mirrors Laravel's `MailMessage::attach` given
    /// an `Attachment`.
    pub fn attach(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Attach `bytes` as a file named `name` of type `content_type`.
    /// Mirrors Laravel's `MailMessage::attachData`.
    pub fn attach_data(
        self,
        bytes: impl Into<Vec<u8>>,
        name: impl Into<String>,
        content_type: impl Into<String>,
    ) -> Self {
        self.attach(Attachment::new(name, bytes.into(), content_type))
    }

    /// Attach the file at `path`, read when the mail is delivered, not now.
    /// Mirrors Laravel's `MailMessage::attach($path, $options)`.
    ///
    /// Reading at delivery keeps a queued notification small, since the
    /// rendering runs on the worker, and a file that cannot be read then
    /// fails the delivery with an error naming the path.
    pub fn attach_path(mut self, path: impl Into<PathBuf>, options: AttachOptions) -> Self {
        self.attachment_paths.push(PathAttachment {
            path: path.into(),
            options,
        });
        self
    }

    /// Attach each `(path, options)` of `files`, as
    /// [`MailRendering::attach_path`] does. Mirrors Laravel's
    /// `MailMessage::attachMany`.
    pub fn attach_many<I, P>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = (P, AttachOptions)>,
        P: Into<PathBuf>,
    {
        for (path, options) in files {
            self = self.attach_path(path, options);
        }
        self
    }
}

/// Opt-in trait for Notifications that want to be deliverable via the
/// mail channel.
///
/// The Notification owns its mail representation - `to_mail` produces
/// the rendered subject/body content. No `Notifiable` argument: the
/// queued path loses the original `Notifiable`, so per-recipient
/// variation must ride on the Notification's own fields (the whole
/// notification is serialized at queue time and rebuilt before
/// `to_mail` runs). `to_mail` reads those fields, not `data()`, so a
/// field the mail needs may stay out of the public payload.
///
/// Bootstrap registers each implementor once via
/// [`register_mail_renderer::<N>()`]. The [`MailChannel`] then looks
/// up the renderer by `N::notification_name()` at dispatch time.
pub trait NotificationMailable: Notification {
    /// Render this notification into a [`MailRendering`] (subject, HTML/text
    /// body, addressing, attachments) ready for the mail transport.
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}

/// Renderer function pointer. v1 uses `fn(...)` rather than
/// `Arc<dyn Fn>` because registered renderers are stateless - every
/// renderer is the monomorphized closure produced by
/// [`register_mail_renderer`], which only closes over the type
/// parameter `N`. Bump to `Arc<dyn Fn>` if a future caller needs to
/// capture state.
type MailRendererFn = fn(&dyn DynNotification) -> Result<MailRendering, FrameworkError>;

static MAIL_RENDERERS: RwLock<Option<HashMap<&'static str, MailRendererFn>>> = RwLock::new(None);

/// Register a Notification's mail renderer. The [`MailChannel`] uses
/// the notification name (from `Notification::notification_name()`)
/// as the registry key.
///
/// Re-registering the same name silently replaces the existing
/// renderer (last-write-wins) - matches the notification factory
/// registry and the dispatcher's channel registration.
pub fn register_mail_renderer<N: NotificationMailable>() -> Result<(), FrameworkError> {
    let renderer: MailRendererFn = |notification| {
        // The notification itself, so `to_mail` sees every field.
        if let Some(n) = notification
            .as_any()
            .and_then(|any| any.downcast_ref::<N>())
        {
            return n.to_mail();
        }
        // A different type registered under the same name, or a
        // hand-written `DynNotification`: nothing typed to hand over, so
        // decode `N` from the public payload as the only shape there is.
        let n: N = serde_json::from_value(notification.data()).map_err(|e| {
            FrameworkError::internal(format!("decode {}: {e}", N::notification_name()))
        })?;
        n.to_mail()
    };
    let mut g = lock::write(&MAIL_RENDERERS, "notification mail renderers")?;
    g.get_or_insert_with(HashMap::new)
        .insert(N::notification_name(), renderer);
    Ok(())
}

fn renderer_for(name: &str) -> Result<MailRendererFn, FrameworkError> {
    let missing = || {
        FrameworkError::internal(format!(
            "no mail renderer for notification {name} - register via suprnova::register_mail_renderer::<N>()"
        ))
    };
    let g = lock::read(&MAIL_RENDERERS, "notification mail renderers")?;
    // Treat "registry never initialized" identically to "this notification
    // not registered" - the operator-facing fix is the same.
    let map = g.as_ref().ok_or_else(missing)?;
    map.get(name).copied().ok_or_else(missing)
}

/// Notification channel that delivers via the bound mail transport.
///
/// Stateless - construction takes no arguments. At dispatch time the
/// channel looks up the per-notification renderer in the global
/// registry populated by [`register_mail_renderer`].
///
/// `cc`, `bcc`, `reply_to`, and `attachments` ride through
/// [`MailRendering`] - populate any of them in `to_mail` and the
/// channel threads them into the outgoing message verbatim. Files in
/// `attachment_paths` are read at delivery and attached after
/// `attachments`; an empty subject becomes the notification's name in
/// title case.
pub struct MailChannel;

impl MailChannel {
    /// Build a new `MailChannel`. Stateless - no arguments needed.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MailChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Channel for MailChannel {
    fn name(&self) -> &'static str {
        "mail"
    }

    async fn deliver(
        &self,
        route: &str,
        notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        let renderer = renderer_for(notification.name())?;
        let rendering = renderer(notification)?;

        // Empty-body guard - mirror MailBuilder::send's upstream check
        // so notification dispatch can never silently send a blank
        // email. Runs BEFORE current_transport() so a missing
        // transport doesn't mask a misconfigured renderer.
        if rendering.html.is_none() && rendering.text.is_none() {
            return Err(FrameworkError::internal(format!(
                "MailChannel: renderer for {} returned no html or text body",
                notification.name()
            )));
        }

        // Read every file attached by path before anything is sent, so a
        // file that cannot be read fails the delivery as a whole.
        let mut attachments = rendering.attachments;
        for file in &rendering.attachment_paths {
            attachments.push(read_attachment(file).await?);
        }

        let from = rendering
            .from
            .unwrap_or_else(|| Address::new("noreply@localhost"));
        let mut msg = OutgoingMessage::new(from);
        msg.to = vec![route.into()];
        msg.cc = rendering.cc;
        msg.bcc = rendering.bcc;
        msg.reply_to = rendering.reply_to;
        msg.subject = if rendering.subject.is_empty() {
            subject_from_name(notification.name())
        } else {
            rendering.subject
        };
        msg.html = rendering.html;
        msg.text = rendering.text;
        msg.attachments = attachments;
        let msg = Mail::apply_always_defaults(msg);

        let transport = Mail::current_transport()?;
        // A notification sent by mail is a dispatched mail, so it fires
        // `MessageSending` and `MessageSent` like any other (Laravel's mail
        // channel sends through the mailer, which fires them too).
        crate::mail::deliver(transport.as_ref(), &msg).await
    }
}

/// Read one file attached by path into an [`Attachment`]: its name is the
/// option's or the file's own, its content type the option's or the one
/// its extension names, `application/octet-stream` when the extension is
/// unknown. The content type carries no charset, as a mail part's own
/// header names none.
async fn read_attachment(file: &PathAttachment) -> Result<Attachment, FrameworkError> {
    let content = tokio::fs::read(&file.path).await.map_err(|e| {
        FrameworkError::internal(format!(
            "MailChannel: cannot read the attachment {}: {e}",
            file.path.display()
        ))
    })?;
    let name = file.options.name.clone().unwrap_or_else(|| {
        file.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.path.display().to_string())
    });
    let content_type = file.options.content_type.clone().unwrap_or_else(|| {
        crate::http::file_response::mime_from_extension(&file.path)
            .unwrap_or("application/octet-stream")
            .to_owned()
    });
    Ok(Attachment::new(name, content, content_type))
}

/// The subject a notification mail without one is sent with: the part of
/// `name` after its last `::`, split into words at case changes and at
/// `_`, `-`, `.` and whitespace, each word in title case. `InvoicePaid`
/// gives `Invoice Paid`; an acronym stays one word (`HTTPError` gives
/// `Http Error`). Mirrors Laravel's
/// `Str::title(Str::snake(class_basename($notification), ' '))`.
fn subject_from_name(name: &str) -> String {
    let base = name.rsplit("::").next().unwrap_or(name);
    let chars: Vec<char> = base.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if matches!(c, '_' | '-' | '.') || c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        if c.is_uppercase() && !word.is_empty() {
            let previous = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if previous.is_lowercase()
                || previous.is_numeric()
                || (previous.is_uppercase() && next_is_lower)
            {
                words.push(std::mem::take(&mut word));
            }
        }
        word.push(c);
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::subject_from_name;

    #[test]
    fn a_name_becomes_a_title_case_subject() {
        assert_eq!(subject_from_name("InvoicePaid"), "Invoice Paid");
        assert_eq!(
            subject_from_name("App::Billing::InvoicePaid"),
            "Invoice Paid"
        );
        assert_eq!(subject_from_name("invoice_paid"), "Invoice Paid");
        assert_eq!(
            subject_from_name("order-shipped.reminder"),
            "Order Shipped Reminder"
        );
        assert_eq!(subject_from_name("HTTPError"), "Http Error");
        assert_eq!(subject_from_name("Order2Shipped"), "Order2 Shipped");
        assert_eq!(subject_from_name("welcome"), "Welcome");
        assert_eq!(subject_from_name(""), "");
    }
}
