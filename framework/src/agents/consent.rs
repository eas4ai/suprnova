//! The approval and claim halves of an application's own consent pages
//! (AGENT-005, AGENT-009, AGENT-017), and of the framework's default ones.
//!
//! A starter kit renders the approval page and the claim page in its own
//! frontend. It reads what the user decides on, and records the decision,
//! through [`AgentApprovals`] and [`AgentClaims`]. The framework's default
//! pages call the same two types, so the kits and the default pages read
//! the engines' answers through one mapping.
//!
//! Both types act for the user the default guard signed in, and only for
//! that user: another user's authorization or claim reads as ended or as
//! someone else's, never as theirs. Neither checks a CSRF token or sets a
//! header. The route that records a decision runs the CSRF middleware, and
//! every consent route runs [`ConsentPageHeaders`](crate::agents::ConsentPageHeaders).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use magnetar::agent_auth::SignedInUser;
use magnetar::authorization_server::{
    AuthorizationServer, DecisionOutcome, PendingAuthorizationView,
};
use magnetar::plugin::WireResponse;
use serde::Serialize;
use serde_json::Value;

use super::oauth::GrantScope;
use crate::error::FrameworkError;
use crate::http::Request;
use crate::magnetar_integration::plugin_host::signed_in_session;

/// A pending authorization, as an application's own approval page shows it
/// (AGENT-005): the client, the client's host, where the user returns and
/// every scope the client asks for.
///
/// It deliberately carries no URL the client chose besides the redirect
/// URI the user returns to: no home page and no logo. A page that loaded
/// the logo would tell the client who looked at the request, and a link
/// to a page the client chose is a phishing aid on a consent screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingApproval {
    /// The pending authorization's id, which the page posts the decision
    /// with.
    pub id: String,
    /// The client's name. The client chose it, so a page shows it beside
    /// [`client_host`](Self::client_host) and isolates its direction
    /// (`<bdi>`), so whatever reordering it carries stays inside it.
    pub client_name: String,
    /// The host the client cannot choose freely: its metadata document's
    /// host, or its redirect URI's.
    pub client_host: String,
    /// Where the user returns after deciding, a URI the client registered.
    /// A page shows it, isolated as the name is.
    pub redirect_uri: String,
    /// Every scope the client asks for, with the words the user reads.
    pub scopes: Vec<GrantScope>,
    /// When the pending authorization lapses.
    pub expires_at: DateTime<Utc>,
}

impl PendingApproval {
    fn from_view(view: PendingAuthorizationView) -> Self {
        Self {
            id: view.id,
            client_name: view.client_name,
            client_host: view.client_host,
            redirect_uri: view.redirect_uri,
            scopes: view
                .scopes
                .into_iter()
                .map(|scope| GrantScope {
                    name: scope.name,
                    description: scope.description,
                })
                .collect(),
            expires_at: view.expires_at,
        }
    }
}

/// What an approval page finds behind an authorization id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalLookup {
    /// The signed-in user's pending authorization: show it.
    Pending(PendingApproval),
    /// No such pending authorization for this user: it lapsed, was
    /// already decided, or belongs to another user. The user starts again
    /// from the client.
    Ended,
    /// No user is signed in: send them to sign in and back
    /// ([`Redirect::guest`](crate::Redirect::guest)).
    SignIn,
}

/// What recording the user's decision comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Send the browser here: the client's registered redirect URI, with
    /// the code or `access_denied`, the client's `state` and `iss`. It
    /// usually lies on another origin. A page that posted a native form
    /// under a `form-action 'self'` policy cannot follow a redirect there,
    /// so it leaves by a location visit or a page of its own.
    Redirect(String),
    /// No such pending authorization for this user: it lapsed, was already
    /// decided, or belongs to another user.
    Ended,
    /// No user is signed in, or the session predates the user's last
    /// password change or "sign out everywhere": sign in again.
    SignIn,
}

/// The approval half of an application's own approval page (AGENT-005):
/// the pending authorization the page shows, and the user's decision on
/// it.
///
/// Taken from the request with [`of`](Self::of) before its body is read,
/// so a `POST` handler can read its form request after it:
///
/// ```rust,ignore
/// use suprnova::agents::{AgentApprovals, ApprovalDecision};
///
/// #[handler]
/// pub async fn decide(request: Request) -> Response {
///     let approvals = AgentApprovals::of(&request)?;
///     let form = <DecisionRequest as FromRequest>::from_request(request).await?;
///     match approvals.decide(&form.authorization, form.decision == "approve").await? {
///         ApprovalDecision::Redirect(url) => Ok(Inertia::location(url)),
///         ApprovalDecision::Ended => Err(FrameworkError::domain("This request has ended.", 404).into()),
///         ApprovalDecision::SignIn => Ok(Redirect::to("/login").into()),
///     }
/// }
/// ```
///
/// The route that calls [`decide`](Self::decide) must check the CSRF token
/// (the global CSRF middleware does), and every approval route must run
/// [`ConsentPageHeaders`](crate::agents::ConsentPageHeaders), so the page
/// is never framed, cached or named in a referrer.
#[derive(Clone)]
pub struct AgentApprovals {
    server: Arc<AuthorizationServer>,
}

impl std::fmt::Debug for AgentApprovals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentApprovals").finish_non_exhaustive()
    }
}

impl AgentApprovals {
    /// The approvals of the OAuth server the request's router mounted.
    ///
    /// # Errors
    ///
    /// 403 for a tool call, one the MCP endpoint dispatched or one a page
    /// made with the [`TOOL_CALL_HEADER`](crate::agents::TOOL_CALL_HEADER):
    /// consent comes from the person, never an agent acting as them. 500
    /// when the router that served the request mounts no OAuth server.
    pub fn of(request: &Request) -> Result<Self, FrameworkError> {
        refuse_agents(request)?;
        super::oauth::mounted_server(request).map(Self::new)
    }

    /// The approvals of `server`, for the framework's default page.
    pub(crate) fn new(server: Arc<AuthorizationServer>) -> Self {
        Self { server }
    }

    /// The signed-in user's pending authorization `id`. A guest gets
    /// [`ApprovalLookup::SignIn`] before anything about the id is read.
    ///
    /// # Errors
    ///
    /// 503 when the authorization server's store cannot answer, and the
    /// guard's error when the signed-in user cannot be read.
    pub async fn pending(&self, id: &str) -> Result<ApprovalLookup, FrameworkError> {
        let Some(session) = signed_in_session().await? else {
            return Ok(ApprovalLookup::SignIn);
        };
        if id.is_empty() {
            return Ok(ApprovalLookup::Ended);
        }
        Ok(
            match self
                .server
                .pending_authorization(id, &session)
                .await
                .map_err(approvals_unavailable)?
            {
                Some(view) => ApprovalLookup::Pending(PendingApproval::from_view(view)),
                None => ApprovalLookup::Ended,
            },
        )
    }

    /// Record the signed-in user's decision on the pending authorization
    /// `id`: `approve` issues a one-time code to the client, otherwise the
    /// client gets `access_denied`. Either way the authorization is spent,
    /// so a second decision on it is [`ApprovalDecision::Ended`].
    ///
    /// # Errors
    ///
    /// As [`pending`](Self::pending).
    pub async fn decide(
        &self,
        id: &str,
        approve: bool,
    ) -> Result<ApprovalDecision, FrameworkError> {
        let Some(session) = signed_in_session().await? else {
            return Ok(ApprovalDecision::SignIn);
        };
        if id.is_empty() {
            return Ok(ApprovalDecision::Ended);
        }
        Ok(
            match self
                .server
                .decide(id, &session, approve)
                .await
                .map_err(approvals_unavailable)?
            {
                DecisionOutcome::Redirect(url) => ApprovalDecision::Redirect(url),
                DecisionOutcome::NotFound => ApprovalDecision::Ended,
                DecisionOutcome::StaleSession => ApprovalDecision::SignIn,
            },
        )
    }
}

/// Refuse a request an agent makes: a tool call the MCP endpoint
/// dispatched, which carries the agent mark, or one from the browser,
/// which carries the [`TOOL_CALL_HEADER`](super::TOOL_CALL_HEADER).
///
/// A tool call runs as the user its caller acts for, so a route an
/// application declared a tool by mistake would otherwise let an agent
/// approve its own authorization or claim itself. Consent comes from the
/// person, so both halves refuse here, before anything is read, whatever
/// the route.
fn refuse_agents(request: &Request) -> Result<(), FrameworkError> {
    if request.is_agent_traffic() || request.header(super::TOOL_CALL_HEADER).is_some() {
        return Err(FrameworkError::domain(
            "Consent must come from the person: an agent's tool call cannot approve, deny or \
             claim.",
            403,
        ));
    }
    Ok(())
}

fn approvals_unavailable(error: magnetar::Error) -> FrameworkError {
    tracing::error!(%error, "the authorization server's store failed");
    FrameworkError::domain("the authorization server is unavailable", 503)
}

/// A pending claim of an agent registration, as an application's own
/// claim page shows it (AGENT-008, AGENT-009).
///
/// Every text in it but the scopes' words was chosen by someone other than
/// the application: the user, the agent or the provider. A page isolates
/// each (`<bdi>`). It has no field for a name a provider sent: the
/// provider is named only by the application's trust list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingClaim {
    /// How the agent registered: `anonymous`, `service_auth` or
    /// `identity_assertion`.
    pub registration_type: String,
    /// The trust list's display name of the provider the agent signed in
    /// through, never one the provider or the agent sent.
    pub provider: Option<String>,
    /// The signed-in user's email address.
    pub signed_in_as: String,
    /// The address the agent named, which the claim is bound to.
    pub email: Option<String>,
    /// What the agent will be allowed once claimed, with the words the user
    /// reads.
    pub scopes: Vec<GrantScope>,
    /// How long the code works from now, in seconds.
    pub expires_in_seconds: u64,
    /// How many more wrong codes the attempt takes.
    pub attempts_remaining: u64,
    /// No agent has acted for this account before: a page warns the user to
    /// allow it only if they asked an agent to connect.
    pub first_time_account: bool,
    /// No agent from this provider has acted for the user before.
    pub first_time_provider: bool,
}

impl PendingClaim {
    /// The pending claim from the engine's JSON answer.
    fn from_body(body: &Value) -> Self {
        let text = |name: &str| body.get(name).and_then(Value::as_str).map(str::to_owned);
        let number = |name: &str| body.get(name).and_then(Value::as_u64).unwrap_or(0);
        let advisories: Vec<&str> = body
            .get("advisories")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        Self {
            registration_type: text("registration_type").unwrap_or_default(),
            provider: body
                .get("provider")
                .and_then(|provider| provider.get("display_name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            signed_in_as: text("signed_in_as").unwrap_or_default(),
            email: text("email"),
            scopes: body
                .get("scopes")
                .and_then(Value::as_array)
                .map(|scopes| {
                    scopes
                        .iter()
                        .map(|scope| {
                            let field = |name: &str| {
                                scope
                                    .get(name)
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned()
                            };
                            GrantScope {
                                name: field("name"),
                                description: field("description"),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            expires_in_seconds: number("expires_in"),
            attempts_remaining: number("attempts_remaining"),
            first_time_account: advisories.contains(&"first_time_account"),
            first_time_provider: advisories.contains(&"first_time_provider"),
        }
    }
}

/// Why a claim page cannot show or confirm a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimRefusal {
    /// No user is signed in, or the session predates the user's last
    /// password change: send them to sign in and back.
    SignIn,
    /// The claim is bound to another account, or to the signed-in user's
    /// address before the application verified it. The page says to sign
    /// in with the address the agent named, and shows nothing of the claim.
    OtherAccount,
    /// No such claim attempt, or it was already confirmed.
    NotFound,
    /// The code expired, or was entered wrongly too often: the agent starts
    /// a new attempt.
    Ended,
    /// The request is not one the engine can read.
    Invalid,
}

/// What a claim page finds behind an attempt token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimLookup {
    /// The claim the signed-in user may confirm: show it, with a form that
    /// takes the code.
    Pending(PendingClaim),
    /// The claim cannot be shown.
    Refused(ClaimRefusal),
}

/// What confirming a claim with a code comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimConfirmation {
    /// The agent registration is the signed-in user's now. The agent's next
    /// poll gets its tokens.
    Claimed {
        /// The registration's id, which an application keeps on what the
        /// agent creates.
        registration_id: String,
    },
    /// The code is wrong: show the claim again with the mistake named.
    WrongCode {
        /// How many more wrong codes the attempt takes.
        attempts_remaining: u64,
    },
    /// The claim cannot be confirmed.
    Refused(ClaimRefusal),
}

/// The claim half of an application's own claim page (AGENT-009): the
/// pending claim the page shows, and its confirmation with the code the
/// user types.
///
/// Taken from the request with [`of`](Self::of) before its body is read,
/// so a `POST` handler can read its form request after it. It keeps the
/// request's client address for the claim's event and audit record.
///
/// The route that calls [`confirm`](Self::confirm) must check the CSRF
/// token (the global CSRF middleware does), and every claim route must run
/// [`ConsentPageHeaders`](crate::agents::ConsentPageHeaders): the page's
/// address carries the attempt token, which must not reach a cache or a
/// referrer.
#[derive(Clone)]
pub struct AgentClaims {
    engine: Arc<magnetar::agent_auth::AgentAuth>,
    address: Option<String>,
}

impl std::fmt::Debug for AgentClaims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentClaims").finish_non_exhaustive()
    }
}

impl AgentClaims {
    /// The claims of the agent auth the request's router mounted, for the
    /// request's client.
    ///
    /// # Errors
    ///
    /// 403 for a tool call, as [`AgentApprovals::of`] refuses one. 500
    /// when the router that served the request mounts no agent auth.
    pub fn of(request: &Request) -> Result<Self, FrameworkError> {
        refuse_agents(request)?;
        let agent = super::oauth::mounted_agent_auth(request)?;
        Ok(Self::new(Arc::clone(&agent.engine), request.ip()))
    }

    /// The claims of `engine` for the client at `address`, for the
    /// framework's default page.
    pub(crate) fn new(
        engine: Arc<magnetar::agent_auth::AgentAuth>,
        address: Option<String>,
    ) -> Self {
        Self { engine, address }
    }

    /// The claim behind `attempt_token`, as the signed-in user may see it.
    /// A guest is refused before anything about the token is read, and a
    /// user signed in as someone else learns only that it is not theirs.
    ///
    /// # Errors
    ///
    /// 503 when agent auth's store cannot answer, and the guard's error
    /// when the signed-in user cannot be read.
    pub async fn pending(&self, attempt_token: &str) -> Result<ClaimLookup, FrameworkError> {
        let user = signed_in_user().await?;
        let answer = Answer::of(
            self.engine
                .pending_claim(attempt_token, user.as_ref())
                .await,
        );
        Ok(match answer.status {
            200 => ClaimLookup::Pending(PendingClaim::from_body(&answer.body)),
            status => ClaimLookup::Refused(refusal(status)?),
        })
    }

    /// Confirm the claim behind `attempt_token` with the code the signed-in
    /// user typed. A wrong code counts against the attempt.
    ///
    /// # Errors
    ///
    /// As [`pending`](Self::pending).
    pub async fn confirm(
        &self,
        attempt_token: &str,
        user_code: &str,
    ) -> Result<ClaimConfirmation, FrameworkError> {
        let user = signed_in_user().await?;
        let answer = Answer::of(
            self.engine
                .confirm_claim(
                    attempt_token,
                    user_code,
                    user.as_ref(),
                    self.address.as_deref(),
                )
                .await,
        );
        Ok(match answer.status {
            200 => ClaimConfirmation::Claimed {
                registration_id: answer
                    .body
                    .get("registration_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            },
            400 if answer.error() == Some("invalid_user_code") => ClaimConfirmation::WrongCode {
                attempts_remaining: answer
                    .body
                    .get("attempts_remaining")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            },
            status => ClaimConfirmation::Refused(refusal(status)?),
        })
    }
}

/// The signed-in user as the engine compares them with a claim.
async fn signed_in_user() -> Result<Option<SignedInUser>, FrameworkError> {
    Ok(signed_in_session().await?.map(|session| SignedInUser {
        user_id: session.user_id().to_owned(),
        auth_epoch: Some(session.auth_epoch()),
    }))
}

/// The refusal an engine status stands for. 403 covers an unverified
/// address too: the engine answers both the same, so a page cannot tell
/// another user whether the address is registered.
fn refusal(status: u16) -> Result<ClaimRefusal, FrameworkError> {
    match status {
        401 => Ok(ClaimRefusal::SignIn),
        403 => Ok(ClaimRefusal::OtherAccount),
        404 => Ok(ClaimRefusal::NotFound),
        410 => Ok(ClaimRefusal::Ended),
        400 => Ok(ClaimRefusal::Invalid),
        status => {
            tracing::error!(status, "agent auth answered a claim with a failure");
            Err(FrameworkError::domain(
                "agent accounts are unavailable",
                503,
            ))
        }
    }
}

/// The engine's answer, read as status and JSON body.
struct Answer {
    status: u16,
    body: Value,
}

impl Answer {
    fn of(response: WireResponse) -> Self {
        let effects = response.into_effects();
        Self {
            status: effects.status,
            body: effects.body.unwrap_or(Value::Null),
        }
    }

    fn error(&self) -> Option<&str> {
        self.body.get("error").and_then(Value::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_008_pending_claim_reads_only_the_trust_list_name() {
        let claim = PendingClaim::from_body(&json!({
            "registration_type": "identity_assertion",
            "signed_in_as": "alice@corp.test",
            "email": "alice@corp.test",
            "provider": { "display_name": "Acme Agents" },
            "name": "Provider-Chosen Name",
            "scopes": [{ "name": "posts:read", "description": "Read your posts" }],
            "expires_in": 540,
            "attempts_remaining": 4,
            "advisories": ["first_time_provider"],
        }));
        assert_eq!(claim.provider.as_deref(), Some("Acme Agents"));
        assert_eq!(claim.registration_type, "identity_assertion");
        assert_eq!(claim.signed_in_as, "alice@corp.test");
        assert_eq!(claim.expires_in_seconds, 540);
        assert_eq!(claim.attempts_remaining, 4);
        assert!(claim.first_time_provider);
        assert!(!claim.first_time_account);
        assert_eq!(claim.scopes[0].name, "posts:read");
        let shown = serde_json::to_string(&claim).unwrap_or_default();
        assert!(!shown.contains("Provider-Chosen Name"), "{shown}");
    }

    #[test]
    fn agent_017_claim_refusals_follow_the_engine_statuses() {
        assert_eq!(refusal(401).ok(), Some(ClaimRefusal::SignIn));
        assert_eq!(refusal(403).ok(), Some(ClaimRefusal::OtherAccount));
        assert_eq!(refusal(404).ok(), Some(ClaimRefusal::NotFound));
        assert_eq!(refusal(410).ok(), Some(ClaimRefusal::Ended));
        assert_eq!(refusal(400).ok(), Some(ClaimRefusal::Invalid));
        for status in [500, 502, 503] {
            let error = refusal(status).err().map(|error| error.status_code());
            assert_eq!(error, Some(503), "{status}");
        }
    }
}
