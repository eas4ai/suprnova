//! The MongoDB session store, behind the `database-mongodb` feature, as
//! the database session driver on SQL and `laravel-mongodb`'s session store
//! (PAR-188).
//!
//! `SESSION_DRIVER=mongodb` makes the session middleware keep sessions in
//! the `sessions` collection (or the one `SESSION_TABLE` names) on the
//! default MongoDB connection (or the one `SESSION_CONNECTION` names).

use std::time::Duration;

use ::bson::{Bson, Document, doc};
use ::mongodb::Collection;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::OnceCell;

use super::database::{Payload, decode_payload, encode_payload};
use crate::error::FrameworkError;
use crate::mongodb::stores::{
    StoreIndex, bson_time, create_indexes, is_duplicate_key, store_error, surely_not_applied,
    validate_collection_name,
};
use crate::mongodb::{Mongo, MongoConnection};
use crate::session::config::SessionConfig;
use crate::session::store::{DestroyedSessions, SessionData, SessionMigrationError, SessionStore};

/// The collection [`MongoSessionDriver::new`] keeps sessions in, Laravel's
/// `sessions`.
pub const DEFAULT_MONGO_SESSIONS_COLLECTION: &str = "sessions";

/// How the store's errors name it.
const STORE: &str = "the MongoDB session store";

/// Where the driver finds its connection.
enum Source {
    /// The `Mongo` facade's connection of this name, or the default one,
    /// looked up on each call: the middleware is built in the
    /// application's bootstrap, before the boot registers the connections.
    Facade(Option<String>),
    /// A connection given to the driver.
    Connection(MongoConnection),
}

/// [`SessionStore`] over a MongoDB collection, Laravel's `sessions` by
/// default.
///
/// A session is one document: `session_id` (the session id, under a
/// unique index), `user_id` (the default guard's user, or null), `guards`
/// (each other guard signed in, as `{guard, user_id}`), `payload` (the data
/// and the CSRF token, in the database driver's format) and
/// `last_activity` (a BSON datetime). The indexes are created on the first
/// operation.
///
/// The session id is a field rather than `_id`, because `_id` cannot
/// change: [`SessionStore::migrate_two_factor_session`] renames the session
/// to its new id with one update of one document, which the server applies
/// atomically, so the two-factor promotion needs no transaction and works
/// on a standalone server.
///
/// A session loaded from the store is written back with an update that
/// creates nothing, so a session a revocation removed between a request's
/// read and its write is not recreated, as the database driver does.
/// [`SessionStore::gc`] removes the sessions whose `last_activity` is
/// older than the lifetime, and a read of such a session answers none.
pub struct MongoSessionDriver {
    lifetime: Duration,
    collection: String,
    source: Source,
    indexes: OnceCell<()>,
}

impl MongoSessionDriver {
    /// The driver over [`DEFAULT_MONGO_SESSIONS_COLLECTION`] on the default
    /// MongoDB connection, which it looks up on each call.
    pub fn new(lifetime: Duration) -> Self {
        Self::from_parts(
            lifetime,
            DEFAULT_MONGO_SESSIONS_COLLECTION,
            Source::Facade(None),
        )
    }

    /// The driver `SESSION_DRIVER=mongodb` builds: over the collection
    /// [`SessionConfig::table_name`] names, on the MongoDB connection
    /// [`SessionConfig::connection`] names or the default one, with the
    /// configured lifetime. A collection name MongoDB refuses fails the
    /// first call, naming it.
    pub fn from_config(config: &SessionConfig) -> Self {
        Self::from_parts(
            config.lifetime,
            &config.table_name,
            Source::Facade(config.connection.clone()),
        )
    }

    /// The driver over the collection `collection` on `connection`.
    ///
    /// # Errors
    ///
    /// When MongoDB would refuse the name: empty, holding `$` or a NUL
    /// character, or starting with `system.`.
    pub fn with_connection(
        lifetime: Duration,
        connection: &MongoConnection,
        collection: &str,
    ) -> Result<Self, FrameworkError> {
        validate_collection_name(collection, "the sessions")?;
        Ok(Self::from_parts(
            lifetime,
            collection,
            Source::Connection(connection.clone()),
        ))
    }

    fn from_parts(lifetime: Duration, collection: &str, source: Source) -> Self {
        Self {
            lifetime,
            collection: collection.to_owned(),
            source,
            indexes: OnceCell::new(),
        }
    }

    /// The lifetime in milliseconds, capped as the database driver caps
    /// it, so the deadline arithmetic stays in range.
    fn lifetime_millis(&self) -> i64 {
        let seconds = self
            .lifetime
            .as_secs()
            .min(crate::session::MAX_SESSION_LIFETIME_SECS);
        i64::try_from(seconds)
            .unwrap_or(i64::MAX)
            .saturating_mul(1000)
    }

    /// The sessions collection, once its indexes exist.
    async fn ready(&self) -> Result<Collection<Document>, FrameworkError> {
        validate_collection_name(&self.collection, "the sessions")?;
        let connection = match &self.source {
            Source::Connection(connection) => connection.clone(),
            Source::Facade(name) => {
                let found = match name {
                    Some(name) => Mongo::connection_named(name),
                    None => Mongo::connection(),
                };
                found.map_err(|error| {
                    FrameworkError::internal(format!(
                        "SESSION_DRIVER=mongodb has no MongoDB connection: {error}"
                    ))
                })?
            }
        };
        let collection = connection.collection::<Document>(&self.collection);
        self.indexes
            .get_or_try_init(|| {
                create_indexes(
                    STORE,
                    &collection,
                    vec![
                        StoreIndex::unique(doc! { "session_id": 1 }),
                        StoreIndex::on(doc! { "user_id": 1 }),
                        StoreIndex::on(doc! { "guards.guard": 1, "guards.user_id": 1 }),
                        StoreIndex::on(doc! { "last_activity": 1 }),
                    ],
                )
            })
            .await?;
        Ok(collection)
    }

    /// The update a write of `session` at `now` sends, as `filter`,
    /// `update` and `upsert`: a session loaded from the store is updated
    /// only, a new one is created.
    #[doc(hidden)]
    pub fn rendered_write(
        &self,
        session: &SessionData,
        now: DateTime<Utc>,
    ) -> Result<Document, FrameworkError> {
        let (filter, update) = write_parts(session, now)?;
        Ok(doc! { "filter": filter, "update": update, "upsert": !session.loaded_from_store })
    }

    /// The update that renames the session `old_id` to `session`, as
    /// `filter` and `update`.
    #[doc(hidden)]
    pub fn rendered_migration(
        &self,
        old_id: &str,
        session: &SessionData,
        now: DateTime<Utc>,
    ) -> Result<Document, FrameworkError> {
        let (_, update) = write_parts(session, now)?;
        Ok(doc! { "filter": { "session_id": old_id }, "update": update })
    }

    /// The delete a collection at `now` sends.
    #[doc(hidden)]
    pub fn rendered_gc(&self, now: DateTime<Utc>) -> Document {
        doc! { "filter": self.gc_filter(now) }
    }

    /// The sessions this driver wrote whose last activity is older than
    /// the lifetime at `now`. A session written by another store sharing
    /// the collection has no `session_id`, and is left to that store.
    fn gc_filter(&self, now: DateTime<Utc>) -> Document {
        let cutoff = ::bson::DateTime::from_millis(
            now.timestamp_millis()
                .saturating_sub(self.lifetime_millis()),
        );
        doc! {
            "session_id": { "$exists": true },
            "last_activity": { "$lt": cutoff },
        }
    }

    /// Delete, one at a time, every session `filter` matches, and name
    /// them. Each delete is atomic, and the loop runs until nothing
    /// matches, so a session signed in while it runs goes too.
    async fn destroy_matching(
        &self,
        filter: Document,
    ) -> Result<DestroyedSessions, FrameworkError> {
        let collection = self.ready().await?;
        let mut destroyed = DestroyedSessions::default();
        while let Some(session) = collection
            .find_one_and_delete(filter.clone())
            .projection(doc! { "session_id": 1 })
            .await
            .map_err(|error| store_error(STORE, "destroy", error))?
        {
            destroyed.count += 1;
            if let Ok(id) = session.get_str("session_id") {
                destroyed.ids.push(id.to_owned());
            }
        }
        Ok(destroyed)
    }
}

/// The filter and update that write `session` at `now`.
fn write_parts(
    session: &SessionData,
    now: DateTime<Utc>,
) -> Result<(Document, Document), FrameworkError> {
    let payload = encode_payload(
        &session.data,
        &session.csrf_token,
        session.user_id.as_deref(),
    )?;
    let mut names = session.auth_guard_names();
    names.sort();
    let guards: Vec<Bson> = names
        .iter()
        .filter_map(|guard| {
            session
                .auth_guard_id(guard)
                .map(|user_id| Bson::Document(doc! { "guard": guard.as_str(), "user_id": user_id }))
        })
        .collect();
    let update = doc! {
        "$set": {
            "session_id": session.id.as_str(),
            "user_id": session.user_id.as_deref(),
            "guards": guards,
            "payload": payload,
            "last_activity": bson_time(now),
        },
    };
    Ok((doc! { "session_id": session.id.as_str() }, update))
}

/// The sessions in which `guard` is signed in as `user_id`: its own entry,
/// or `user_id` for the default guard, as
/// [`SessionData::is_signed_in_as`] reads it.
fn signed_in_as(guard: &str, user_id: &str) -> Document {
    let own_entry = doc! { "guards": { "$elemMatch": { "guard": guard, "user_id": user_id } } };
    if guard == crate::auth::Auth::default_guard_name() {
        doc! { "$or": [ { "user_id": user_id }, own_entry ] }
    } else {
        own_entry
    }
}

#[async_trait]
impl SessionStore for MongoSessionDriver {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        let collection = self.ready().await?;
        let Some(stored) = collection
            .find_one(doc! { "session_id": id })
            .await
            .map_err(|error| store_error(STORE, "read", error))?
        else {
            return Ok(None);
        };

        let last_activity = stored
            .get_datetime("last_activity")
            .map(|at| at.timestamp_millis())
            .unwrap_or(i64::MIN);
        let now = crate::clock::now().timestamp_millis();
        if now > last_activity.saturating_add(self.lifetime_millis()) {
            // Expired. The read already answers "no session"; a delete that
            // fails leaves it for `gc`, so it is logged, without the id,
            // which is a bearer credential.
            if let Err(error) = self.destroy(id).await {
                tracing::warn!(
                    error = %error,
                    "expired session could not be deleted; garbage collection will remove it"
                );
            }
            return Ok(None);
        }

        let Payload {
            data,
            csrf_token,
            user_id,
        } = decode_payload(stored.get_str("payload").unwrap_or_default()).unwrap_or_else(|| {
            tracing::warn!("stored session payload failed to parse; treating the session as empty");
            Payload {
                data: Default::default(),
                csrf_token: None,
                user_id: None,
            }
        });
        Ok(Some(SessionData {
            id: id.to_owned(),
            data,
            user_id: stored
                .get_str("user_id")
                .ok()
                .map(str::to_owned)
                .or(user_id),
            csrf_token: csrf_token.unwrap_or_else(crate::session::middleware::generate_csrf_token),
            dirty: false,
            loaded_from_store: true,
        }))
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let (filter, update) = write_parts(session, crate::clock::now())?;
        let collection = self.ready().await?;
        // SEC-02(c): a session read from the store is updated only. Its
        // document gone means a revocation or `gc` removed it, and the write
        // must not bring it back.
        let create = !session.loaded_from_store;
        for round in 0..2 {
            match collection
                .update_one(filter.clone(), update.clone())
                .upsert(create)
                .await
            {
                Ok(result) => {
                    if !create && result.matched_count == 0 {
                        tracing::debug!(
                            authenticated = session.user_id.is_some(),
                            "session write skipped: the session no longer exists (revoked or \
                             expired concurrently)"
                        );
                    }
                    return Ok(());
                }
                // Two first writes of one new session at once: the loser's
                // insert collides on the unique index, and the next round
                // updates the winner's.
                Err(error) if create && round == 0 && is_duplicate_key(&error) => {}
                Err(error) => return Err(store_error(STORE, "write", error)),
            }
        }
        Ok(())
    }

    /// One update renames the session: the old id is gone and the new one
    /// holds `session` in the same atomic step, or nothing changed.
    async fn migrate_two_factor_session(
        &self,
        old_id: &str,
        session: &SessionData,
    ) -> Result<(), SessionMigrationError> {
        let (_, update) =
            write_parts(session, crate::clock::now()).map_err(SessionMigrationError::RolledBack)?;
        let collection = self
            .ready()
            .await
            .map_err(SessionMigrationError::RolledBack)?;
        match collection
            .update_one(doc! { "session_id": old_id }, update)
            .await
        {
            Ok(result) if result.matched_count == 1 => Ok(()),
            Ok(_) => Err(SessionMigrationError::RolledBack(FrameworkError::internal(
                "atomic 2FA session migration requires an existing old session",
            ))),
            // The server refused the update (the new id exists, say), or it
            // never reached a server: nothing changed.
            Err(error) if surely_not_applied(&error) => Err(SessionMigrationError::RolledBack(
                store_error(STORE, "two-factor migration", error),
            )),
            Err(error) => Err(SessionMigrationError::OutcomeUnknown(store_error(
                STORE,
                "two-factor migration",
                error,
            ))),
        }
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.ready()
            .await?
            .delete_one(doc! { "session_id": id })
            .await
            .map_err(|error| store_error(STORE, "destroy", error))?;
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let guard = crate::auth::Auth::default_guard_name();
        Ok(self.destroy_guard_sessions(&guard, user_id).await?.count)
    }

    async fn destroy_guard_sessions(
        &self,
        guard: &str,
        user_id: &str,
    ) -> Result<DestroyedSessions, FrameworkError> {
        self.destroy_matching(signed_in_as(guard, user_id)).await
    }

    async fn destroy_other_guard_sessions(
        &self,
        guard: &str,
        user_id: &str,
        current_id: &str,
    ) -> Result<DestroyedSessions, FrameworkError> {
        let mut filter = signed_in_as(guard, user_id);
        filter.insert("session_id", doc! { "$ne": current_id });
        self.destroy_matching(filter).await
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        let filter = self.gc_filter(crate::clock::now());
        Ok(self
            .ready()
            .await?
            .delete_many(filter)
            .await
            .map_err(|error| store_error(STORE, "gc", error))?
            .deleted_count)
    }
}
