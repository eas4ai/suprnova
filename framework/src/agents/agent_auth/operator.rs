//! What an operator runs against agent auth (AGENT-010, AGENT-011): the
//! commands `agents:revoke-all`, `agents:prune` and `agents:key`, and the
//! maintenance pass `serve` runs every five minutes.

use std::sync::Arc;
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

/// What one maintenance pass did.
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
            tracing::error!(%error, step = name, "an agent auth maintenance step failed; the pass goes on");
            failed_steps.push(name);
            0
        }
    }
}

/// Expire the registrations whose window passed, delete the replay records
/// no token can use, deliver the claim events still waiting and delete the
/// claim events delivered more than seven days ago. Every step runs
/// whatever happens to the others: a failed step is logged and named in
/// [`Maintenance::failed_steps`]. Each step is a conditional write, so every
/// node may run the pass at once.
pub(crate) async fn maintain(engine: &magnetar::agent_auth::AgentAuth) -> Maintenance {
    let mut failed_steps = Vec::new();
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
        failed_steps,
    }
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
/// task lives, when `engine` is set. A failed pass is logged, and the next
/// one tries again.
pub(crate) fn spawn_maintenance(
    engine: Option<Arc<magnetar::agent_auth::AgentAuth>>,
) -> Option<MaintenanceTask> {
    let engine = engine?;
    Some(MaintenanceTask(tokio::spawn(async move {
        let start = tokio::time::Instant::now() + MAINTENANCE_INTERVAL;
        let mut interval = tokio::time::interval_at(start, MAINTENANCE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            // A failed step was logged by the pass itself.
            let done = maintain(&engine).await;
            tracing::debug!(
                delivered = done.delivered,
                expired = done.expired,
                pruned = done.pruned,
                events_pruned = done.events_pruned,
                failed = done.failed,
                failed_steps = done.failed_steps.len(),
                "agent auth maintenance pass"
            );
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

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
