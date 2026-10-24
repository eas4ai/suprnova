//! What an operator runs against the OAuth server and agent auth (AGENT-010,
//! AGENT-011): the commands `agents:revoke-all`, `agents:prune` and
//! `agents:key`, and the maintenance pass `serve` runs every five minutes
//! whenever the router mounts the OAuth server, with agent auth or without.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::FrameworkError;

/// How often `serve` runs the maintenance pass.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Revoke every agent registration of the application and every token
/// issued from them. Returns how many registrations this run revoked;
/// running it again finishes what a failed run left, a registration whose
/// grant revocation failed included.
pub(crate) async fn revoke_all(
    engine: &magnetar::agent_auth::AgentAuth,
) -> Result<u64, FrameworkError> {
    engine
        .revoke_all_agent_credentials()
        .await
        .map_err(|error| {
            FrameworkError::internal(format!(
                "revoking every agent credential failed partway; what was revoked stays revoked, \
                 and running agents:revoke-all again finishes the rest: {error}"
            ))
        })
}

/// What one maintenance pass did. The agent auth counts are 0 when agent
/// auth is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Maintenance {
    /// Unclaimed registrations whose window passed, now expired with their
    /// tokens ended and `registration.expired` recorded.
    pub(crate) expired: u64,
    /// Replay records of provider tokens that can no longer be replayed.
    pub(crate) pruned: u64,
    /// Claim events a failed dispatch or a stopped process left, now
    /// delivered.
    pub(crate) delivered: u64,
    /// Claim events delivered longer ago than the outbox keeps them.
    pub(crate) events_pruned: u64,
    /// Claim events that failed and wait to be put back.
    pub(crate) failed: u64,
    /// Rows of the OAuth server's tables that can no longer be used.
    pub(crate) oauth_pruned: u64,
    /// Whether agent auth is on, so its steps ran.
    pub(crate) agent_auth: bool,
    /// The steps that failed, each already logged at error level.
    pub(crate) failed_steps: Vec<&'static str>,
}

/// Run one step of the pass: its count, or 0 after logging its error and
/// naming it in `failed_steps`, so the steps after it still run.
async fn step(
    name: &'static str,
    failed_steps: &mut Vec<&'static str>,
    run: impl std::future::Future<Output = magnetar::Result<u64>>,
) -> u64 {
    match run.await {
        Ok(count) => count,
        Err(error) => {
            tracing::error!(%error, step = name, "a maintenance step failed; the pass goes on");
            failed_steps.push(name);
            0
        }
    }
}

/// Prune the OAuth server's rows that can no longer be used, then, when
/// agent auth is on, expire the registrations whose window passed, delete
/// the replay records no token can use, deliver the claim events still
/// waiting and delete the claim events delivered more than seven days ago.
///
/// The OAuth server's pruning is Magnetar's
/// [`prune_expired`](magnetar::authorization_server::AuthorizationServer::prune_expired):
/// pending authorizations and codes that lapsed and tokens that expired,
/// then grants revoked longer ago than the server's grant retention (30
/// days) with their tokens, and grants left with no token whose last use is
/// older than it. A live grant or token is never pruned.
///
/// Every step runs whatever happens to the others: a failed step is logged
/// and named in [`Maintenance::failed_steps`]. Each step is a conditional
/// write, so every node may run the pass at once.
pub(crate) async fn maintain(mounted: &crate::agents::MountedOAuth) -> Maintenance {
    let mut failed_steps = Vec::new();
    let oauth_pruned = step(
        "pruning the OAuth server's ended rows",
        &mut failed_steps,
        mounted.server.prune_expired(),
    )
    .await;
    let Some(agent) = &mounted.agent else {
        return Maintenance {
            expired: 0,
            pruned: 0,
            delivered: 0,
            events_pruned: 0,
            failed: 0,
            oauth_pruned,
            agent_auth: false,
            failed_steps,
        };
    };
    let engine = &agent.engine;
    let expired = step(
        "expiring agent registrations",
        &mut failed_steps,
        engine.expire_registrations(),
    )
    .await;
    let pruned = step(
        "pruning replay records",
        &mut failed_steps,
        engine.prune_replay_records(),
    )
    .await;
    let delivered = step(
        "delivering claim events",
        &mut failed_steps,
        engine.deliver_pending_events(),
    )
    .await;
    let events_pruned = step(
        "pruning delivered claim events",
        &mut failed_steps,
        engine.prune_delivered_events(),
    )
    .await;
    let failed = step(
        "counting failed claim events",
        &mut failed_steps,
        engine.failed_events(),
    )
    .await;
    Maintenance {
        expired,
        pruned,
        delivered,
        events_pruned,
        failed,
        oauth_pruned,
        agent_auth: true,
        failed_steps,
    }
}

/// What `agents:prune` prints about a pass, one line each: the OAuth
/// server's count, then agent auth's when it is on.
pub(crate) fn report(done: &Maintenance) -> Vec<String> {
    let mut lines = vec![format!(
        "Pruned {} rows of the OAuth server that can no longer be used: lapsed authorizations, \
         expired tokens and grants ended longer ago than their retention.",
        done.oauth_pruned
    )];
    if !done.agent_auth {
        return lines;
    }
    lines.push(format!(
        "Delivered {} claim events, expired {} agent registrations, and pruned {} replay \
         records and {} delivered claim events.",
        done.delivered, done.expired, done.pruned, done.events_pruned
    ));
    if done.failed > 0 {
        lines.push(format!(
            "{} claim events failed after their last delivery attempt; fix what refused them, \
             then run `agents:prune --retry-failed`.",
            done.failed
        ));
    }
    lines
}

/// Put the claim events that failed after their last delivery attempt back
/// with a fresh count, for `agents:prune --retry-failed`. Returns how many.
pub(crate) async fn retry_failed(
    engine: &magnetar::agent_auth::AgentAuth,
) -> Result<u64, FrameworkError> {
    engine.retry_failed_events().await.map_err(|error| {
        FrameworkError::internal(format!("putting failed claim events back failed: {error}"))
    })
}

/// A new signing key: its identifier and its P-256 private key in PKCS#8
/// PEM, as `agents:key` prints them.
pub(crate) fn new_key() -> Result<(String, SecretString), FrameworkError> {
    let pem = magnetar::agent_auth::SigningKey::generate_pem()
        .map_err(|error| FrameworkError::internal(format!("generate a signing key: {error}")))?;
    let mut random = [0_u8; 4];
    getrandom::fill(&mut random).map_err(|error| {
        FrameworkError::internal(format!("operating system randomness: {error}"))
    })?;
    let kid = format!(
        "agent-{}-{}",
        crate::clock::now().format("%Y%m%d"),
        hex::encode(random)
    );
    Ok((kid, pem))
}

/// The two environment lines `agents:key` prints, the PEM's line breaks
/// written as `\n` so the key fits one line of an environment file.
pub(crate) fn environment_lines(kid: &str, pem: &SecretString) -> String {
    let one_line = pem.expose_secret().trim_end().replace('\n', "\\n");
    format!(
        "{}={kid}\n{}=\"{one_line}\"\n",
        super::KEY_ID_VARIABLE,
        super::PRIVATE_KEY_VARIABLE
    )
}

/// Stops the maintenance task when dropped, so it ends with the server.
pub(crate) struct MaintenanceTask(tokio::task::JoinHandle<()>);

impl Drop for MaintenanceTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Run the maintenance pass every five minutes for as long as the returned
/// task lives, when `router` mounts the OAuth server, with agent auth or
/// without. A failed pass is logged, and the next one tries again.
pub(crate) fn spawn_maintenance(router: &crate::routing::Router) -> Option<MaintenanceTask> {
    spawn_maintenance_every(router, MAINTENANCE_INTERVAL)
}

/// [`spawn_maintenance`], every `every`.
fn spawn_maintenance_every(
    router: &crate::routing::Router,
    every: Duration,
) -> Option<MaintenanceTask> {
    let mounted = router.oauth_server()?;
    Some(MaintenanceTask(tokio::spawn(async move {
        let start = tokio::time::Instant::now() + every;
        let mut interval = tokio::time::interval_at(start, every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            // A failed step was logged by the pass itself.
            let done = maintain(&mounted).await;
            tracing::debug!(
                oauth_pruned = done.oauth_pruned,
                delivered = done.delivered,
                expired = done.expired,
                pruned = done.pruned,
                events_pruned = done.events_pruned,
                failed = done.failed,
                failed_steps = done.failed_steps.len(),
                "agents maintenance pass"
            );
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(oauth_pruned: u64, agent_auth: bool) -> Maintenance {
        Maintenance {
            expired: 1,
            pruned: 2,
            delivered: 3,
            events_pruned: 4,
            failed: 0,
            oauth_pruned,
            agent_auth,
            failed_steps: Vec::new(),
        }
    }

    #[test]
    fn agent_005_prune_reports_the_oauth_servers_count() {
        let oauth_only = report(&done(5, false));
        assert_eq!(
            oauth_only,
            vec![
                "Pruned 5 rows of the OAuth server that can no longer be used: lapsed \
                 authorizations, expired tokens and grants ended longer ago than their retention."
                    .to_owned()
            ],
            "with only the OAuth server, its count and nothing about agent auth"
        );
        let both = report(&done(5, true));
        assert_eq!(both.len(), 2, "{both:?}");
        assert!(
            both[0].starts_with("Pruned 5 rows of the OAuth server"),
            "{both:?}"
        );
        assert!(both[1].starts_with("Delivered 3 claim events"), "{both:?}");
    }

    #[tokio::test]
    async fn agent_005_the_serve_pass_runs_with_only_the_oauth_server() {
        use magnetar::authorization_server::sql::tokens;
        use magnetar::authorization_server::{
            AuthorizationStore, GrantRecord, NewTokenRecord, SqlAuthorizationStore,
        };
        use sea_orm::{EntityTrait, PaginatorTrait};

        crate::config::Config::register(
            crate::config::AppConfig::builder()
                .url("http://localhost")
                .build(),
        );
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        crate::agents::OAuthServer::migrate(&db)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let router = crate::routing::Router::new()
            .try_agents(
                crate::agents::AgentsConfig::new()
                    .oauth_server(crate::agents::OAuthServer::new().connection(db.clone())),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let now = crate::clock::now();
        SqlAuthorizationStore::new(db.clone())
            .create_grant(
                GrantRecord {
                    id: "grant-1".to_owned(),
                    kind: magnetar::authorization_server::USER_GRANT_KIND.to_owned(),
                    user_id: Some("7".to_owned()),
                    client_id: "client".to_owned(),
                    client_name: "Client".to_owned(),
                    client_host: "client.test".to_owned(),
                    scopes: Vec::new(),
                    resource: "http://localhost/mcp".to_owned(),
                    auth_epoch: None,
                    access_kind: "access".to_owned(),
                    access_lifetime: None,
                    created_at: now,
                    last_used_at: None,
                    revoked_at: None,
                },
                vec![NewTokenRecord {
                    id: "token-1".to_owned(),
                    kind: "access".to_owned(),
                    token_hash: "expired".to_owned(),
                    scopes: Vec::new(),
                    expires_at: now - chrono::Duration::hours(1),
                }],
            )
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let task = spawn_maintenance_every(&router, Duration::from_millis(50));
        assert!(
            task.is_some(),
            "a router with only the OAuth server runs the pass"
        );
        let mut left = u64::MAX;
        for _ in 0..100 {
            left = tokens::Entity::find()
                .count(&db)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            if left == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(left, 0, "the pass pruned the expired token");
        drop(task);
    }

    #[test]
    fn agent_007_new_key_prints_as_two_environment_lines_that_read_back() {
        let (kid, pem) = new_key().expect("a key");
        assert!(kid.starts_with("agent-"), "{kid}");
        let lines = environment_lines(&kid, &pem);
        let mut rows = lines.lines();
        assert_eq!(
            rows.next(),
            Some(format!("AGENT_AUTH_KEY_ID={kid}").as_str())
        );
        let key_row = rows.next().expect("the key row");
        let value = key_row
            .strip_prefix("AGENT_AUTH_PRIVATE_KEY=\"")
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("a quoted value");
        assert!(!value.contains('\n'), "one line");
        assert!(
            super::super::read_key(kid.clone(), value).is_ok(),
            "the line reads back as the key"
        );
        assert_eq!(rows.next(), None);
    }
}
