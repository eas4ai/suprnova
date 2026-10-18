//! The token and revocation endpoints.

use std::collections::BTreeMap;

use chrono::Utc;
use subtle::ConstantTimeEq;

use super::flow::{
    CODE_ISSUED, CODE_KIND, CODE_REDEEMED_PREFIX, CODE_REDEEMING_PREFIX, CODE_REPLAYED_PREFIX,
    CodePayload, decode,
};
use super::grants::{TokenError, TokenRequest, TokenResponse};
use super::store::{ACCESS_TOKEN_KIND, GrantRecord, REFRESH_TOKEN_KIND, USER_GRANT_KIND};
use super::{
    AuthorizationServer, MAX_TOKEN_LENGTH, TokenIssuer, is_code_verifier, new_id,
    normalize_resource, pkce_challenge, token_digest, token_storage_error,
};

type TokenResult<T> = std::result::Result<T, TokenError>;

fn required<'a>(request: &'a TokenRequest, name: &str) -> TokenResult<&'a str> {
    request
        .param(name)
        .ok_or_else(|| TokenError::invalid_request(format!("{name} is required")))
}

impl AuthorizationServer {
    /// Answer the token endpoint: the `authorization_code` and
    /// `refresh_token` grants here, any other registered grant type through
    /// its [`GrantHandler`](super::GrantHandler), and
    /// `unsupported_grant_type` for the rest.
    ///
    /// # Errors
    ///
    /// Returns the RFC 6749 section 5.2 error to answer with; a storage
    /// failure is logged and answered as `server_error`.
    pub async fn token(&self, request: &TokenRequest) -> TokenResult<TokenResponse> {
        let grant_type = required(request, "grant_type")?;
        match grant_type {
            "authorization_code" => self.exchange_code(request).await,
            "refresh_token" => self.refresh(request).await,
            other => match self.grant_handlers.get(other) {
                Some(handler) => {
                    handler
                        .exchange(request, &TokenIssuer { server: self })
                        .await
                }
                None => Err(TokenError::unsupported_grant_type(format!(
                    "grant_type {other:?} is not supported"
                ))),
            },
        }
    }

    /// Exchange a code: one time, before it expires, by the client it was
    /// issued to, with the same redirect URI and resource, and a verifier
    /// whose S256 challenge matches. A code presented again, after or
    /// alongside its first exchange, revokes what that exchange issued
    /// (OAuth 2.1 section 4.1.3).
    ///
    /// The code's state carries the grant it is producing: `issued`, then
    /// `redeeming:<grant>` while the grant is written, then
    /// `redeemed:<grant>`. A replay that finds `redeeming:<grant>` marks it
    /// `replayed:<grant>`, and the first exchange, finding its last step
    /// refused, revokes the grant it just wrote instead of answering with
    /// it.
    async fn exchange_code(&self, request: &TokenRequest) -> TokenResult<TokenResponse> {
        let code = required(request, "code")?;
        let client_id = required(request, "client_id")?;
        let redirect_uri = required(request, "redirect_uri")?;
        let verifier = required(request, "code_verifier")?;
        let resource = required(request, "resource")?;
        if code.len() > MAX_TOKEN_LENGTH {
            return Err(TokenError::invalid_grant("the code is invalid"));
        }
        let selector = token_digest(code);
        let Some(record) = self
            .store
            .find_authorization(&selector, CODE_KIND, Utc::now())
            .await
            .map_err(token_storage_error)?
        else {
            return Err(TokenError::invalid_grant("the code is invalid or expired"));
        };
        if record.state != CODE_ISSUED {
            self.answer_code_replay(&selector, &record.state).await;
            return Err(TokenError::invalid_grant("the code was already used"));
        }
        let payload: CodePayload = decode(&record.payload).map_err(token_storage_error)?;
        let now = Utc::now();
        if payload.code_expires_at <= now {
            return Err(TokenError::invalid_grant("the code expired"));
        }
        if payload.client_id != client_id {
            return Err(TokenError::invalid_grant(
                "the code was issued to another client",
            ));
        }
        if payload.redirect_uri != redirect_uri {
            return Err(TokenError::invalid_grant(
                "redirect_uri differs from the authorization request",
            ));
        }
        if normalize_resource(resource).as_deref() != Some(payload.resource.as_str()) {
            return Err(TokenError::invalid_target(
                "resource differs from the authorization request",
            ));
        }
        if !is_code_verifier(verifier)
            || pkce_challenge(verifier)
                .as_bytes()
                .ct_eq(payload.code_challenge.as_bytes())
                .unwrap_u8()
                != 1
        {
            return Err(TokenError::invalid_grant(
                "code_verifier does not match the code challenge",
            ));
        }
        if self
            .epochs
            .current_epoch(&payload.user_id)
            .await
            .map_err(token_storage_error)?
            != Some(payload.auth_epoch)
        {
            return Err(TokenError::invalid_grant(
                "the user signed out everywhere or changed their password since approving",
            ));
        }
        // Only one exchange may move the code off `issued`; a concurrent
        // one loses here, and is a replay of the code like any later one.
        let grant_id = new_id();
        let redeeming = format!("{CODE_REDEEMING_PREFIX}{grant_id}");
        if !self
            .store
            .transition_authorization(&selector, CODE_KIND, CODE_ISSUED, &redeeming, now)
            .await
            .map_err(token_storage_error)?
        {
            if let Some(current) = self
                .store
                .find_authorization(&selector, CODE_KIND, Utc::now())
                .await
                .map_err(token_storage_error)?
            {
                self.answer_code_replay(&selector, &current.state).await;
            }
            return Err(TokenError::invalid_grant("the code was already used"));
        }
        let minted = self
            .mint_tokens(
                &payload.scopes,
                ACCESS_TOKEN_KIND,
                self.config.access_token_lifetime,
                payload.refresh,
                now,
            )
            .map_err(token_storage_error)?;
        self.store
            .create_grant(
                GrantRecord {
                    id: grant_id.clone(),
                    kind: USER_GRANT_KIND.to_owned(),
                    user_id: Some(payload.user_id),
                    client_id: payload.client_id,
                    client_name: payload.client_name,
                    client_host: payload.client_host,
                    scopes: payload.scopes,
                    resource: payload.resource,
                    auth_epoch: Some(payload.auth_epoch),
                    access_kind: ACCESS_TOKEN_KIND.to_owned(),
                    access_lifetime: self.config.access_token_lifetime,
                    created_at: now,
                    last_used_at: None,
                    revoked_at: None,
                },
                minted.records,
            )
            .await
            .map_err(token_storage_error)?;
        match self
            .store
            .transition_authorization(
                &selector,
                CODE_KIND,
                &redeeming,
                &format!("{CODE_REDEEMED_PREFIX}{grant_id}"),
                Utc::now(),
            )
            .await
        {
            Ok(true) => Ok(minted.response),
            Ok(false) => {
                // A replay marked the code while this exchange wrote its
                // grant: the tokens must not leave the server.
                self.revoke_code_grant(&grant_id).await;
                Err(TokenError::invalid_grant("the code was presented twice"))
            }
            Err(error) => {
                // The code stays `redeeming:<grant>`, which a replay still
                // reads and revokes; the tokens are the client's.
                tracing::warn!(error = %error, grant_id, "recording a redeemed code's grant failed");
                Ok(minted.response)
            }
        }
    }

    /// Answer a code presented again: mark a grant still being written as
    /// replayed, so its exchange revokes it, and revoke the grant the code
    /// produced. Failures are logged; the answer is `invalid_grant` either
    /// way.
    async fn answer_code_replay(&self, selector: &str, state: &str) {
        let grant_id = if let Some(grant_id) = state.strip_prefix(CODE_REDEEMING_PREFIX) {
            if let Err(error) = self
                .store
                .transition_authorization(
                    selector,
                    CODE_KIND,
                    state,
                    &format!("{CODE_REPLAYED_PREFIX}{grant_id}"),
                    Utc::now(),
                )
                .await
            {
                tracing::error!(error = %error, grant_id, "marking a replayed code failed");
            }
            grant_id
        } else if let Some(grant_id) = state
            .strip_prefix(CODE_REDEEMED_PREFIX)
            .or_else(|| state.strip_prefix(CODE_REPLAYED_PREFIX))
        {
            grant_id
        } else {
            return;
        };
        tracing::warn!(
            grant_id,
            "an authorization code was presented again; revoking its grant"
        );
        self.revoke_code_grant(grant_id).await;
    }

    async fn revoke_code_grant(&self, grant_id: &str) {
        if let Err(error) = self.store.revoke_grant(grant_id, Utc::now()).await {
            tracing::error!(error = %error, grant_id, "revoking a replayed code's grant failed");
        }
    }

    /// Rotate a refresh token. Every use spends it and returns a new one;
    /// presenting a spent one, after or alongside its first use, revokes
    /// the whole grant (RFC 9700 section 4.14.2).
    async fn refresh(&self, request: &TokenRequest) -> TokenResult<TokenResponse> {
        let refresh_token = required(request, "refresh_token")?;
        let client_id = required(request, "client_id")?;
        if refresh_token.len() > MAX_TOKEN_LENGTH {
            return Err(TokenError::invalid_grant("the refresh token is invalid"));
        }
        let Some(token) = self
            .store
            .find_token(&token_digest(refresh_token))
            .await
            .map_err(token_storage_error)?
        else {
            return Err(TokenError::invalid_grant("the refresh token is invalid"));
        };
        if token.kind != REFRESH_TOKEN_KIND {
            return Err(TokenError::invalid_grant("the refresh token is invalid"));
        }
        let Some(grant) = self
            .store
            .find_grant(&token.grant_id)
            .await
            .map_err(token_storage_error)?
        else {
            return Err(TokenError::invalid_grant("the refresh token is invalid"));
        };
        if grant.client_id != client_id {
            return Err(TokenError::invalid_grant(
                "the refresh token was issued to another client",
            ));
        }
        let now = Utc::now();
        if token.used_at.is_some() {
            self.revoke_reused_grant(&grant.id).await;
            return Err(TokenError::invalid_grant(
                "the refresh token was already used",
            ));
        }
        if token.revoked_at.is_some() || grant.revoked_at.is_some() {
            return Err(TokenError::invalid_grant("the refresh token was revoked"));
        }
        if token.expires_at <= now {
            return Err(TokenError::invalid_grant("the refresh token expired"));
        }
        if !self
            .epoch_is_current(&grant)
            .await
            .map_err(token_storage_error)?
        {
            self.store
                .revoke_grant(&grant.id, now)
                .await
                .map_err(token_storage_error)?;
            return Err(TokenError::invalid_grant(
                "the user signed out everywhere or changed their password",
            ));
        }
        if let Some(resource) = request.param("resource")
            && normalize_resource(resource).as_deref() != Some(grant.resource.as_str())
        {
            return Err(TokenError::invalid_target(
                "resource differs from the one the grant is bound to",
            ));
        }
        let scopes = match request.param("scope") {
            None => grant.scopes.clone(),
            Some(scope) => {
                let requested: Vec<String> = scope.split_whitespace().map(str::to_owned).collect();
                if let Some(outside) = requested.iter().find(|name| !grant.scopes.contains(name)) {
                    return Err(TokenError::invalid_scope(format!(
                        "scope {outside:?} was not granted"
                    )));
                }
                grant
                    .scopes
                    .iter()
                    .filter(|name| requested.contains(name))
                    .cloned()
                    .collect()
            }
        };
        // The access token takes the requested subset and the kind and
        // lifetime the grant was issued with; the replacement refresh token
        // keeps the grant's full scopes, as RFC 6749 section 6 asks.
        let mut minted = self
            .mint_tokens(
                &scopes,
                &grant.access_kind,
                grant.access_lifetime,
                true,
                now,
            )
            .map_err(token_storage_error)?;
        for record in &mut minted.records {
            if record.kind == REFRESH_TOKEN_KIND {
                record.scopes = grant.scopes.clone();
            }
        }
        match self
            .store
            .rotate_refresh_token(&token.id, &grant.id, now, minted.records)
            .await
        {
            Ok(true) => Ok(minted.response),
            Ok(false) => {
                // Another request spent or revoked the token between the
                // read above and the guarded write: a concurrent second use.
                self.revoke_reused_grant(&grant.id).await;
                Err(TokenError::invalid_grant(
                    "the refresh token was already used",
                ))
            }
            Err(error) => {
                // A failed rotation, a deadlock victim included, may hide a
                // concurrent reuse; the grant is ended rather than left live.
                tracing::error!(error = %error, grant_id = %grant.id, "rotating a refresh token failed");
                self.revoke_reused_grant(&grant.id).await;
                Err(TokenError::server_error())
            }
        }
    }

    /// Revoke the grant of a refresh token that was presented again, or
    /// whose rotation failed. A failure is logged, not answered: the
    /// client's answer is the same either way.
    async fn revoke_reused_grant(&self, grant_id: &str) {
        tracing::warn!(grant_id, "revoking the grant of a reused refresh token");
        if let Err(error) = self.store.revoke_grant(grant_id, Utc::now()).await {
            tracing::error!(error = %error, grant_id, "revoking a reused refresh token's grant failed");
        }
    }

    /// Answer the revocation endpoint (RFC 7009). A refresh token revokes
    /// its whole grant; an access token only itself. An unknown or already
    /// revoked token, or one issued to a client other than the `client_id`
    /// given, answers success without revealing which.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` without a `token`, and `server_error`
    /// when storage fails.
    pub async fn revoke(&self, params: &BTreeMap<String, String>) -> TokenResult<()> {
        let Some(token) = params.get("token").filter(|token| !token.is_empty()) else {
            return Err(TokenError::invalid_request("token is required"));
        };
        if token.len() > MAX_TOKEN_LENGTH {
            return Ok(());
        }
        let Some(record) = self
            .store
            .find_token(&token_digest(token))
            .await
            .map_err(token_storage_error)?
        else {
            return Ok(());
        };
        let Some(grant) = self
            .store
            .find_grant(&record.grant_id)
            .await
            .map_err(token_storage_error)?
        else {
            return Ok(());
        };
        if let Some(client_id) = params.get("client_id").filter(|value| !value.is_empty())
            && *client_id != grant.client_id
        {
            return Ok(());
        }
        let now = Utc::now();
        if record.kind == REFRESH_TOKEN_KIND {
            self.store
                .revoke_grant(&grant.id, now)
                .await
                .map_err(token_storage_error)?;
        } else {
            self.store
                .revoke_token(&record.id, now)
                .await
                .map_err(token_storage_error)?;
        }
        Ok(())
    }
}
