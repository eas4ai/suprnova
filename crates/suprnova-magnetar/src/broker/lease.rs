//! The pre-call lease/CAS protocol
//! (`docs/specs/suprnova-magnetar/11-token-broker.md`'s "Refresh under
//! rotation" and "M2M cache" sections).
//!
//! Every entry point here goes through the identical shape: read the row,
//! decide whether a claim is needed, conditionally claim it, run exactly
//! one provider exchange as the claim's owner, and conditionally commit.
//! Every step is correct under `single_flight = false`
//! ([`super::singleflight`] only decides whether concurrent in-process
//! callers additionally serialize before reaching this loop) because every
//! mutating step is a single conditioned SQL statement checked against
//! `rows_affected` -- two [`super::TokenBrokerService`] instances sharing
//! one database converge the same way two concurrent callers of the same
//! instance do.
//!
//! Reuse-vs-follower disambiguation (spec 11's "Refresh under rotation",
//! and the ambiguity flagged in
//! `docs/specs/suprnova-magnetar/reviews/001-adversarial.md` finding H9)
//! turns on one rule, applied fresh on every loop iteration: a presented
//! generation behind the stored one is reuse only when no claim is
//! currently live on the record. While a claim is live, a stale-looking
//! presenter is treated identically to an ordinary follower -- it waits
//! and re-reads -- because a live claim is proof a refresh is already in
//! flight and the row has not finished settling; reuse is only declared
//! once the row is quiescent (no claim) and still shows a generation past
//! what the caller holds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};

use super::cache;
use super::policy::{self, FailureClass};
use super::{
    AccessToken, BrokerError, BrokerResult, M2MCacheKey, RefreshRequest, TokenBrokerService,
};
use crate::crypto::CryptoPurpose;
use crate::oauth::grants::{client_credentials, refresh};
use crate::oauth::provider::OAuthProvider;
use crate::storage::{CommitProviderToken, NewProviderToken, ProviderTokenRow};

/// Provider name used for failures not attributable to a resolved
/// provider (a caller-protocol violation, a decode failure on a stored
/// value) -- distinct from any real provider's registry name.
const INTERNAL: &str = "broker";

fn resolve_provider(
    service: &TokenBrokerService,
    name: &str,
) -> BrokerResult<Arc<dyn OAuthProvider>> {
    service
        .registry
        .get(name)
        .ok_or_else(|| BrokerError::UnknownProvider {
            provider: name.to_owned(),
        })
}

fn split_scopes(joined: &str) -> Vec<String> {
    joined.split_whitespace().map(str::to_owned).collect()
}

fn overall_deadline(service: &TokenBrokerService) -> Instant {
    Instant::now() + service.config.provider_call_timeout + service.config.lease_grace * 2
}

fn decrypt_access_token(
    service: &TokenBrokerService,
    row: &ProviderTokenRow,
) -> BrokerResult<AccessToken> {
    let plaintext = service
        .encryptor
        .decrypt(CryptoPurpose::ProviderToken, &row.access_ciphertext)?;
    let value = String::from_utf8(plaintext).map_err(|_| BrokerError::Terminal {
        provider: INTERNAL,
        message: "stored access token is not valid utf-8".to_owned(),
    })?;
    Ok(AccessToken {
        value: SecretString::from(value),
        token_type: row.token_type.clone(),
        expires_at: row.access_expires_at,
        scopes: split_scopes(&row.scopes),
    })
}

fn row_is_fresh(row: &ProviderTokenRow, now: DateTime<Utc>) -> bool {
    !row.access_ciphertext.is_empty()
        && row
            .access_expires_at
            .map(|expires_at| expires_at > now)
            .unwrap_or(false)
}

// ---------------------------------------------------------------------
// Linked-account path
// ---------------------------------------------------------------------

/// Where a `presented_generation` value came from -- decides how
/// [`linked_loop`] reacts when it finds the stored generation ahead of
/// what was presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationProvenance {
    /// [`access_token`]'s internal freshness check merely observed this
    /// generation moments ago; it is a baseline, not a security
    /// assertion. Finding the stored generation ahead, with no live
    /// claim, means an ordinary concurrent refresh (possibly from another
    /// broker instance entirely -- the two-pod case) already committed:
    /// adopt the new generation and keep going, never reuse.
    Observed,
    /// [`refresh`]'s caller explicitly asserted this generation. Finding
    /// the stored generation ahead, with no live claim, is the reuse
    /// determination itself.
    Asserted,
}

/// [`super::TokenBroker::access_token`]: serve the stored token
/// immediately when it is still fresh, otherwise trigger an internal
/// refresh presenting the row's own observed generation.
pub(super) async fn access_token(
    service: &TokenBrokerService,
    record_id: &str,
) -> BrokerResult<AccessToken> {
    let row = service
        .store
        .read(record_id)
        .await?
        .ok_or_else(|| BrokerError::NotFound {
            record_id: record_id.to_owned(),
        })?;
    if row.revoked_at.is_some() {
        return Err(BrokerError::Revoked {
            record_id: record_id.to_owned(),
            reused: row.revoked_reused.unwrap_or(false),
        });
    }
    let now = Utc::now();
    if row_is_fresh(&row, now) {
        return decrypt_access_token(service, &row);
    }
    ensure_fresh_linked(
        service,
        record_id,
        row.generation,
        GenerationProvenance::Observed,
    )
    .await
}

/// [`super::TokenBroker::refresh`]: force a refresh under an explicitly
/// presented generation, bypassing the freshness fast path. A stale
/// presented generation here is the reuse-detection surface.
pub(super) async fn refresh(
    service: &TokenBrokerService,
    request: RefreshRequest,
) -> BrokerResult<AccessToken> {
    ensure_fresh_linked(
        service,
        &request.record_id,
        request.presented_generation,
        GenerationProvenance::Asserted,
    )
    .await
}

async fn ensure_fresh_linked(
    service: &TokenBrokerService,
    record_id: &str,
    presented_generation: i64,
    provenance: GenerationProvenance,
) -> BrokerResult<AccessToken> {
    if service.config.single_flight {
        service
            .coalescing
            .run(
                record_id,
                linked_loop(service, record_id, presented_generation, provenance),
            )
            .await
    } else {
        linked_loop(service, record_id, presented_generation, provenance).await
    }
}

async fn linked_loop(
    service: &TokenBrokerService,
    record_id: &str,
    presented_generation: i64,
    provenance: GenerationProvenance,
) -> BrokerResult<AccessToken> {
    let deadline = overall_deadline(service);
    let mut presented_generation = presented_generation;
    loop {
        let now = Utc::now();
        let row = service
            .store
            .read(record_id)
            .await?
            .ok_or_else(|| BrokerError::NotFound {
                record_id: record_id.to_owned(),
            })?;
        if row.revoked_at.is_some() {
            return Err(BrokerError::Revoked {
                record_id: record_id.to_owned(),
                reused: row.revoked_reused.unwrap_or(false),
            });
        }

        if provenance == GenerationProvenance::Observed && row_is_fresh(&row, now) {
            // Whatever generation this settled at (ours or adopted from a
            // sibling pod), the stored token is fresh right now: done.
            return decrypt_access_token(service, &row);
        }

        if row.generation < presented_generation {
            return Err(BrokerError::Terminal {
                provider: INTERNAL,
                message: format!(
                    "presented generation {presented_generation} is ahead of the stored \
                     generation {}; a well-behaved caller can never observe this",
                    row.generation
                ),
            });
        }

        if row.generation > presented_generation {
            if row.has_live_claim(now) {
                // Someone is actively refreshing right now; the row has
                // not settled. Not (yet) a reuse determination.
                wait_a_bit(service).await;
                if Instant::now() > deadline {
                    return Err(BrokerError::LeaseTimeout {
                        record_id: record_id.to_owned(),
                    });
                }
                continue;
            }
            match provenance {
                GenerationProvenance::Observed => {
                    // An ordinary concurrent refresh already committed
                    // (possibly from another broker instance entirely --
                    // the two-pod case): adopt the new baseline and let
                    // the top-of-loop freshness check, or another lease
                    // attempt at the new generation, decide what happens
                    // next. Never reuse: this caller never asserted the
                    // old generation as a credential, it only observed it
                    // moments ago as a starting point.
                    presented_generation = row.generation;
                    continue;
                }
                GenerationProvenance::Asserted => {
                    // Quiescent and strictly past what the caller
                    // explicitly asserted holding: reuse.
                    let provider = resolve_provider(service, &row.provider)?;
                    let won = service
                        .store
                        .revoke_family_if_unrevoked(record_id, now)
                        .await?;
                    if won && let Some(hook) = &service.reuse_hook {
                        hook.on_reuse_detected(record_id, provider.name()).await;
                    }
                    return Err(BrokerError::Revoked {
                        record_id: record_id.to_owned(),
                        reused: true,
                    });
                }
            }
        }

        // row.generation == presented_generation.
        if row.has_live_claim(now) {
            wait_a_bit(service).await;
            if Instant::now() > deadline {
                return Err(BrokerError::LeaseTimeout {
                    record_id: record_id.to_owned(),
                });
            }
            continue;
        }
        let claim_id = crate::storage::random_id();
        let claim_deadline = now
            + chrono::Duration::from_std(
                service.config.provider_call_timeout + service.config.lease_grace,
            )
            .unwrap_or(chrono::Duration::seconds(15));
        let claimed = service
            .store
            .claim(
                record_id,
                presented_generation,
                &claim_id,
                claim_deadline,
                now,
            )
            .await?;
        if !claimed {
            continue;
        }
        return run_leader_linked(service, record_id, &row, presented_generation, &claim_id).await;
    }
}

async fn run_leader_linked(
    service: &TokenBrokerService,
    record_id: &str,
    row: &ProviderTokenRow,
    presented_generation: i64,
    claim_id: &str,
) -> BrokerResult<AccessToken> {
    let provider = resolve_provider(service, &row.provider)?;
    let refresh_ciphertext =
        row.refresh_ciphertext
            .clone()
            .ok_or_else(|| BrokerError::Terminal {
                provider: provider.name(),
                message: "record has no refresh token on file".to_owned(),
            })?;
    let refresh_plaintext = service
        .encryptor
        .decrypt(CryptoPurpose::RefreshToken, &refresh_ciphertext)?;
    let refresh_token_value =
        String::from_utf8(refresh_plaintext).map_err(|_| BrokerError::Terminal {
            provider: provider.name(),
            message: "stored refresh token is not valid utf-8".to_owned(),
        })?;
    let scopes = split_scopes(&row.scopes);

    let outcome = refresh::execute_with_raw(
        provider.as_ref(),
        service.transport.as_ref(),
        SecretString::from(refresh_token_value),
        &scopes,
    )
    .await;

    match outcome {
        Ok((success, raw_body)) => {
            let rotated = policy::rotated(&success);
            let new_generation = if rotated {
                presented_generation + 1
            } else {
                presented_generation
            };
            let access_ciphertext = service.encryptor.encrypt(
                CryptoPurpose::ProviderToken,
                success.access_token.expose_secret().as_bytes(),
            )?;
            let refresh_ciphertext_new = if rotated {
                let new_refresh_token = success
                    .refresh_token
                    .as_ref()
                    .expect("policy::rotated only returns true when refresh_token is Some");
                Some(service.encryptor.encrypt(
                    CryptoPurpose::RefreshToken,
                    new_refresh_token.expose_secret().as_bytes(),
                )?)
            } else {
                None
            };
            let raw_payload_ciphertext = service
                .encryptor
                .encrypt(CryptoPurpose::ProviderToken, raw_body.as_bytes())?;
            let access_expires_at = success
                .expires_in
                .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
            let scopes_out = success.scope.clone().unwrap_or_else(|| row.scopes.clone());

            let committed = service
                .store
                .commit(
                    record_id,
                    claim_id,
                    presented_generation,
                    CommitProviderToken {
                        access_ciphertext,
                        refresh_ciphertext: refresh_ciphertext_new,
                        raw_payload_ciphertext,
                        token_type: success.token_type.clone(),
                        scopes: scopes_out.clone(),
                        access_expires_at,
                        new_generation,
                    },
                )
                .await?;
            if !committed {
                return Err(BrokerError::Terminal {
                    provider: provider.name(),
                    message: "lease was reclaimed before commit; discarding this result".to_owned(),
                });
            }
            Ok(AccessToken {
                value: success.access_token,
                token_type: success.token_type,
                expires_at: access_expires_at,
                scopes: split_scopes(&scopes_out),
            })
        }
        Err(error) => {
            match policy::classify(&error, provider.refresh_policy().invalid_grant_meaning) {
                FailureClass::Reuse => {
                    let won = service
                        .store
                        .mark_revoked_by_claim(record_id, claim_id, true)
                        .await?;
                    if won && let Some(hook) = &service.reuse_hook {
                        hook.on_reuse_detected(record_id, provider.name()).await;
                    }
                    Err(BrokerError::Revoked {
                        record_id: record_id.to_owned(),
                        reused: true,
                    })
                }
                FailureClass::OrdinaryRevocation => {
                    service
                        .store
                        .mark_revoked_by_claim(record_id, claim_id, false)
                        .await?;
                    Err(BrokerError::Revoked {
                        record_id: record_id.to_owned(),
                        reused: false,
                    })
                }
                FailureClass::Retriable { retry_after } => Err(BrokerError::Retriable {
                    provider: provider.name(),
                    message: error.to_string(),
                    retry_after,
                }),
                FailureClass::Terminal => Err(BrokerError::Terminal {
                    provider: provider.name(),
                    message: error.to_string(),
                }),
            }
        }
    }
}

// ---------------------------------------------------------------------
// M2M (client-credentials) path
// ---------------------------------------------------------------------

/// [`super::TokenBroker::client_credentials`]: provision the cache entry
/// if this is its first use, then serve it fresh -- refreshing through the
/// identical claim/commit primitives the linked-account path uses, but
/// with no reuse/`invalid_grant`-dossier handling (an M2M cache entry has
/// no "family" to revoke).
pub(super) async fn client_credentials(
    service: &TokenBrokerService,
    key: M2MCacheKey,
) -> BrokerResult<AccessToken> {
    let record_id = key.record_id();
    let scopes = key.normalized_scopes();
    service
        .store
        .create_if_missing(NewProviderToken {
            id: record_id.clone(),
            provider: key.provider.clone(),
        })
        .await?;
    if service.config.single_flight {
        service
            .coalescing
            .run(&record_id, m2m_loop(service, &record_id, &scopes))
            .await
    } else {
        m2m_loop(service, &record_id, &scopes).await
    }
}

async fn m2m_loop(
    service: &TokenBrokerService,
    record_id: &str,
    scopes: &[String],
) -> BrokerResult<AccessToken> {
    let deadline = overall_deadline(service);
    loop {
        let now = Utc::now();
        let row = service
            .store
            .read(record_id)
            .await?
            .ok_or_else(|| BrokerError::NotFound {
                record_id: record_id.to_owned(),
            })?;
        if row.revoked_at.is_some() {
            return Err(BrokerError::Revoked {
                record_id: record_id.to_owned(),
                reused: row.revoked_reused.unwrap_or(false),
            });
        }

        let jitter_fraction: f64 = rand::random();
        let needs = row.access_ciphertext.is_empty()
            || cache::needs_refresh(
                row.access_expires_at,
                now,
                &service.config.m2m_cache,
                jitter_fraction,
            );
        if !needs {
            return decrypt_access_token(service, &row);
        }

        if row.has_live_claim(now) {
            wait_a_bit(service).await;
            if Instant::now() > deadline {
                return Err(BrokerError::LeaseTimeout {
                    record_id: record_id.to_owned(),
                });
            }
            continue;
        }
        let claim_id = crate::storage::random_id();
        let claim_deadline = now
            + chrono::Duration::from_std(
                service.config.provider_call_timeout + service.config.lease_grace,
            )
            .unwrap_or(chrono::Duration::seconds(15));
        let claimed = service
            .store
            .claim(record_id, row.generation, &claim_id, claim_deadline, now)
            .await?;
        if !claimed {
            continue;
        }
        return run_leader_m2m(service, record_id, &row, &claim_id, scopes).await;
    }
}

async fn run_leader_m2m(
    service: &TokenBrokerService,
    record_id: &str,
    row: &ProviderTokenRow,
    claim_id: &str,
    scopes: &[String],
) -> BrokerResult<AccessToken> {
    let provider = resolve_provider(service, &row.provider)?;
    let outcome =
        client_credentials::execute_with_raw(provider.as_ref(), service.transport.as_ref(), scopes)
            .await;
    match outcome {
        Ok((success, raw_body)) => {
            let access_ciphertext = service.encryptor.encrypt(
                CryptoPurpose::ProviderToken,
                success.access_token.expose_secret().as_bytes(),
            )?;
            let raw_payload_ciphertext = service
                .encryptor
                .encrypt(CryptoPurpose::ProviderToken, raw_body.as_bytes())?;
            let access_expires_at = success
                .expires_in
                .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds as i64));
            let scopes_out = success.scope.clone().unwrap_or_else(|| scopes.join(" "));

            let committed = service
                .store
                .commit(
                    record_id,
                    claim_id,
                    row.generation,
                    CommitProviderToken {
                        access_ciphertext,
                        refresh_ciphertext: None,
                        raw_payload_ciphertext,
                        token_type: success.token_type.clone(),
                        scopes: scopes_out.clone(),
                        access_expires_at,
                        new_generation: row.generation,
                    },
                )
                .await?;
            if !committed {
                return Err(BrokerError::Terminal {
                    provider: provider.name(),
                    message: "lease was reclaimed before commit; discarding this result".to_owned(),
                });
            }
            Ok(AccessToken {
                value: success.access_token,
                token_type: success.token_type,
                expires_at: access_expires_at,
                scopes: split_scopes(&scopes_out),
            })
        }
        Err(error) => {
            match policy::classify(&error, provider.refresh_policy().invalid_grant_meaning) {
                FailureClass::Retriable { retry_after } => Err(BrokerError::Retriable {
                    provider: provider.name(),
                    message: error.to_string(),
                    retry_after,
                }),
                // M2M cache entries have no refresh-token family: neither
                // `invalid_grant` outcome revokes anything here, it is simply
                // a terminal client-credentials failure (bad client secret,
                // disabled app, etc).
                _ => Err(BrokerError::Terminal {
                    provider: provider.name(),
                    message: error.to_string(),
                }),
            }
        }
    }
}

async fn wait_a_bit(service: &TokenBrokerService) {
    tokio::time::sleep(poll_interval(service)).await;
}

fn poll_interval(service: &TokenBrokerService) -> Duration {
    service.config.poll_interval
}
