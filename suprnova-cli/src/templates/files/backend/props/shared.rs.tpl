//! The props every page receives: `auth`, the signed-in user or none.
//!
//! The HTTP stack shares `auth` with `App::inertia_share_lazy` (see
//! `bootstrap::register_http_stack`), so [`Auth::current`] runs for each
//! response that sends it, with that request's session. A layout reads
//! `usePage().props.auth.user` to show the user's name and the sign-out
//! link, or the sign-in and register links when it is `null`. A partial
//! reload that does not name `auth`, such as the dashboard's poll of
//! `stats`, does not run it.

use suprnova::{FrameworkError, InertiaProps};

use crate::models::user::User;

/// The shared props, as `suprnova generate-types` adds them to the
/// generated `SharedProps` beside the framework's own `root`. The marker
/// only types them; the share in `bootstrap.rs` sends them.
#[derive(InertiaProps)]
#[inertia_props(shared)]
pub struct SharedData {
    pub auth: Auth,
}

/// Who is signed in.
#[derive(InertiaProps)]
pub struct Auth {
    /// The signed-in user, or `None` for a guest.
    pub user: Option<UserInfo>,
}

impl Auth {
    /// The signed-in user of the request being answered, or none.
    pub async fn current() -> Result<Self, FrameworkError> {
        // The registered user provider resolves the typed `User` from the
        // session id; `user_as` downcasts the `Authenticatable` for us.
        let user = suprnova::Auth::user_as::<User>().await?;
        Ok(Self {
            user: user.map(UserInfo::from),
        })
    }
}

/// A user as the pages show them: never the password or a token.
#[derive(InertiaProps)]
pub struct UserInfo {
    pub id: u64,
    pub name: String,
    pub email: String,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            name: user.name,
            email: user.email,
        }
    }
}
