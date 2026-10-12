//! The documents the kit renders: two layouts under
//! `templates/live/layouts/` and the pages under `templates/live/pages/`
//! that extend them.
//!
//! Every page view holds a [`Frame`], the part both layouts read: the
//! title, the Live bootstrap tags, the signed-in user, the CSRF token of the
//! sign-out form, and the toast an earlier step kept for this page. A
//! page's own fields are the islands its controller mounted and whatever
//! else its template shows. `#[suprnova::view]` checks each template against its
//! struct when the crate compiles, so a name a template reads that the
//! struct lacks fails the build.

use suprnova::live::{LiveBootstrapOptions, LiveDocument};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{Auth, FrameworkError, HttpResponse, Lang, StatusCode};

use crate::flash::Toast;
use crate::models::user::User;

/// The checked filters the templates use: `trusted_html` places the
/// bootstrap tags and the mounted islands, and `live_key` keys the toast.
mod filters {
    pub use suprnova::view::filters::{live_key, trusted_html};
}

/// The application's name, which every tab title ends with.
pub const APP_NAME: &str = "{project_title}";

/// The navigation entry the application layout marks as current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nav {
    /// No entry: the home page, the auth pages and the error page.
    None,
    /// The dashboard.
    Dashboard,
    /// The notes pages.
    Notes,
}

/// The signed-in user as the layouts show them: never the password or a
/// token.
pub struct FrameUser {
    /// The display name.
    pub name: String,
    /// The email address.
    pub email: String,
    /// Up to two initials of the name, for the account menu's avatar.
    pub initials: String,
}

impl From<User> for FrameUser {
    fn from(user: User) -> Self {
        let initials = user
            .name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .take(2)
            .collect();
        Self {
            name: user.name,
            email: user.email,
            initials,
        }
    }
}

/// What both layouts read.
pub struct Frame {
    /// The page's title, which the tab shows before the application's name.
    pub title: String,
    /// The application's name.
    pub app_name: &'static str,
    /// The language of the request, for `<html lang>`.
    pub lang: String,
    /// The public root every link starts from: empty at the host root,
    /// `/billing` behind a proxy that serves the application there.
    pub root: String,
    /// The Live bootstrap tags, or `None` for the error page, which is not
    /// a Live document and mounts nothing.
    pub bootstrap: Option<TrustedHtml>,
    /// The signed-in user, or `None` for a guest.
    pub user: Option<FrameUser>,
    /// The session's CSRF token, for the sign-out form.
    pub csrf: String,
    /// The toast an earlier step kept for this page, shown once.
    pub toast: Option<Toast>,
    /// The navigation entry this page belongs to.
    pub nav: Nav,
}

impl Frame {
    /// The frame of a Live document whose islands are all mounted: it emits
    /// the bootstrap tags, which must come after the last mount, and reads
    /// the user and the toast of this request.
    pub async fn live(
        document: &mut LiveDocument<'_>,
        title: &str,
        nav: Nav,
    ) -> Result<Self, FrameworkError> {
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        let mut frame = Self::plain(title, nav).await?;
        frame.bootstrap = Some(bootstrap.html().clone());
        Ok(frame)
    }

    /// The frame of a document that is not a Live document: the error
    /// page, which renders for requests no Live route prepared.
    pub async fn plain(title: &str, nav: Nav) -> Result<Self, FrameworkError> {
        let user = Auth::user_as::<User>().await?.map(FrameUser::from);
        Ok(Self {
            title: title.to_owned(),
            app_name: APP_NAME,
            lang: Lang::locale().to_string(),
            root: suprnova::url::root(),
            bootstrap: None,
            user,
            csrf: suprnova::csrf_token().unwrap_or_default(),
            toast: Toast::take(),
            nav,
        })
    }

    /// `path` under the public root, for a link.
    pub fn href(&self, path: &str) -> String {
        format!("{}{path}", self.root)
    }

    /// Whether `nav` is the entry this page belongs to.
    pub fn is_current(&self, nav: Nav) -> bool {
        self.nav == nav
    }
}

/// Renders one page of a Live document. `view` names the page's template,
/// which its struct names too.
pub fn render<T: suprnova::view::ViewTemplate>(
    document: LiveDocument<'_>,
    view: &str,
    page: &T,
) -> Result<HttpResponse, FrameworkError> {
    let name = ViewName::parse(view)
        .map_err(|_| FrameworkError::internal(format!("`{view}` is not a view name")))?;
    let intent = DocumentResponseIntent::html(StatusCode::OK)
        .map_err(|_| FrameworkError::internal("the HTML response intent"))?;
    Ok(document.render(name, page, intent, AssetSet::empty())?)
}

/// `/`: the welcome page.
#[suprnova::view(path = "live/pages/home.html")]
pub struct HomePage {
    pub frame: Frame,
    /// The headline, from the `welcome` message of `lang/<locale>/app.ftl`.
    pub welcome: String,
}

/// `/login`: the sign-in form.
#[suprnova::view(path = "live/pages/login.html")]
pub struct LoginPage {
    pub frame: Frame,
    pub form: TrustedHtml,
}

/// `/register`: the registration form.
#[suprnova::view(path = "live/pages/register.html")]
pub struct RegisterPage {
    pub frame: Frame,
    pub form: TrustedHtml,
}

/// `/forgot-password`: the form that asks for a reset link.
#[suprnova::view(path = "live/pages/forgot-password.html")]
pub struct ForgotPasswordPage {
    pub frame: Frame,
    pub form: TrustedHtml,
}

/// `/reset-password`: the form that takes the new password.
#[suprnova::view(path = "live/pages/reset-password.html")]
pub struct ResetPasswordPage {
    pub frame: Frame,
    pub form: TrustedHtml,
}

/// `/verify-email`: the notice and its resend button.
#[suprnova::view(path = "live/pages/verify-email.html")]
pub struct VerifyEmailPage {
    pub frame: Frame,
    pub notice: TrustedHtml,
}

/// `/dashboard`: the welcome, the display-name form and the note counts.
#[suprnova::view(path = "live/pages/dashboard.html")]
pub struct DashboardPage {
    pub frame: Frame,
    pub profile: TrustedHtml,
    pub stats: TrustedHtml,
}

/// `/notes`: the note form and the searchable list.
#[suprnova::view(path = "live/pages/notes-index.html")]
pub struct NotesIndexPage {
    pub frame: Frame,
    pub form: TrustedHtml,
    pub list: TrustedHtml,
}

/// A whole note, as `/notes/{id}` shows it.
pub struct NoteDetail {
    pub title: String,
    pub body: Option<String>,
    /// When the note was written, as RFC 3339 for the `datetime` attribute.
    pub created_at: String,
    /// When the note was written, as the page prints it.
    pub created_on: String,
}

/// `/notes/{id}`: one of the user's notes.
#[suprnova::view(path = "live/pages/notes-show.html")]
pub struct NoteShowPage {
    pub frame: Frame,
    pub note: NoteDetail,
}

/// The page every framework error response a browser asked for renders
/// as. See `controllers::errors`.
#[suprnova::view(path = "live/pages/error.html")]
pub struct ErrorDocument {
    pub frame: Frame,
    pub status: u16,
    pub message: String,
    /// The request's id, which the log lines of the failure carry.
    pub request_id: Option<String>,
    /// The label before the request's id, from `error-reference`.
    pub reference: String,
    /// The link home, from `error-go-home`.
    pub go_home: String,
}
