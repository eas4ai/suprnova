//! Agents: the approval page an MCP client sends the user to, the claim
//! page an agent sends the user to, and the user's connected agents.
//!
//! `src/agents.rs` serves the OAuth server and agent auth with these pages
//! in place of the framework's defaults. The approval and claim pages read
//! and record what the user decides through `AgentApprovals` and
//! `AgentClaims`, and the connected-agents page lists and revokes through
//! `ConnectedAgents`. Each acts for the signed-in user only: another user's
//! request, claim or grant reads as ended or not found.
//!
//! Every route here runs `ConsentPageHeaders` (see `routes.rs`), so no
//! other site can frame these pages and no cache or referrer keeps the
//! authorization id or the claim's attempt token. The global CSRF
//! middleware checks every form. No route here offers a tool, and no page
//! registers one: consent comes from the person, never from an agent
//! acting as them.
//!
//! A guest who opens the approval or the claim page signs in and comes back
//! (`Redirect::guest`). A decision leaves for the client's redirect URI by
//! a location visit (`Inertia::location`): the browser navigates there
//! itself, which the page's `form-action 'self'` policy does not govern,
//! while a redirect answering the form's post would be blocked. The pages
//! send no referrer, so every answer here names the page it returns to
//! rather than going back.

use serde::Deserialize;
use suprnova::agents::{
    AgentAccount, ApprovalDecision, ApprovalLookup, ClaimConfirmation, ClaimLookup, ClaimRefusal,
    ClientGrant, GrantScope, PendingApproval, PendingClaim,
};
use suprnova::{
    AgentApprovals, AgentClaims, ConnectedAgents, FormRequest, FrameworkError, FromRequest,
    Inertia, InertiaProps, Redirect, Request, Response, Validate, ValidationErrors, handler,
    inertia_response,
};

use crate::props::flash::Toast;

/// Where a guest signs in before coming back.
const SIGN_IN: &str = "/login";

/// Where the connected agents are listed.
const CONNECTED: &str = "/connected-agents";

/// One permission as a consent page shows it: the words the user reads and
/// the name a token carries.
#[derive(InertiaProps)]
pub struct ScopeView {
    pub name: String,
    pub description: String,
}

impl From<GrantScope> for ScopeView {
    fn from(scope: GrantScope) -> Self {
        Self {
            name: scope.name,
            description: scope.description,
        }
    }
}

fn scopes(scopes: Vec<GrantScope>) -> Vec<ScopeView> {
    scopes.into_iter().map(ScopeView::from).collect()
}

/// A pending authorization, as `Agents/Approve` shows it. The client chose
/// its name, and registered the redirect URI: the page isolates both, and
/// the host, in `<bdi>`. It carries no URL the client chose for a logo or a
/// home page, which a consent page must not load or link.
#[derive(InertiaProps)]
pub struct ApprovalView {
    pub id: String,
    pub client_name: String,
    pub client_host: String,
    pub redirect_uri: String,
    pub scopes: Vec<ScopeView>,
    pub expires_at: String,
}

impl From<PendingApproval> for ApprovalView {
    fn from(approval: PendingApproval) -> Self {
        Self {
            id: approval.id,
            client_name: approval.client_name,
            client_host: approval.client_host,
            redirect_uri: approval.redirect_uri,
            scopes: scopes(approval.scopes),
            expires_at: approval.expires_at.to_rfc3339(),
        }
    }
}

/// The `Agents/Approve` page's props.
#[derive(InertiaProps)]
pub struct ApprovalProps {
    pub approval: ApprovalView,
}

/// A pending claim, as `Agents/Claim` shows it. The provider is the trust
/// list's name, never one the provider or the agent sent. The page
/// isolates the addresses in `<bdi>`: the user and the agent chose them.
#[derive(InertiaProps)]
pub struct ClaimView {
    pub registration_type: String,
    pub provider: Option<String>,
    pub signed_in_as: String,
    pub email: Option<String>,
    pub scopes: Vec<ScopeView>,
    pub expires_in_seconds: u64,
    pub attempts_remaining: u64,
    pub first_time_account: bool,
    pub first_time_provider: bool,
}

impl From<PendingClaim> for ClaimView {
    fn from(claim: PendingClaim) -> Self {
        Self {
            registration_type: claim.registration_type,
            provider: claim.provider,
            signed_in_as: claim.signed_in_as,
            email: claim.email,
            scopes: scopes(claim.scopes),
            expires_in_seconds: claim.expires_in_seconds,
            attempts_remaining: claim.attempts_remaining,
            first_time_account: claim.first_time_account,
            first_time_provider: claim.first_time_provider,
        }
    }
}

/// The `Agents/Claim` page's props: the claim, and the attempt token its
/// form posts back.
#[derive(InertiaProps)]
pub struct ClaimProps {
    pub claim: ClaimView,
    pub claim_attempt_token: String,
}

/// A client the user approved, as `Agents/Connected` lists it.
#[derive(InertiaProps)]
pub struct ClientView {
    pub id: String,
    pub client_name: String,
    pub client_host: String,
    pub scopes: Vec<ScopeView>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl From<ClientGrant> for ClientView {
    fn from(grant: ClientGrant) -> Self {
        Self {
            id: grant.id,
            client_name: grant.client_name,
            client_host: grant.client_host,
            scopes: scopes(grant.scopes),
            created_at: grant.created_at.to_rfc3339(),
            last_used_at: grant.last_used_at.map(|used| used.to_rfc3339()),
        }
    }
}

/// An agent registration bound to the user, as `Agents/Connected` lists it.
#[derive(InertiaProps)]
pub struct AgentView {
    pub id: String,
    pub registration_type: String,
    pub provider: Option<String>,
    pub scopes: Vec<ScopeView>,
    pub created_at: String,
    pub claimed_at: Option<String>,
}

impl From<AgentAccount> for AgentView {
    fn from(agent: AgentAccount) -> Self {
        Self {
            id: agent.id,
            registration_type: agent.registration_type,
            provider: agent.provider,
            scopes: scopes(agent.scopes),
            created_at: agent.created_at.to_rfc3339(),
            claimed_at: agent.claimed_at.map(|claimed| claimed.to_rfc3339()),
        }
    }
}

/// The `Agents/Connected` page's props: the clients and the agents that act
/// for the user, newest first.
#[derive(InertiaProps)]
pub struct ConnectedAgentsProps {
    pub clients: Vec<ClientView>,
    pub agents: Vec<AgentView>,
}

/// The approval page's two forms: which authorization, and the decision.
#[derive(Deserialize, Validate)]
pub struct ApprovalDecisionRequest {
    #[validate(length(min = 1, max = 128, message = "Start again from the app that asked."))]
    pub authorization: String,
    pub decision: String,
}

impl FormRequest for ApprovalDecisionRequest {
    /// A decision is `approve` or `deny`, nothing else: a form that sends
    /// another word decides nothing.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.decision != "approve" && self.decision != "deny" {
            let mut errs = ValidationErrors::new();
            errs.add("decision", "Choose Allow or Deny.");
            return Err(errs);
        }
        Ok(())
    }
}

/// The claim page's form: the attempt the link named, and the code the
/// agent gave the user.
#[derive(Deserialize, Validate)]
pub struct ClaimCodeRequest {
    #[validate(length(
        min = 1,
        max = 512,
        message = "Open the link the agent gave you again."
    ))]
    pub claim_attempt_token: String,
    #[validate(length(min = 1, max = 16, message = "Enter the code the agent gave you."))]
    pub user_code: String,
}

impl FormRequest for ClaimCodeRequest {}

/// The form of a consent page's `POST`. Every consent page sends no
/// referrer, so a validation redirect could not find the page the form came
/// from, and would land on whatever page the session last recorded. A
/// consent form's fields are hidden, or bounded in the page, so a form that
/// fails its rules was not one the page sent: it gets an error page that
/// says what to do instead.
async fn consent_form<T: FromRequest>(req: Request, message: &str) -> Result<T, FrameworkError> {
    T::from_request(req).await.map_err(|error| match error {
        FrameworkError::Validation(_) | FrameworkError::ValidationRedirect { .. } => {
            FrameworkError::domain(message, 400)
        }
        error => error,
    })
}

/// The answer for an authorization that is not the signed-in user's to
/// decide: it lapsed, was answered, or is someone else's.
fn approval_ended() -> FrameworkError {
    FrameworkError::domain(
        "This request has ended. Start again from the app that asked.",
        404,
    )
}

/// The answer for a claim the page cannot show or confirm. The handlers
/// send a guest, or a session that no longer speaks for the user, to sign
/// in before they get here.
fn claim_refused(refusal: ClaimRefusal) -> FrameworkError {
    match refusal {
        // The facade's one answer for both: the claim names another
        // account, or the signed-in user's address before it is verified.
        ClaimRefusal::OtherAccount => FrameworkError::domain(
            "This claim is for another account, or your address is not verified yet. Sign in \
             with the address the agent named.",
            403,
        ),
        ClaimRefusal::NotFound => FrameworkError::domain(
            "This link does not name a claim. Ask the agent for a new one.",
            404,
        ),
        ClaimRefusal::Ended => FrameworkError::domain(
            "This code can no longer be used. Ask the agent for a new one.",
            410,
        ),
        ClaimRefusal::Invalid | ClaimRefusal::SignIn => {
            FrameworkError::domain("Open the link the agent gave you again.", 400)
        }
    }
}

/// `GET /oauth/approve?authorization=...` - the client, its host, where the
/// user returns and every permission it asks for, with Allow and Deny.
#[handler]
pub async fn approval(req: Request) -> Response {
    let id = req.query_param("authorization").filter(|id| !id.is_empty());
    // The facade answers a guest before it reads the id, so nothing about
    // the request shows before the user signs in.
    let lookup = AgentApprovals::of(&req)?
        .pending(id.as_deref().unwrap_or_default())
        .await?;
    match (lookup, id) {
        (ApprovalLookup::SignIn, _) => Redirect::guest(&req, SIGN_IN).into(),
        (_, None) => Err(FrameworkError::domain(
            "This link does not name an authorization request.",
            400,
        )
        .into()),
        (ApprovalLookup::Ended, Some(_)) => Err(approval_ended().into()),
        (ApprovalLookup::Pending(approval), Some(_)) => inertia_response!(
            &req,
            "Agents/Approve",
            ApprovalProps {
                approval: approval.into(),
            }
        ),
    }
}

/// `POST /oauth/approve` - record the decision and send the browser to the
/// client's redirect URI, with the code or `access_denied`.
#[handler]
pub async fn decide(req: Request) -> Response {
    // Taken before the form reads the body.
    let approvals = AgentApprovals::of(&req)?;
    let form: ApprovalDecisionRequest =
        consent_form(req, "Start again from the app that asked.").await?;
    match approvals
        .decide(&form.authorization, form.decision == "approve")
        .await?
    {
        ApprovalDecision::Redirect(url) => {
            // Inertia's `clearHistory` drops the key an application that
            // turns on `encryptHistory` encrypts its history with, so the
            // approval page no longer reads back from the browser's history.
            // The kit does not encrypt its history, so here the call changes
            // nothing; it is kept for an application that does. The
            // authorization id it would hide is spent by now.
            Inertia::clear_history();
            Ok(Inertia::location(url))
        }
        ApprovalDecision::Ended => Err(approval_ended().into()),
        // A form post cannot be replayed after sign-in: the user signs in
        // and opens the client's link again.
        ApprovalDecision::SignIn => Redirect::to(SIGN_IN).into(),
    }
}

/// `GET /agent/claim?claim_attempt_token=...` - who the agent will act as,
/// with which permissions, and the form that takes the code.
#[handler]
pub async fn claim(req: Request) -> Response {
    let token = req
        .query_param("claim_attempt_token")
        .filter(|token| !token.is_empty());
    // The engine refuses a guest before it reads the token.
    let lookup = AgentClaims::of(&req)?
        .pending(token.as_deref().unwrap_or_default())
        .await?;
    match (lookup, token) {
        (ClaimLookup::Refused(ClaimRefusal::SignIn), _) => Redirect::guest(&req, SIGN_IN).into(),
        (_, None) => Err(FrameworkError::domain(
            "This link does not name a claim. Ask the agent for a new one.",
            400,
        )
        .into()),
        (ClaimLookup::Pending(claim), Some(token)) => inertia_response!(
            &req,
            "Agents/Claim",
            ClaimProps {
                claim: claim.into(),
                claim_attempt_token: token,
            }
        ),
        (ClaimLookup::Refused(refusal), Some(_)) => Err(claim_refused(refusal).into()),
    }
}

/// `POST /agent/claim` - confirm the claim with the code the user typed. A
/// wrong code comes back to the page as the code field's error, with the
/// attempts left.
#[handler]
pub async fn confirm(req: Request) -> Response {
    // Taken before the form reads the body.
    let claims = AgentClaims::of(&req)?;
    let form: ClaimCodeRequest =
        consent_form(req, "Open the link the agent gave you again.").await?;
    match claims
        .confirm(&form.claim_attempt_token, &form.user_code)
        .await?
    {
        ClaimConfirmation::Claimed { .. } => {
            // As in `decide`: only an application that encrypts its history
            // gains from this. The attempt token is in the claim page's
            // address anyway, and it is spent by now.
            Inertia::clear_history();
            Inertia::flash("toast", Toast::success("The agent is connected."))?;
            Redirect::to(CONNECTED).status(303).into()
        }
        // Back to the page by name: it sends no referrer to go back by.
        ClaimConfirmation::WrongCode { attempts_remaining } => Redirect::route("agents.claim")
            .query("claim_attempt_token", form.claim_attempt_token)
            .status(303)
            .with_errors([(
                "user_code",
                format!("That code is wrong. {attempts_remaining} attempts left."),
            )])
            .into(),
        // A form post cannot be replayed after sign-in: the user signs in
        // and opens the agent's link again.
        ClaimConfirmation::Refused(ClaimRefusal::SignIn) => Redirect::to(SIGN_IN).into(),
        ClaimConfirmation::Refused(refusal) => Err(claim_refused(refusal).into()),
    }
}

/// `GET /connected-agents` - the clients and the agents that act for the
/// signed-in user, each with a revoke button.
#[handler]
pub async fn connected(req: Request) -> Response {
    let clients = ConnectedAgents::client_grants(&req).await?;
    let agents = ConnectedAgents::agent_accounts(&req).await?;
    inertia_response!(
        &req,
        "Agents/Connected",
        ConnectedAgentsProps {
            clients: clients.into_iter().map(ClientView::from).collect(),
            agents: agents.into_iter().map(AgentView::from).collect(),
        }
    )
}

/// `POST /connected-agents/clients/{grant}/revoke` - end one of the user's
/// client grants and every token of it.
#[handler]
pub async fn revoke_client(req: Request, grant: String) -> Response {
    if !ConnectedAgents::revoke_client_grant(&req, &grant).await? {
        return Err(FrameworkError::domain("You have no connected app by that name.", 404).into());
    }
    Inertia::flash(
        "toast",
        Toast::success("The app can no longer act for you."),
    )?;
    Redirect::to(CONNECTED).status(303).into()
}

/// `POST /connected-agents/agents/{registration}/revoke` - end one of the
/// user's agent registrations and every token issued from it.
#[handler]
pub async fn revoke_agent(req: Request, registration: String) -> Response {
    if !ConnectedAgents::revoke_agent_account(&req, &registration).await? {
        return Err(
            FrameworkError::domain("You have no connected agent by that name.", 404).into(),
        );
    }
    Inertia::flash(
        "toast",
        Toast::success("The agent can no longer act for you."),
    )?;
    Redirect::to(CONNECTED).status(303).into()
}
