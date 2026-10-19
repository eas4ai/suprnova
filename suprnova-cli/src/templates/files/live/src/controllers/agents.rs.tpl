//! The agent pages (AGENT-017): the approval page an MCP client sends the
//! user to, the claim page an agent sends the user to, and the signed-in
//! user's connected agents.
//!
//! They are plain server-rendered documents, not Live documents. Each one
//! renders through `Frame::plain`, mounts no island and loads no Live
//! runtime, so an agent in the browser is never offered a tool that
//! approves or claims: consent comes from the person. Every form is a
//! native form that posts the session's CSRF token, which the global CSRF
//! middleware checks.
//!
//! `AgentApprovals`, `AgentClaims` and `ConnectedAgents` read and change
//! what acts for the signed-in user, and only for that user: another
//! user's authorization reads as ended, their claim as someone else's,
//! their grant as unknown. The routes run `ConsentPageHeaders`, so no answer
//! here is framed, cached or named in a referrer, since the addresses carry
//! the authorization id and the claim's attempt token. A refusal renders
//! the error page with its own words, as HTML, which the error page
//! middleware leaves as it is.

use serde::Deserialize;
use suprnova::agents::{
    AgentAccount, AgentApprovals, AgentClaims, ApprovalDecision, ApprovalLookup, ClaimConfirmation,
    ClaimLookup, ClaimRefusal, ClientGrant, ConnectedAgents, PendingClaim,
};
use suprnova::{FrameworkError, HttpResponse, Redirect, Request, Response, Validate, handler};

use crate::flash::Toast;
use crate::views::{
    self, AgentsApprovePage, AgentsClaimPage, AgentsConnectedPage, AgentsReturningPage,
    ConnectedAgent, ConnectedClient, ErrorDocument, Frame, Nav,
};

/// Where a guest signs in, to come back to the page after.
const SIGN_IN: &str = "/login";

/// Where a claim and a revoke continue to.
const CONNECTED: &str = "/connected-agents";

const NO_AUTHORIZATION: &str = "This link does not name an authorization request.";
const ENDED: &str = "This request has ended. Start again from the app that asked.";
const NOT_A_DECISION: &str = "The form is not valid. Start again from the app that asked.";
const APPROVAL_SIGNED_OUT: &str =
    "Your sign-in has ended. Sign in, then start again from the app that asked.";
const NO_CLAIM: &str = "This link does not name a claim. Ask the agent for a new one.";
const OTHER_ACCOUNT: &str = "This claim is for another account, or your address is not verified \
                             yet. Sign in with the address the agent named.";
const CODE_ENDED: &str = "This code can no longer be used: it expired or was entered wrongly too \
                          often. Ask the agent for a new one.";
const NOT_A_CLAIM: &str = "The form is not valid. Open the link the agent gave you again.";
const CLAIM_SIGNED_OUT: &str =
    "Your sign-in has ended. Sign in, then open the link the agent gave you again.";
const ENTER_THE_CODE: &str = "Enter the code the agent gave you.";

/// The approval page's form: the pending authorization and what the user
/// decided. Each of the page's two forms carries both.
#[derive(Deserialize, Validate)]
struct DecisionForm {
    #[serde(default)]
    #[validate(length(min = 1, max = 128))]
    authorization: String,
    #[serde(default)]
    decision: String,
}

/// The claim page's form: the claim attempt and the code the agent gave
/// the user.
#[derive(Deserialize, Validate)]
struct ClaimForm {
    #[serde(default)]
    #[validate(length(min = 1, max = 512))]
    claim_attempt_token: String,
    #[serde(default)]
    #[validate(length(min = 1, max = 16))]
    user_code: String,
}

/// `GET /oauth/approve?authorization=...`: what the client asks to do as
/// the user. A guest signs in and comes back.
#[handler]
pub async fn approval(req: Request) -> Response {
    let approvals = AgentApprovals::of(&req)?;
    let id = req.query_param("authorization").filter(|id| !id.is_empty());
    // A guest is answered before the id is read, so nothing about the
    // request shows until the user signs in.
    let lookup = approvals.pending(id.as_deref().unwrap_or_default()).await?;
    match (lookup, id) {
        (ApprovalLookup::SignIn, _) => Redirect::guest(&req, SIGN_IN).into(),
        (_, None) => Ok(refusal(400, NO_AUTHORIZATION).await?),
        (ApprovalLookup::Ended, Some(_)) => Ok(refusal(404, ENDED).await?),
        (ApprovalLookup::Pending(approval), Some(_)) => {
            // The title names no client: it holds no markup to isolate a
            // name the client chose.
            let page = AgentsApprovePage {
                frame: Frame::plain("Allow an app to act for you?", Nav::Agents).await?,
                approval,
            };
            Ok(views::render_plain(&page, 200)?)
        }
    }
}

/// `POST /oauth/approve`: record the user's decision, then return them to
/// the client with the returning page.
#[handler]
pub async fn decide(req: Request) -> Response {
    let approvals = AgentApprovals::of(&req)?;
    let form: DecisionForm = req.form().await?;
    let approve = match form.decision.as_str() {
        "approve" => true,
        "deny" => false,
        _ => return Ok(refusal(400, NOT_A_DECISION).await?),
    };
    if form.validate().is_err() {
        return Ok(refusal(400, NO_AUTHORIZATION).await?);
    }
    // The client's name for the returning page, read before the decision
    // spends the authorization.
    let client_name = match approvals.pending(&form.authorization).await? {
        ApprovalLookup::Pending(pending) => pending.client_name,
        ApprovalLookup::Ended => return Ok(refusal(404, ENDED).await?),
        ApprovalLookup::SignIn => return Ok(refusal(401, APPROVAL_SIGNED_OUT).await?),
    };
    match approvals.decide(&form.authorization, approve).await? {
        ApprovalDecision::Redirect(url) => Ok(returning_page(client_name, url).await?),
        ApprovalDecision::Ended => Ok(refusal(404, ENDED).await?),
        ApprovalDecision::SignIn => Ok(refusal(401, APPROVAL_SIGNED_OUT).await?),
    }
}

/// The answer to a decision: a page that sends the browser to `url`, the
/// client's registered redirect URI with the code or the denial.
///
/// It is a page rather than a redirect because the kit's policy carries
/// `form-action 'self'`, which a browser applies to the redirects that
/// follow a form post: a `303` to the client, usually on another origin,
/// would be blocked. The page's meta refresh is a navigation of its own,
/// which `form-action` does not govern. `url` is the authorization
/// server's own redirect, built from a redirect URI the client registered:
/// HTTPS, loopback HTTP or a private-use scheme, never `javascript:`.
async fn returning_page(client_name: String, url: String) -> Result<HttpResponse, FrameworkError> {
    let page = AgentsReturningPage {
        frame: Frame::plain("Returning you to the app", Nav::Agents).await?,
        client_name,
        redirect: url,
    };
    views::render_plain(&page, 200)
}

/// `GET /agent/claim?claim_attempt_token=...`: the agent that asks to act
/// as the user, and the form that takes the code it gave them. A guest
/// signs in and comes back.
#[handler]
pub async fn claim(req: Request) -> Response {
    let claims = AgentClaims::of(&req)?;
    let token = req
        .query_param("claim_attempt_token")
        .filter(|token| !token.is_empty());
    // A guest is answered before the token is read.
    let lookup = claims.pending(token.as_deref().unwrap_or_default()).await?;
    match (lookup, token) {
        (ClaimLookup::Refused(ClaimRefusal::SignIn), _) => Redirect::guest(&req, SIGN_IN).into(),
        (_, None) => Ok(refusal(400, NO_CLAIM).await?),
        (ClaimLookup::Pending(claim), Some(token)) => {
            Ok(claim_page(claim, token, String::new(), 200).await?)
        }
        (ClaimLookup::Refused(why), Some(_)) => Ok(refused(why).await?),
    }
}

/// `POST /agent/claim`: confirm the claim with the code the user typed. A
/// wrong code shows the page again with the attempts left, and the right
/// one continues to the connected agents.
#[handler]
pub async fn confirm(req: Request) -> Response {
    let claims = AgentClaims::of(&req)?;
    let form: ClaimForm = req.form().await?;
    if let Err(errors) = form.validate() {
        if errors.field_errors().contains_key("claim_attempt_token") {
            return Ok(refusal(400, NO_CLAIM).await?);
        }
        // An empty or overlong code is no guess: the page again, with
        // nothing counted against the attempt.
        let page = code_again(&claims, form.claim_attempt_token, ENTER_THE_CODE.to_owned());
        return Ok(page.await?);
    }
    match claims
        .confirm(&form.claim_attempt_token, &form.user_code)
        .await?
    {
        ClaimConfirmation::Claimed { .. } => {
            Toast::success("The agent is connected.").flash()?;
            Redirect::to(CONNECTED).status(303).into()
        }
        ClaimConfirmation::WrongCode { attempts_remaining } => {
            let message = format!("That code is wrong. {attempts_remaining} attempts left.");
            Ok(code_again(&claims, form.claim_attempt_token, message).await?)
        }
        ClaimConfirmation::Refused(why) => Ok(refused(why).await?),
    }
}

/// The claim page with `code_error` under the code, answering `status`.
async fn claim_page(
    claim: PendingClaim,
    claim_attempt_token: String,
    code_error: String,
    status: u16,
) -> Result<HttpResponse, FrameworkError> {
    let minutes = claim.expires_in_seconds.div_ceil(60);
    let page = AgentsClaimPage {
        frame: Frame::plain("Let an agent act for you?", Nav::Agents).await?,
        claim,
        claim_attempt_token,
        minutes,
        code_error,
    };
    views::render_plain(&page, status)
}

/// The claim page again, as the claim stands now, with `message` under the
/// code. A claim the last wrong code ended answers as ended.
async fn code_again(
    claims: &AgentClaims,
    claim_attempt_token: String,
    message: String,
) -> Result<HttpResponse, FrameworkError> {
    match claims.pending(&claim_attempt_token).await? {
        ClaimLookup::Pending(claim) => claim_page(claim, claim_attempt_token, message, 422).await,
        ClaimLookup::Refused(why) => refused(why).await,
    }
}

/// The page for a claim the engine refused. A form post cannot be replayed
/// after a sign-in, so a user whose sign-in ended opens the link again.
async fn refused(why: ClaimRefusal) -> Result<HttpResponse, FrameworkError> {
    match why {
        ClaimRefusal::OtherAccount => refusal(403, OTHER_ACCOUNT).await,
        ClaimRefusal::NotFound => refusal(404, NO_CLAIM).await,
        ClaimRefusal::Ended => refusal(410, CODE_ENDED).await,
        ClaimRefusal::Invalid => refusal(400, NOT_A_CLAIM).await,
        ClaimRefusal::SignIn => refusal(401, CLAIM_SIGNED_OUT).await,
    }
}

/// `GET /connected-agents`: every MCP client and agent that acts for the
/// signed-in user, newest first, each with a form that revokes it.
#[handler]
pub async fn connected(req: Request) -> Response {
    let clients = ConnectedAgents::client_grants(&req)
        .await?
        .into_iter()
        .map(client_row)
        .collect();
    let agents = ConnectedAgents::agent_accounts(&req)
        .await?
        .into_iter()
        .map(agent_row)
        .collect();
    let page = AgentsConnectedPage {
        frame: Frame::plain("Connected agents", Nav::Agents).await?,
        clients,
        agents,
    };
    Ok(views::render_plain(&page, 200)?)
}

/// `POST /connected-agents/clients/{grant}/revoke`: end one of the
/// signed-in user's client grants and every token of it.
#[handler]
pub async fn revoke_client(req: Request, grant: String) -> Response {
    if !ConnectedAgents::revoke_client_grant(&req, &grant).await? {
        let message = "You have no connected app by that name. It may already be revoked.";
        return Ok(refusal(404, message).await?);
    }
    Toast::success("The app can no longer act for you.").flash()?;
    Redirect::to(CONNECTED).status(303).into()
}

/// `POST /connected-agents/agents/{registration}/revoke`: end one of the
/// signed-in user's agent registrations and every token issued from it.
#[handler]
pub async fn revoke_agent(req: Request, registration: String) -> Response {
    if !ConnectedAgents::revoke_agent_account(&req, &registration).await? {
        let message = "You have no connected agent by that name. It may already be revoked.";
        return Ok(refusal(404, message).await?);
    }
    Toast::success("The agent can no longer act for you.").flash()?;
    Redirect::to(CONNECTED).status(303).into()
}

/// When something happened, as the connected-agents page prints it.
fn when(at: chrono::DateTime<chrono::Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// A client grant as the connected-agents page shows it.
fn client_row(grant: ClientGrant) -> ConnectedClient {
    ConnectedClient {
        allowed_on: when(grant.created_at),
        last_used: grant.last_used_at.map_or_else(|| "never".to_owned(), when),
        id: grant.id,
        client_name: grant.client_name,
        client_host: grant.client_host,
        scopes: grant.scopes,
    }
}

/// An agent registration as the connected-agents page shows it. The
/// provider's name is the application's trust list entry, never one the
/// provider sent.
fn agent_row(agent: AgentAccount) -> ConnectedAgent {
    let how = match agent.registration_type.as_str() {
        "anonymous" => "Registered on its own and claimed by you",
        "service_auth" => "Registered with your email address",
        "identity_assertion" => "Signed in through a provider you use",
        _ => "An agent account",
    };
    ConnectedAgent {
        registered_on: when(agent.created_at),
        claimed_on: agent.claimed_at.map(when),
        how,
        id: agent.id,
        provider: agent.provider,
        scopes: agent.scopes,
    }
}

/// The error page with `message`, answering `status`. Rendered here, as
/// HTML, so it keeps its words: the error page middleware gives a `404`
/// its own.
async fn refusal(status: u16, message: &str) -> Result<HttpResponse, FrameworkError> {
    let page = ErrorDocument {
        frame: Frame::plain(&status.to_string(), Nav::Agents).await?,
        status,
        message: message.to_owned(),
        request_id: None,
        reference: "Reference:".to_owned(),
        go_home: "Go home".to_owned(),
    };
    views::render_plain(&page, status)
}
