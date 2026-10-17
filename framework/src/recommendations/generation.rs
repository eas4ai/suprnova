//! Staging, publication and cleanup keep every reader on one completed snapshot.
use super::{
    Actor, EventProvenance, GenerateRun, GenerationOutcome, RecommendConfig, RecommendationRun,
    documents,
    history::{Histories, HistorySnapshots},
    ranking::{self, Ranked},
    storage,
};
use crate::{Cache, DB, FrameworkError, Viewer};
use sea_orm::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub(crate) async fn generate(request: GenerateRun) -> Result<GenerationOutcome, FrameworkError> {
    request.validate()?;
    let config = RecommendConfig::current()?;
    let ttl = Duration::from_secs(config.lock_ttl_secs);
    let Some(lock) = Cache::lock("recommendations:generate", ttl).await? else {
        return Ok(GenerationOutcome::Locked);
    };
    let id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    let result = under_lock(
        &lock,
        ttl,
        batch(&id, &request, &config, started, &lock),
        "recommendations generation",
    )
    .await;
    let result = match result {
        Ok(run) => Ok(GenerationOutcome::Completed(run)),
        Err(error) => {
            let message = error.to_string();
            match fail(&id, &message, started).await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(FrameworkError::database(format!(
                    "{message}; recommendation failure cleanup also failed: {cleanup}"
                ))),
            }
        }
    };
    let release = lock.release().await;
    match (result, release) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(_), Ok(false)) => Err(FrameworkError::internal(
            "recommendations generation cache lock expired before release",
        )),
        (Ok(result), Ok(true)) => Ok(result),
    }
}
/// Run `work` while renewing `lock` every third of its lease, so a long batch never
/// outlives the lock that keeps two runs apart; losing the lock stops the work.
pub(crate) async fn under_lock<T>(
    lock: &crate::cache::LockGuard,
    ttl: Duration,
    work: impl std::future::Future<Output = Result<T, FrameworkError>>,
    what: &str,
) -> Result<T, FrameworkError> {
    tokio::pin!(work);
    let mut heartbeat = tokio::time::interval(ttl / 3);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut work => break result,
            _ = heartbeat.tick() => {
                match lock.refresh(ttl).await {
                    Ok(true) => {},
                    Ok(false) => break Err(FrameworkError::internal(format!("{what} lost its cache lock"))),
                    Err(error) => break Err(error),
                }
            }
        }
    }
}
async fn fail(id: &str, message: &str, started: Instant) -> Result<(), FrameworkError> {
    let id = id.to_owned();
    let message = message.to_owned();
    DB::transaction(|_| Box::pin(async move {
        if storage::scalar("SELECT COUNT(*) FROM recommendation_runs WHERE id = ? AND state = ?", vec![id.clone().into(), "complete".into()]).await? > 0 { return Ok(()); }
        storage::execute("DELETE FROM starter_recommendations WHERE run_id = ?", vec![id.clone().into()]).await?;
        storage::execute("DELETE FROM recommendation_explanations WHERE run_id = ?", vec![id.clone().into()]).await?;
        storage::execute("UPDATE recommendation_runs SET state = ?, error = ?, duration_ms = ? WHERE id = ? AND state = ?",
            vec!["failed".into(), message.into(), millis(started).into(), id.into(), "running".into()]).await?;
        Ok(())
    })).await
}
fn millis(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
struct Inputs {
    histories: Histories,
    catalog: documents::Catalog,
    popularity: BTreeMap<super::RecommendationTarget, i64>,
    viewers: BTreeMap<Actor, Viewer>,
    semantic_indexes: BTreeSet<String>,
    changed: BTreeSet<Actor>,
    snapshots: HistorySnapshots,
}
async fn batch(
    id: &str,
    request: &GenerateRun,
    config: &RecommendConfig,
    started: Instant,
    lock: &crate::cache::LockGuard,
) -> Result<RecommendationRun, FrameworkError> {
    let (mut run, previous, high_water) = begin(id, request, started).await?;
    let inputs = inputs(request, config, previous.as_ref(), high_water, &mut run).await?;
    run.events_read = inputs.histories.events_read;
    run.users_covered = inputs.histories.actors.len() as i64;
    for (actor, history) in &inputs.histories.actors {
        let selected = request
            .user
            .as_ref()
            .is_none_or(|user| matches!(actor, Actor::User(id) if id == user));
        if selected
            && (previous.is_none()
                || inputs.changed.contains(actor)
                || request.user.is_some()
                || request.surface.is_some())
        {
            regenerate(
                &run.id,
                actor,
                history,
                request,
                config,
                &inputs,
                &mut run.items_scored,
            )
            .await?;
        }
        if !lock
            .refresh(Duration::from_secs(config.lock_ttl_secs))
            .await?
        {
            return Err(FrameworkError::internal(
                "recommendations generation lost its lock after a chunk",
            ));
        }
        tokio::task::yield_now().await;
    }
    statistics(&mut run, &inputs.histories).await?;
    run.duration_ms = millis(started);
    run.state = "complete".into();
    if request.user.is_some() || request.surface.is_some() {
        run.catalog_digest = previous
            .as_ref()
            .map(|old| old.catalog_digest.clone())
            .unwrap_or_default();
    }
    publish(&run, &inputs.snapshots).await?;
    Ok(run)
}
async fn begin(
    id: &str,
    request: &GenerateRun,
    started: Instant,
) -> Result<(RecommendationRun, Option<RecommendationRun>, i64), FrameworkError> {
    // A prior process may have died while staging. Its cache lease has expired now.
    for row in storage::rows(
        "SELECT id FROM recommendation_runs WHERE state = ?",
        vec!["running".into()],
    )
    .await?
    {
        fail(
            &row.get_string("id")?,
            "generation interrupted before publication",
            started,
        )
        .await?;
    }
    let previous = storage::rows("SELECT * FROM recommendation_runs WHERE state = ? ORDER BY started_at DESC, id DESC LIMIT ?", vec!["complete".into(), 1.into()]).await?
        .first().map(super::starter::run_from_row).transpose()?;
    let last = previous.as_ref().map_or(0, |run| run.high_water_mark);
    let high_water = storage::scalar(
        "SELECT COALESCE(MAX(id), 0) FROM recommendation_events",
        vec![],
    )
    .await?
    .max(last);
    let new_events = storage::scalar(
        "SELECT COUNT(*) FROM recommendation_events WHERE id > ? AND id <= ?",
        vec![last.into(), high_water.into()],
    )
    .await?;
    let last_start = storage::scalar(
        "SELECT COALESCE(MAX(started_at), 0) FROM recommendation_runs",
        vec![],
    )
    .await?;
    let run = RecommendationRun {
        id: id.into(),
        started_at: crate::clock::now()
            .timestamp_micros()
            .max(last_start.saturating_add(1)),
        duration_ms: 0,
        events_read: 0,
        new_events,
        users_covered: 0,
        items_scored: 0,
        coverage: 0.0,
        fallback_share: 0.0,
        high_water_mark: if request.user.is_some() || request.surface.is_some() {
            last
        } else {
            high_water
        },
        catalog_digest: String::new(),
        state: "running".into(),
        error: None,
    };
    storage::execute("INSERT INTO recommendation_runs (id, started_at, duration_ms, events_read, new_events, users_covered, items_scored, coverage, fallback_share, high_water_mark, state) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        vec![id.into(), run.started_at.into(), 0_i64.into(), 0_i64.into(), new_events.into(), 0_i64.into(), 0_i64.into(), 0.0.into(), 0.0.into(), run.high_water_mark.into(), "running".into()]).await?;
    if let Some(previous) = &previous {
        storage::execute("INSERT INTO starter_recommendations (run_id, actor_kind, actor_id, surface, target_index, target_key, score, rank, explanation_id) SELECT ?, actor_kind, actor_id, surface, target_index, target_key, score, rank, explanation_id FROM starter_recommendations WHERE run_id = ?", vec![id.into(), previous.id.clone().into()]).await?;
    }
    Ok((run, previous, high_water))
}
async fn inputs(
    request: &GenerateRun,
    config: &RecommendConfig,
    previous: Option<&RecommendationRun>,
    high_water: i64,
    run: &mut RecommendationRun,
) -> Result<Inputs, FrameworkError> {
    let mut histories = Histories::load(high_water, config.history).await?;
    if let Some(user) = &request.user {
        histories
            .actors
            .entry(Actor::User(user.clone()))
            .or_default()
            .surfaces
            .insert(request.surface.clone().unwrap_or("home".into()));
    }
    if let Some(previous) = previous {
        for row in storage::rows("SELECT actor_kind, actor_id, surface FROM starter_recommendations WHERE run_id = ? GROUP BY actor_kind, actor_id, surface", vec![previous.id.clone().into()]).await? {
            histories.actors.entry(storage::actor(&row)?).or_default().surfaces.insert(row.get_string("surface")?);
        }
    }
    let (catalog, viewers) = catalog(&histories).await?;
    let mut digest = Sha256::new();
    for document in catalog.values() {
        digest.update(document.content_hash("recommendations:v1").as_bytes());
        digest.update(document.source_version.to_le_bytes());
    }
    run.catalog_digest = hex::encode(digest.finalize());
    let old_snapshots = match previous {
        Some(previous) => storage::rows(
            "SELECT actor_history_snapshots FROM recommendation_runs WHERE id = ?",
            vec![previous.id.clone().into()],
        )
        .await?
        .first()
        .map(|row| row.get_optional_string("actor_history_snapshots"))
        .transpose()?
        .flatten()
        .map(|value| {
            serde_json::from_str::<HistorySnapshots>(&value).map_err(|error| {
                FrameworkError::database(format!("recommendation history snapshots: {error}"))
            })
        })
        .transpose()?
        .unwrap_or_default(),
        None => HistorySnapshots::new(),
    };
    let current_snapshots = histories.snapshots();
    let changed = if previous.is_none_or(|old| old.catalog_digest != run.catalog_digest) {
        histories.actors.keys().cloned().collect()
    } else {
        changed_actors(&histories, previous, &old_snapshots, &current_snapshots).await?
    };
    // A partial run must leave every unprocessed global invalidation pending.
    let snapshots = if request.user.is_some() || request.surface.is_some() {
        old_snapshots
    } else {
        current_snapshots
    };
    let popularity = ranking::popularity(high_water, config).await?;
    let semantic_indexes = semantic_indexes().await?;
    Ok(Inputs {
        histories,
        catalog,
        popularity,
        viewers,
        semantic_indexes,
        changed,
        snapshots,
    })
}
async fn catalog(
    histories: &Histories,
) -> Result<(documents::Catalog, BTreeMap<Actor, Viewer>), FrameworkError> {
    let mut catalog = documents::visible(&Viewer::Guest).await?;
    let provider = match crate::Auth::route_user_provider() {
        Ok(provider) => provider,
        Err(error) if crate::Auth::is_missing_provider(&error) => {
            return Ok((catalog, BTreeMap::new()));
        }
        Err(error) => return Err(error),
    };
    let mut viewers = BTreeMap::new();
    // Resolve real principals through the application's provider, never fabricate a user from an id.
    for actor in histories.actors.keys() {
        let Actor::User(id) = actor else {
            continue;
        };
        let Some(user) = provider.retrieve_by_id(id).await? else {
            continue;
        };
        let viewer = Viewer::Authenticated {
            id: user.get_auth_identifier(),
            user,
        };
        catalog.extend(documents::additional(&viewer).await?);
        viewers.insert(actor.clone(), viewer);
    }
    Ok((catalog, viewers))
}
async fn semantic_indexes() -> Result<BTreeSet<String>, FrameworkError> {
    let mut indexes = BTreeSet::new();
    for index in crate::SearchRegistry::indexes()? {
        let descriptor = (crate::SearchRegistry::by_index(index)?.descriptor)();
        let engine = crate::SearchRegistry::engine_for(index)?;
        let generation = crate::SearchRegistry::generation_for(index).await?;
        let settings = engine.read_settings(descriptor, &generation).await?;
        let name = settings
            .index_time
            .embedder
            .as_deref()
            .unwrap_or(crate::SearchConfig::current()?.embedder.as_str());
        if name != "none" && engine.capabilities().semantic {
            indexes.insert(index.to_string());
        }
    }
    Ok(indexes)
}
async fn regenerate(
    id: &str,
    actor: &Actor,
    history: &super::history::History,
    request: &GenerateRun,
    config: &RecommendConfig,
    inputs: &Inputs,
    items_scored: &mut i64,
) -> Result<(), FrameworkError> {
    let leading = leading(actor, history, config, inputs).await?;
    let candidates = ranking::fill(
        history,
        &inputs.catalog,
        &inputs.catalog,
        &inputs.popularity,
        leading,
    )?;
    *items_scored += candidates.len() as i64;
    // Retain lower-ranked tiers so read-time scope filtering can still fill the requested limit.
    for surface in &history.surfaces {
        if request
            .surface
            .as_ref()
            .is_some_and(|selected| selected != surface)
        {
            continue;
        }
        let (kind, actor_id) = actor.parts();
        storage::execute("DELETE FROM starter_recommendations WHERE run_id = ? AND actor_kind = ? AND actor_id = ? AND surface = ?",
            vec![id.into(), kind.into(), actor_id.into(), surface.clone().into()]).await?;
        stage(id, actor, surface, &candidates, &inputs.histories.evidence).await?;
    }
    Ok(())
}
async fn leading(
    actor: &Actor,
    history: &super::history::History,
    config: &RecommendConfig,
    inputs: &Inputs,
) -> Result<Vec<Ranked>, FrameworkError> {
    let mut leading = ranking::collaborative(
        actor,
        history,
        &inputs.histories,
        config.peers,
        config.min_peers,
    );
    let mut used: BTreeSet<_> = leading
        .iter()
        .map(|candidate| candidate.target.clone())
        .collect();
    let viewer = inputs.viewers.get(actor).unwrap_or(&Viewer::Guest);
    let mut neighbors = Vec::new();
    for seed in history
        .positive
        .keys()
        .filter(|target| inputs.semantic_indexes.contains(&target.index))
    {
        for mut neighbor in
            ranking::semantic(seed, &inputs.catalog, viewer, inputs.catalog.len()).await?
        {
            if history.excluded.contains(&neighbor.target) || !used.insert(neighbor.target.clone())
            {
                continue;
            }
            if let Some(positive) = history.positive.get(seed) {
                neighbor.reason.event_ids.insert(positive.event_id);
            }
            neighbors.push(neighbor);
        }
    }
    leading.extend(ranking::ordered(neighbors));
    Ok(leading)
}
async fn statistics(
    run: &mut RecommendationRun,
    histories: &Histories,
) -> Result<(), FrameworkError> {
    let mut present = BTreeSet::new();
    for row in storage::rows("SELECT actor_kind, actor_id, surface FROM starter_recommendations WHERE run_id = ? GROUP BY actor_kind, actor_id, surface", vec![run.id.clone().into()]).await? {
        present.insert((storage::actor(&row)?, row.get_string("surface")?));
    }
    let mut pairs = 0_i64;
    let mut covered = 0_i64;
    for (actor, history) in &histories.actors {
        for surface in &history.surfaces {
            pairs += 1;
            if present.contains(&(actor.clone(), surface.clone())) {
                covered += 1;
            }
        }
    }
    run.coverage = if pairs == 0 {
        0.0
    } else {
        covered as f64 / pairs as f64
    };
    let total = storage::scalar(
        "SELECT COUNT(*) FROM starter_recommendations WHERE run_id = ?",
        vec![run.id.clone().into()],
    )
    .await?;
    let fallback = storage::scalar("SELECT COUNT(*) FROM starter_recommendations r JOIN recommendation_explanations e ON e.id = r.explanation_id WHERE r.run_id = ? AND e.fallback_tier IN (?, ?, ?)", vec![run.id.clone().into(), "affinity".into(), "popularity".into(), "recent".into()]).await?;
    run.fallback_share = if total == 0 {
        0.0
    } else {
        fallback as f64 / total as f64
    };
    Ok(())
}
async fn changed_actors(
    histories: &Histories,
    previous: Option<&RecommendationRun>,
    old: &HistorySnapshots,
    current: &HistorySnapshots,
) -> Result<BTreeSet<Actor>, FrameworkError> {
    let mut changed = BTreeSet::new();
    let keys: BTreeSet<_> = old.keys().chain(current.keys()).collect();
    for key in keys {
        if old.get(key) == current.get(key) {
            continue;
        }
        changed.insert(super::history::actor_from_key(key)?);
        // Old dependencies also invalidate peers when retention or the cap removes a shared target.
        for snapshot in old.get(key).into_iter().chain(current.get(key)) {
            for target in &snapshot.positive_targets {
                if let Some(peers) = histories.by_target.get(target) {
                    changed.extend(peers.iter().map(|(actor, _)| actor.clone()));
                }
            }
        }
    }
    if let Some(previous) = previous {
        for row in storage::rows("SELECT DISTINCT r.actor_kind, r.actor_id FROM starter_recommendations r JOIN recommendation_explanations e ON e.id = r.explanation_id WHERE r.run_id = ? AND e.fallback_tier IN (?, ?, ?)", vec![previous.id.clone().into(), "affinity".into(), "popularity".into(), "recent".into()]).await? {
            changed.insert(storage::actor(&row)?);
        }
    }
    // Actual empty prior results must recover when the catalog grows without another event.
    let mut represented = BTreeSet::new();
    if let Some(previous) = previous {
        for row in storage::rows(
            "SELECT DISTINCT actor_kind, actor_id FROM starter_recommendations WHERE run_id = ?",
            vec![previous.id.clone().into()],
        )
        .await?
        {
            represented.insert(storage::actor(&row)?);
        }
    }
    for actor in histories.actors.keys() {
        if !represented.contains(actor) {
            changed.insert(actor.clone());
        }
    }
    Ok(changed)
}
async fn stage(
    id: &str,
    actor: &Actor,
    surface: &str,
    candidates: &[Ranked],
    evidence: &BTreeMap<i64, EventProvenance>,
) -> Result<(), FrameworkError> {
    let (kind, actor_id) = actor.parts();
    for (chunk_index, chunk) in candidates.chunks(100).enumerate() {
        let mut explanation_values = Vec::<Value>::new();
        let mut recommendation_values = Vec::<Value>::new();
        for (offset, candidate) in chunk.iter().enumerate() {
            let explanation_id = uuid::Uuid::new_v4().to_string();
            let refs: Vec<_> = candidate
                .reason
                .event_ids
                .iter()
                .filter_map(|id| evidence.get(id))
                .collect();
            if refs.len() != candidate.reason.event_ids.len() {
                return Err(FrameworkError::database(
                    "recommendation evidence was removed during generation; retry the batch",
                ));
            }
            explanation_values.extend([
                explanation_id.clone().into(),
                id.into(),
                storage::encode(&refs)?.into(),
                storage::encode(&candidate.reason.attributes)?.into(),
                (candidate.reason.peers as i64).into(),
                (candidate.reason.shared.len() as i64).into(),
                storage::encode(&candidate.reason.shared)?.into(),
                candidate.reason.tier.clone().into(),
            ]);
            recommendation_values.extend([
                id.into(),
                kind.into(),
                actor_id.into(),
                surface.into(),
                candidate.target.index.clone().into(),
                candidate.target.key.clone().into(),
                candidate.score.into(),
                ((chunk_index * 100 + offset + 1) as i64).into(),
                explanation_id.into(),
            ]);
        }
        storage::execute(&format!("INSERT INTO recommendation_explanations (id, run_id, event_ids, attribute_values, peer_count, shared_target_count, shared_targets, fallback_tier) VALUES {}", vec!["(?, ?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ")), explanation_values).await?;
        storage::execute(&format!("INSERT INTO starter_recommendations (run_id, actor_kind, actor_id, surface, target_index, target_key, score, rank, explanation_id) VALUES {}", vec!["(?, ?, ?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ")), recommendation_values).await?;
    }
    Ok(())
}
async fn publish(
    run: &RecommendationRun,
    snapshots: &HistorySnapshots,
) -> Result<(), FrameworkError> {
    let run = run.clone();
    let snapshots = storage::encode(snapshots)?;
    DB::transaction(|_| Box::pin(async move {
        // One publication statement makes every staged row visible together.
        let updated = storage::execute("UPDATE recommendation_runs SET state = ?, duration_ms = ?, events_read = ?, new_events = ?, users_covered = ?, items_scored = ?, coverage = ?, fallback_share = ?, high_water_mark = ?, catalog_digest = ?, actor_history_snapshots = ? WHERE id = ? AND state = ?",
            vec!["complete".into(), run.duration_ms.into(), run.events_read.into(), run.new_events.into(), run.users_covered.into(), run.items_scored.into(), run.coverage.into(), run.fallback_share.into(), run.high_water_mark.into(), run.catalog_digest.clone().into(), snapshots.into(), run.id.clone().into(), "running".into()]).await?;
        if updated != 1 { return Err(FrameworkError::database("recommendation run is no longer available for publication")); }
        storage::execute("DELETE FROM starter_recommendations WHERE run_id <> ?", vec![run.id.clone().into()]).await?;
        storage::execute("UPDATE recommendation_runs SET actor_history_snapshots = ? WHERE id <> ?", vec![sea_orm::Value::String(None), run.id.clone().into()]).await?;
        storage::execute("DELETE FROM recommendation_explanations WHERE NOT EXISTS (SELECT 1 FROM starter_recommendations WHERE explanation_id = recommendation_explanations.id)", vec![]).await?;
        Ok(())
    })).await
}
