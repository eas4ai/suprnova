//! Index dependencies use reserved table names, independent of physical generations.
//!
//! Table names prefixed with `search:` are reserved for logical indexes. Both observers and
//! writers build it here so a generation switch invalidates the same identity
//! as a document write, without evicting another index's representations.
use super::{SearchConfig, SearchRegistry};
use crate::{
    FrameworkError,
    render_cache::{collector, orm},
};
use std::future::Future;
use suprnova_live::render_cache::generation::DependencyIdentity;

tokio::task_local! { static INDEX_READ: (); static COORDINATED_WRITE: String; static INITIALIZING_READ: (); }

/// Build the stable index identity shared by readers and every mutation.
pub(crate) fn identity(index: &str) -> Result<DependencyIdentity, FrameworkError> {
    DependencyIdentity::try_table(&format!("search:{index}"))
        .map_err(|error| FrameworkError::internal(format!("search dependency: {error:?}")))
}

/// Attribute search dependencies and replay restrictions to the active render.
pub(crate) fn observe(
    index: &str,
    explicit_viewer: bool,
    opt_out: bool,
) -> Result<(), FrameworkError> {
    if !collector::is_active() {
        return Ok(());
    }
    let descriptor = (SearchRegistry::by_index(index)?.descriptor)();
    collector::observe_search_read(
        explicit_viewer || descriptor.scope.is_some(),
        SearchConfig::current()?.analytics,
        opt_out,
    );
    Ok(())
}

// The scope covers only driver reads. Hydration, raw application SQL and
// sibling futures keep their ordinary accounting even while a read awaits.
/// Account only the driver read, preserving ordinary hydration and raw SQL hooks.
pub(crate) async fn read<F: Future>(index: &str, future: F) -> Result<F::Output, FrameworkError> {
    if !collector::is_active() {
        return Ok(future.await);
    }
    observe(index, false, false)?;
    collector::anchor_search_snapshot().await?;
    Ok(INDEX_READ.scope((), future).await)
}

/// Keep a first empty collection read from probing for a second pooled connection.
///
/// An explicit caller transaction may own the only connection without being
/// ambient. Lazy read initialization follows the ledger's held-connection gate;
/// a later document write probes normally and advances the initialized index.
pub(crate) async fn generation_for_read(
    index: &str,
) -> Result<super::GenerationId, FrameworkError> {
    INITIALIZING_READ
        .scope((), Box::pin(SearchRegistry::generation_for(index)))
        .await
}

/// Let database driver SQL join the snapshot after its index was observed.
pub(crate) fn accounted_read() -> bool {
    INDEX_READ.try_with(|()| ()).is_ok()
}

/// Reuse ORM advancement and its failure protection for a nontransactional write.
pub(crate) async fn advance(index: &str) -> Result<(), FrameworkError> {
    // ORM advancement joins the ambient default when opening its dedicated
    // transaction. Search's nontransactional path must name primary explicitly.
    crate::DB::with_default_connection(
        crate::database::PRIMARY_CONNECTION_NAME,
        orm::advance(vec![identity(index)?]),
    )
    .await
}

// External engines cannot roll back with SQL. Keep the same cancellation
// and failed-advance protection as an ORM write on a separate connection.
struct UnfinishedWrite(Option<DependencyIdentity>);
impl Drop for UnfinishedWrite {
    fn drop(&mut self) {
        if let Some(identity) = self.0.take() {
            crate::render_cache::write_side::suspend_serving(&[identity]);
        }
    }
}

/// Protect writes whose external storage cannot roll back with the ledger.
pub(crate) async fn external_write<T>(
    index: &str,
    write: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<T, FrameworkError> {
    if COORDINATED_WRITE
        .try_with(|owner| owner == index)
        .unwrap_or(false)
    {
        return write.await;
    }
    // Resolve the gate before a mutation holds a database connection.
    let caller_may_hold_connection = crate::database::after_commit::in_transaction()
        || INITIALIZING_READ.try_with(|()| ()).is_ok();
    if !crate::render_cache::write_side_open(caller_may_hold_connection).await? {
        return write.await;
    }
    let mut pending = UnfinishedWrite(Some(identity(index)?));
    let deferred = defer_if_ambient(index).await?;
    let result = write.await;
    // Errors may follow partial external mutations, so advance on either result.
    if !deferred {
        advance(index).await?;
    }
    pending.0 = None;
    result
}

// A generation coordinator owns both the engine mutation and its SQL commit.
// It advances through that handle; suppress a nested engine's standalone advance.
/// Let a coordinator advance through its own transaction without nested advances.
pub(crate) async fn coordinated_write<T>(
    index: &str,
    write: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<T, FrameworkError> {
    if !crate::render_cache::write_side_open(crate::database::after_commit::in_transaction())
        .await?
    {
        return write.await;
    }
    let owns_coordination = COORDINATED_WRITE.try_with(|_| ()).is_err();
    let mut pending = UnfinishedWrite(Some(identity(index)?));
    let deferred = defer_if_ambient(index).await?;
    let result = COORDINATED_WRITE.scope(index.to_owned(), write).await;
    if !deferred
        && (result.is_err()
            || crate::DB::default_connection() != crate::database::PRIMARY_CONNECTION_NAME)
    {
        // External storage may have changed before a metadata failure; named
        // metadata cannot share its transaction with the primary ledger.
        advance(index).await?;
    }
    if owns_coordination
        && !deferred
        && result.is_ok()
        && crate::render_cache::write_side::serving_suspended()
    {
        // Like ORM atomic writes, a confirmed raw commit repairs any older
        // missed advances before this process resumes serving cached entries.
        let repair =
            crate::DB::with_default_connection(crate::database::PRIMARY_CONNECTION_NAME, async {
                orm::repair_unresolved().await;
                Ok::<(), FrameworkError>(())
            })
            .await;
        if let Err(error) = repair {
            tracing::warn!(target: "suprnova::render_cache", error = %error,
                "search generation repair could not select primary; cached entries remain unserved");
        }
    }
    pending.0 = None;
    result
}

/// Advance through a primary driver transaction before its data commits.
pub(crate) async fn advance_transaction(
    tx: &sea_orm::DatabaseTransaction,
    index: &str,
) -> Result<(), FrameworkError> {
    if crate::DB::default_connection() == crate::database::PRIMARY_CONNECTION_NAME {
        crate::render_cache::ledger::advance_in_borrowed_transaction(tx, &[identity(index)?]).await
    } else {
        // Named database mutations are guarded and advanced after their own
        // commit by named_write, like model writes on another connection.
        Ok(())
    }
}

/// Keep named database mutations protected until the primary ledger catches up.
pub(crate) async fn named_write<T>(
    index: &str,
    write: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<T, FrameworkError> {
    if crate::DB::default_connection() == crate::database::PRIMARY_CONNECTION_NAME {
        coordinated_write(index, write).await
    } else {
        external_write(index, write).await
    }
}

// External state can commit while its caller still owns an ambient SQL
// transaction. Suspend replay until that transaction ends, even if it rolls
// back. Savepoint rollback reattaches the repair to the surviving parent.
async fn defer_if_ambient(index: &str) -> Result<bool, FrameworkError> {
    if !crate::database::after_commit::in_transaction() {
        return Ok(false);
    }
    crate::render_cache::write_side::suspend_serving(&[identity(index)?]);
    defer_advance(index).await?;
    Ok(true)
}

async fn defer_advance(index: &str) -> Result<(), FrameworkError> {
    let committed = index.to_owned();
    crate::DB::after_commit(move || finish_external_write(committed)).await?;
    let rolled_back = index.to_owned();
    crate::DB::after_rollback(move || finish_external_write(rolled_back)).await
}

fn finish_external_write(
    index: String,
) -> std::pin::Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    Box::pin(async move {
        if crate::database::after_commit::in_transaction() {
            defer_advance(&index).await
        } else {
            advance(&index).await
        }
    })
}

// A prune can affect several indexes on a named database. Keep every one
// protected until its transaction has committed and the primary has advanced.
/// Protect each affected index during a multi-index named database mutation.
pub(crate) async fn named_write_many<T>(
    indexes: &[String],
    write: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<T, FrameworkError> {
    if crate::DB::default_connection() == crate::database::PRIMARY_CONNECTION_NAME
        || !crate::render_cache::write_side_open(crate::database::after_commit::in_transaction())
            .await?
    {
        return write.await;
    }
    let identities = indexes
        .iter()
        .map(|index| identity(index))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pending = identities
        .iter()
        .cloned()
        .map(|identity| UnfinishedWrite(Some(identity)))
        .collect::<Vec<_>>();
    let mut deferred = false;
    for index in indexes {
        deferred |= defer_if_ambient(index).await?;
    }
    let result = write.await;
    if !deferred {
        crate::DB::with_default_connection(
            crate::database::PRIMARY_CONNECTION_NAME,
            orm::advance(identities),
        )
        .await?;
    }
    for guard in &mut pending {
        guard.0 = None;
    }
    result
}

// A process can enable analytics while entries from an analytics-off request
// still exist. Recheck search eligibility before every replay, including L1.
/// Prevent analytics and viewer-scoped searches from replaying older stored entries.
pub(crate) fn replay_allowed(
    observed: &suprnova_live::render_cache::generation::GenerationSet,
) -> bool {
    type IndexIdentity = ([u8; 32], bool);
    static INDEXES: std::sync::LazyLock<Result<Vec<IndexIdentity>, FrameworkError>> =
        std::sync::LazyLock::new(|| {
            inventory::iter::<super::registry::SearchableEntry>
                .into_iter()
                .map(|entry| {
                    identity(entry.index)
                        .map(|identity| (identity.digest(), (entry.descriptor)().scope.is_some()))
                })
                .collect()
        });
    let Ok(indexes) = INDEXES.as_ref() else {
        return false;
    };
    let mut searched = false;
    for (digest, scoped) in indexes {
        if observed.get_digest(digest).is_some() {
            if *scoped {
                return false;
            }
            searched = true;
        }
    }
    !searched || SearchConfig::current().is_ok_and(|config| !config.analytics)
}
