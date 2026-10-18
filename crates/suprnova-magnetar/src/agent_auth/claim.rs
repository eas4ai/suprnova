//! The two routes behind the host's claim page: the pending claim as JSON,
//! and the confirmation that takes the user code. Both need the signed-in
//! user, refuse any user but the one the claim is bound to, and refuse to
//! be framed.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use super::store::{IDENTITY_ASSERTION, RegistrationRecord, STATUS_PENDING, STATUS_UNCLAIMED};
use super::{
    AgentUser, Core, NO_FRAMING, NO_STORE, SignedInUser, digest, failure, respond, same_email,
    timestamp,
};
use crate::Result;
use crate::authorization_server::AuthorizationServer;
use crate::plugin::{LifecycleEventKind, WireResponse};

/// The headers of both claim page routes.
const HEADERS: [(&str, &str); 3] = [NO_FRAMING[0], NO_FRAMING[1], NO_STORE[0]];

fn page_error(status: u16, error: &str, description: &str) -> WireResponse {
    respond(
        status,
        Some(json!({"error": error, "error_description": description})),
        &HEADERS,
    )
}

/// What the claim page routes found behind an attempt token.
enum Lookup {
    /// The attempt can be confirmed by this user. Boxed so the refusal
    /// stays small.
    Ready(Box<(RegistrationRecord, AgentUser)>),
    /// The answer to give instead.
    Refused(WireResponse),
}

impl Core {
    /// Find the attempt and check the user against it. Order matters: no
    /// one learns anything about an attempt before signing in, and a user
    /// signed in as someone else learns only that it is not theirs.
    async fn lookup_attempt(
        &self,
        attempt_token: &str,
        signed_in: Option<&SignedInUser>,
        now: DateTime<Utc>,
    ) -> Result<Lookup> {
        let Some(signed_in) = signed_in else {
            return Ok(Lookup::Refused(page_error(
                401,
                "login_required",
                "sign in to continue",
            )));
        };
        let not_found = || {
            Lookup::Refused(page_error(
                404,
                "not_found",
                "no such claim attempt; ask the agent for a new link",
            ))
        };
        if attempt_token.is_empty() || attempt_token.len() > 512 {
            return Ok(not_found());
        }
        let Some(record) = self
            .store
            .find_by_attempt_token(&digest(attempt_token))
            .await?
        else {
            return Ok(not_found());
        };
        if record.status != STATUS_UNCLAIMED && record.status != STATUS_PENDING {
            return Ok(not_found());
        }
        // A session from before the user's last password change or "sign
        // out everywhere" no longer speaks for them.
        let user = match self.users.find_by_id(&signed_in.user_id).await? {
            Some(user) if user.auth_epoch.is_none() || user.auth_epoch == signed_in.auth_epoch => {
                user
            }
            _ => {
                return Ok(Lookup::Refused(page_error(
                    401,
                    "login_required",
                    "sign in again to continue",
                )));
            }
        };
        let wrong_user = record
            .claim_user_id
            .as_deref()
            .is_some_and(|bound| bound != user.user_id);
        // The claim is bound to an email, so the user must hold it: the
        // same address, verified by the application.
        let wrong_email = !user.email_verified
            || record
                .claim_email
                .as_deref()
                .is_none_or(|bound| !same_email(bound, &user.email));
        if wrong_user || wrong_email {
            return Ok(Lookup::Refused(page_error(
                403,
                "access_denied",
                "this claim is for another account; sign in with the email the agent named",
            )));
        }
        let open = record
            .user_code_expires_at
            .is_some_and(|expiry| expiry > now)
            && record.claim_expires_at.is_some_and(|window| window > now)
            && record.user_code_failures < self.config.max_user_code_failures
            && record.user_code_hash.is_some();
        if !open {
            return Ok(Lookup::Refused(page_error(
                410,
                "expired",
                "this code expired or was entered wrongly too often; ask the agent for a new one",
            )));
        }
        Ok(Lookup::Ready(Box::new((record, user))))
    }

    pub(crate) async fn pending_claim(
        &self,
        attempt_token: &str,
        signed_in: Option<&SignedInUser>,
    ) -> WireResponse {
        self.pending_claim_checked(attempt_token, signed_in)
            .await
            .unwrap_or_else(|error| with_headers(failure(&error)))
    }

    async fn pending_claim_checked(
        &self,
        attempt_token: &str,
        signed_in: Option<&SignedInUser>,
    ) -> Result<WireResponse> {
        let now = Utc::now();
        let (record, user) = match self.lookup_attempt(attempt_token, signed_in, now).await? {
            Lookup::Ready(ready) => *ready,
            Lookup::Refused(response) => return Ok(response),
        };
        let earlier = self.store.user_registrations(&user.user_id).await?;
        let mut advisories = Vec::new();
        if record.kind == IDENTITY_ASSERTION
            && !earlier
                .iter()
                .any(|other| other.issuer.is_some() && other.issuer == record.issuer)
        {
            advisories.push("first_time_provider");
        }
        if earlier.is_empty() {
            advisories.push("first_time_account");
        }
        // The provider's name comes from the application's trust list, never
        // from anything the provider sent.
        let provider = record
            .issuer
            .as_deref()
            .and_then(|issuer| self.config.trusted(issuer))
            .map(|entry| json!({"display_name": entry.display_name}));
        let expires_at = record.user_code_expires_at.unwrap_or(now);
        Ok(respond(
            200,
            Some(json!({
                "registration_id": record.id,
                "registration_type": record.kind,
                "claim_attempt_id": record.attempt_id,
                "signed_in_as": user.email,
                "email": record.claim_email,
                "provider": provider.unwrap_or(Value::Null),
                "scopes": self
                    .describe(&record.scopes)
                    .into_iter()
                    .map(|scope| json!({"name": scope.name, "description": scope.description}))
                    .collect::<Vec<_>>(),
                "expires_at": timestamp(expires_at),
                "expires_in": (expires_at - now).num_seconds().max(0),
                "attempts_remaining": self
                    .config
                    .max_user_code_failures
                    .saturating_sub(record.user_code_failures),
                "advisories": advisories,
            })),
            &HEADERS,
        ))
    }

    pub(crate) async fn confirm_claim(
        &self,
        server: &AuthorizationServer,
        attempt_token: &str,
        user_code: &str,
        signed_in: Option<&SignedInUser>,
        client_address: Option<&str>,
    ) -> WireResponse {
        self.confirm_claim_checked(server, attempt_token, user_code, signed_in, client_address)
            .await
            .unwrap_or_else(|error| with_headers(failure(&error)))
    }

    async fn confirm_claim_checked(
        &self,
        server: &AuthorizationServer,
        attempt_token: &str,
        user_code: &str,
        signed_in: Option<&SignedInUser>,
        client_address: Option<&str>,
    ) -> Result<WireResponse> {
        let now = Utc::now();
        let (record, user) = match self.lookup_attempt(attempt_token, signed_in, now).await? {
            Lookup::Ready(ready) => *ready,
            Lookup::Refused(response) => return Ok(response),
        };
        let attempt_hash = digest(attempt_token);
        let code: String = user_code
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();
        let code_hash = digest(&code);
        let matches = code.len() == 6
            && code.bytes().all(|byte| byte.is_ascii_digit())
            && record.user_code_hash.as_deref().is_some_and(|stored| {
                stored.as_bytes().ct_eq(code_hash.as_bytes()).unwrap_u8() == 1
            });
        if !matches {
            let counted = self
                .store
                .record_code_failure(
                    &record.id,
                    &attempt_hash,
                    self.config.max_user_code_failures,
                )
                .await?;
            let used = record.user_code_failures.saturating_add(u32::from(counted));
            let remaining = self.config.max_user_code_failures.saturating_sub(used);
            return Ok(respond(
                400,
                Some(json!({
                    "error": "invalid_user_code",
                    "error_description": "the code is wrong",
                    "attempts_remaining": remaining,
                })),
                &HEADERS,
            ));
        }
        if !self
            .store
            .confirm_claim(
                &record.id,
                &attempt_hash,
                &code_hash,
                &user.user_id,
                user.auth_epoch,
                self.config.max_user_code_failures,
                now,
            )
            .await?
        {
            // Another confirmation, a wrong code that hit the limit or the
            // end of the window got there first.
            return Ok(page_error(
                410,
                "expired",
                "this code can no longer be used; ask the agent for a new one",
            ));
        }
        // The claim is committed: it is recorded before anything else can
        // fail.
        let mut attributes = vec![
            ("registration_id", record.id.clone()),
            ("registration_type", record.kind.clone()),
            ("claimed_by_user_id", user.user_id.clone()),
        ];
        if let Some(issuer) = &record.issuer {
            attributes.push(("iss", issuer.clone()));
        }
        if let Some(subject) = &record.subject {
            attributes.push(("sub", subject.clone()));
        }
        self.emit(
            LifecycleEventKind::AgentClaimConfirmed,
            Some(&user.user_id),
            &attributes,
            client_address,
        )
        .await;
        // The pre-claim tokens end here: the canonical credential is the
        // one the agent's next poll receives. If this revocation fails, the
        // claim still stands, and that poll replaces the pre-claim grant,
        // which no longer fits the claimed registration, revoking it then.
        match self.revoke_grants(&server.token_issuer(), &record.id).await {
            Ok(0) => {}
            Ok(revoked) => {
                self.emit(
                    LifecycleEventKind::AgentTokenRevoked,
                    None,
                    &[
                        ("registration_id", record.id.clone()),
                        ("reason", "claimed".to_owned()),
                        ("grants", revoked.to_string()),
                    ],
                    client_address,
                )
                .await;
            }
            Err(error) => {
                tracing::error!(error = %error, registration_id = %record.id, "revoking a claimed registration's pre-claim grant failed");
            }
        }
        Ok(respond(
            200,
            Some(json!({"status": "claimed", "registration_id": record.id})),
            &HEADERS,
        ))
    }
}

/// A failure answer with the claim page headers.
fn with_headers(response: WireResponse) -> WireResponse {
    let mut effects = response.into_effects();
    for (name, value) in HEADERS {
        effects.headers.insert(name.to_owned(), value.to_owned());
    }
    WireResponse::from_effects(effects)
}
