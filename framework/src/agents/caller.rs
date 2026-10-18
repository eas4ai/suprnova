//! Who calls a tool (AGENT-002).
//!
//! Every tool call has a caller: a guest, a signed-in user, or an agent
//! account with the user it is bound to, if any. The handler receives it as
//! a `ToolCaller` argument, and a gate policy reads it with
//! [`ToolCaller::current`], so an application can let agents read and
//! forbid them to delete.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::IpAddr;
use std::sync::Arc;

use crate::auth::Authenticatable;
use crate::error::FrameworkError;

/// The kinds of caller a tool may accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CallerKind {
    /// No credential: a guest in the browser, or an MCP client without a
    /// token.
    Public,
    /// A signed-in user: by browser session, or by a token the user
    /// approved for a client.
    User,
    /// An agent account registered under auth.md, possibly bound to a user.
    Agent,
}

impl CallerKind {
    /// The name `#[tool(callers = [...])]` spells the kind with.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

impl std::fmt::Display for CallerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a tool call reached the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerChannel {
    /// The tool's route, called from a page with the user's session.
    Browser,
    /// The MCP endpoint, by a client presenting a token or none.
    Client,
    /// The tool's route, called by a user signed in some other way than a
    /// browser session: a bearer token, basic auth or a custom guard. Such
    /// a caller holds no scope.
    Direct,
}

/// An agent account registered under auth.md (AGENT-007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistration {
    id: String,
    claimed: bool,
}

impl AgentRegistration {
    /// The registration `id`, unclaimed.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            claimed: false,
        }
    }

    /// The registration's identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether a user has claimed the registration.
    pub fn is_claimed(&self) -> bool {
        self.claimed
    }
}

/// The scopes a caller holds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Scopes {
    /// A browser session holds every scope.
    Every,
    /// A token holds the scopes it was granted.
    Granted(BTreeSet<String>),
}

/// The caller of a tool call.
///
/// A [`CallerResolver`](super::CallerResolver) builds one for each token it
/// accepts; the framework builds the guest of a call without a token and the
/// browser caller of a call through the tool's route.
///
/// ```rust
/// use suprnova::agents::{CallerKind, ToolCaller};
///
/// let caller = ToolCaller::agent("reg_123").claimed(true).scopes(["posts:read"]);
/// assert_eq!(caller.kind(), CallerKind::Agent);
/// assert!(caller.can("posts:read"));
/// assert!(!caller.can("posts:delete"));
/// ```
#[derive(Clone)]
pub struct ToolCaller {
    kind: CallerKind,
    channel: CallerChannel,
    user: Option<Arc<dyn Authenticatable>>,
    registration: Option<AgentRegistration>,
    client: Option<String>,
    scopes: Scopes,
    address: Option<IpAddr>,
}

impl std::fmt::Debug for ToolCaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCaller")
            .field("kind", &self.kind)
            .field("channel", &self.channel)
            .field("user", &self.user_id())
            .field("registration", &self.registration)
            .field("client", &self.client)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

tokio::task_local! {
    /// The caller of the tool call being served.
    static CURRENT: ToolCaller;
}

impl ToolCaller {
    /// A caller with no credential.
    pub fn guest() -> Self {
        Self {
            kind: CallerKind::Public,
            channel: CallerChannel::Client,
            user: None,
            registration: None,
            client: None,
            scopes: Scopes::Granted(BTreeSet::new()),
            address: None,
        }
    }

    /// A user acting through a client, holding no scope until
    /// [`scopes`](Self::scopes) grants some.
    pub fn user(user: Arc<dyn Authenticatable>) -> Self {
        Self {
            kind: CallerKind::User,
            user: Some(user),
            ..Self::guest()
        }
    }

    /// The agent account `registration_id`, unclaimed and bound to no user,
    /// holding no scope until [`scopes`](Self::scopes) grants some.
    pub fn agent(registration_id: impl Into<String>) -> Self {
        Self {
            kind: CallerKind::Agent,
            registration: Some(AgentRegistration::new(registration_id)),
            ..Self::guest()
        }
    }

    /// A request to a tool's route with no user: a guest of the page.
    pub(crate) fn route_guest(address: Option<IpAddr>) -> Self {
        Self {
            channel: CallerChannel::Browser,
            address,
            ..Self::guest()
        }
    }

    /// A request to a tool's route by the user the session guard resolved
    /// from the browser's session. A browser session holds every scope.
    pub(crate) fn session_user(user: Arc<dyn Authenticatable>, address: Option<IpAddr>) -> Self {
        Self {
            kind: CallerKind::User,
            channel: CallerChannel::Browser,
            user: Some(user),
            registration: None,
            client: None,
            scopes: Scopes::Every,
            address,
        }
    }

    /// A request to a tool's route by a user signed in some other way: a
    /// bearer token, basic auth or a custom guard. Not a browser session,
    /// so it holds no scope.
    pub(crate) fn direct_user(user: Arc<dyn Authenticatable>, address: Option<IpAddr>) -> Self {
        Self {
            kind: CallerKind::User,
            channel: CallerChannel::Direct,
            user: Some(user),
            registration: None,
            client: None,
            scopes: Scopes::Granted(BTreeSet::new()),
            address,
        }
    }

    /// Grant the caller `scopes`, replacing any granted before. Takes scope
    /// names or the constants `tool_scopes!` declares.
    pub fn scopes<I>(mut self, scopes: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.scopes = Scopes::Granted(
            scopes
                .into_iter()
                .map(|scope| scope.as_ref().to_string())
                .collect(),
        );
        self
    }

    /// Record the client or provider the call came through: an OAuth client
    /// id, or the issuer of an agent's identity.
    pub fn client(mut self, client: impl Into<String>) -> Self {
        self.client = Some(client.into());
        self
    }

    /// Mark an agent account as claimed by a user, or not. Does nothing for
    /// any other caller.
    pub fn claimed(mut self, claimed: bool) -> Self {
        if let Some(registration) = &mut self.registration {
            registration.claimed = claimed;
        }
        self
    }

    /// Bind an agent account to the user it acts for. The call is signed in
    /// as that user, so `Auth::user()` and `#[authorize]` see them. Does
    /// nothing for any other caller.
    pub fn bound_to(mut self, user: Arc<dyn Authenticatable>) -> Self {
        if self.kind == CallerKind::Agent {
            self.user = Some(user);
        }
        self
    }

    /// The same caller, calling from `address`.
    pub(crate) fn with_address(mut self, address: Option<IpAddr>) -> Self {
        self.address = address;
        self
    }

    /// The same caller, acting as `user`: the record a tool's own guard
    /// loaded for the identifier the resolver returned.
    pub(crate) fn with_acting_user(mut self, user: Arc<dyn Authenticatable>) -> Self {
        self.user = Some(user);
        self
    }

    /// The caller of the tool call being served, or `None` outside one.
    ///
    /// A gate policy reads it to treat callers differently:
    ///
    /// ```rust,ignore
    /// Gate::define::<User, Post>("delete", |_user, _post| {
    ///     !ToolCaller::current().is_some_and(|caller| caller.is_agent())
    /// });
    /// ```
    pub fn current() -> Option<Self> {
        CURRENT.try_with(Clone::clone).ok()
    }

    /// The caller a `ToolCaller` handler argument receives.
    ///
    /// # Errors
    ///
    /// A 500 when the handler runs outside a tool call: its route serves no
    /// `#[tool]`.
    #[doc(hidden)]
    pub fn __for_handler() -> Result<Self, FrameworkError> {
        Self::current().ok_or_else(|| {
            FrameworkError::internal(
                "a `ToolCaller` argument is only available to a handler marked #[tool], \
                 called as a tool",
            )
        })
    }

    /// Run `future` with `self` as the current caller.
    pub(crate) fn scope<F: Future>(self, future: F) -> impl Future<Output = F::Output> {
        CURRENT.scope(self, future)
    }

    /// The caller's kind.
    pub fn kind(&self) -> CallerKind {
        self.kind
    }

    /// How the call reached the application.
    pub fn channel(&self) -> CallerChannel {
        self.channel
    }

    /// Whether the caller presented no credential.
    pub fn is_guest(&self) -> bool {
        self.kind == CallerKind::Public
    }

    /// Whether the caller is a signed-in user.
    pub fn is_user(&self) -> bool {
        self.kind == CallerKind::User
    }

    /// Whether the caller is an agent account.
    pub fn is_agent(&self) -> bool {
        self.kind == CallerKind::Agent
    }

    /// Whether the call came from the browser with the user's session,
    /// rather than from a client acting for the user.
    pub fn via_browser(&self) -> bool {
        self.channel == CallerChannel::Browser
    }

    /// The user the call acts as: a user caller, or the user an agent
    /// account is bound to.
    pub fn acting_user(&self) -> Option<&Arc<dyn Authenticatable>> {
        self.user.as_ref()
    }

    /// The identifier of [`acting_user`](Self::acting_user).
    pub fn user_id(&self) -> Option<String> {
        self.user.as_ref().map(|user| user.get_auth_identifier())
    }

    /// The agent account, for an agent caller.
    pub fn registration(&self) -> Option<&AgentRegistration> {
        self.registration.as_ref()
    }

    /// The client or provider the call came through.
    pub fn client_id(&self) -> Option<&str> {
        self.client.as_deref()
    }

    /// The address the call came from, when known.
    pub fn address(&self) -> Option<IpAddr> {
        self.address
    }

    /// Whether the caller holds `scope`. A browser session holds every
    /// scope.
    pub fn can(&self, scope: impl AsRef<str>) -> bool {
        match &self.scopes {
            Scopes::Every => true,
            Scopes::Granted(granted) => granted.contains(scope.as_ref()),
        }
    }

    /// The scopes a token granted, sorted; `None` for a browser session,
    /// which holds every scope.
    pub fn granted_scopes(&self) -> Option<Vec<&str>> {
        match &self.scopes {
            Scopes::Every => None,
            Scopes::Granted(granted) => Some(granted.iter().map(String::as_str).collect()),
        }
    }

    /// The key the caller's calls are counted under: the user, the agent
    /// account, or the client address of a guest.
    pub(crate) fn limit_key(&self) -> String {
        match self.kind {
            CallerKind::Public => format!(
                "agents:address:{}",
                super::limit::address_bucket(self.address)
            ),
            CallerKind::User => format!("agents:user:{}", self.user_id().unwrap_or_default()),
            CallerKind::Agent => format!(
                "agents:agent:{}",
                self.registration
                    .as_ref()
                    .map(AgentRegistration::id)
                    .unwrap_or_default()
            ),
        }
    }
}
