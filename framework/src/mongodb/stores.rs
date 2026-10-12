//! What the MongoDB queue, cache and session stores share: the check of a
//! collection name, the conversion of times, the reading of a driver error,
//! and the indexes each store creates on its first use.

use std::time::Duration;

use ::bson::{Bson, Document};
use ::mongodb::error::{Error, ErrorKind, WriteFailure};
use ::mongodb::options::IndexOptions;
use ::mongodb::{Collection, IndexModel};
use chrono::{DateTime, Utc};

use crate::error::FrameworkError;

/// The server's error code for a write that would break a unique index.
const DUPLICATE_KEY: i32 = 11000;

/// The server's error code for an operator applied to a value of the wrong
/// type, such as `$inc` on a string.
pub(crate) const TYPE_MISMATCH: i32 = 14;

/// Refuse a collection name MongoDB would refuse, before any server sees
/// it, naming the name and what it is for.
pub(crate) fn validate_collection_name(name: &str, what: &str) -> Result<(), FrameworkError> {
    let reason = if name.is_empty() {
        Some("it is empty")
    } else if name.contains('$') {
        Some("it holds '$'")
    } else if name.contains('\0') {
        Some("it holds a NUL character")
    } else if name.starts_with("system.") {
        Some("the 'system.' prefix is the server's own")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(FrameworkError::internal(format!(
            "{name:?} is not a MongoDB collection name for {what}: {reason}"
        ))),
        None => Ok(()),
    }
}

/// `at` as a BSON datetime, at millisecond precision.
pub(crate) fn bson_time(at: DateTime<Utc>) -> ::bson::DateTime {
    ::bson::DateTime::from_millis(at.timestamp_millis())
}

/// A BSON datetime as a chrono one. A date chrono cannot hold reads as the
/// nearest one it can.
pub(crate) fn chrono_time(at: ::bson::DateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(at.timestamp_millis()).unwrap_or(
        if at.timestamp_millis() < 0 {
            DateTime::<Utc>::MIN_UTC
        } else {
            DateTime::<Utc>::MAX_UTC
        },
    )
}

/// `at` moved forward by `by`, as a BSON datetime. A sum past the dates
/// BSON holds is the latest one, which never comes: a lifetime that long is
/// forever.
pub(crate) fn bson_time_after(at: DateTime<Utc>, by: Duration) -> ::bson::DateTime {
    let by = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
    ::bson::DateTime::from_millis(at.timestamp_millis().saturating_add(by))
}

/// Whether `error` is the server refusing a write that would break a
/// unique index, the way an insert of a key that exists fails.
pub(crate) fn is_duplicate_key(error: &Error) -> bool {
    server_code(error) == Some(DUPLICATE_KEY)
}

/// The code the server answered `error` with, when the server answered.
pub(crate) fn server_code(error: &Error) -> Option<i32> {
    match error.kind.as_ref() {
        ErrorKind::Command(command) => Some(command.code),
        ErrorKind::Write(WriteFailure::WriteError(write)) => Some(write.code),
        ErrorKind::InsertMany(insert) => insert
            .write_errors
            .as_ref()
            .and_then(|errors| errors.first())
            .map(|error| error.code),
        _ => None,
    }
}

/// Whether the server refused the operation behind `error` without
/// applying it, or it never reached a server. A write concern error, a
/// lost connection or a timeout leaves the outcome unknown.
pub(crate) fn surely_not_applied(error: &Error) -> bool {
    matches!(
        error.kind.as_ref(),
        ErrorKind::Command(_)
            | ErrorKind::Write(WriteFailure::WriteError(_))
            | ErrorKind::ServerSelection { .. }
            | ErrorKind::InvalidArgument { .. }
            | ErrorKind::BsonSerialization(_)
            | ErrorKind::Authentication { .. }
    )
}

/// A driver error as a [`FrameworkError`] saying which store and which
/// operation failed, with the driver's error as its source.
pub(crate) fn store_error(store: &str, operation: &str, error: Error) -> FrameworkError {
    FrameworkError::from_external_with(
        format!("{store} {operation}: MongoDB: {}", error.kind),
        error,
    )
}

/// One index a store needs: its keys, and the expiry for a TTL index.
pub(crate) struct StoreIndex {
    /// The indexed fields.
    pub(crate) keys: Document,
    /// `Some` for a TTL index: the server removes a document this long
    /// after the date in its indexed field.
    pub(crate) expire_after: Option<Duration>,
    /// Whether no two documents may share the indexed value.
    pub(crate) unique: bool,
}

impl StoreIndex {
    /// An ordinary index on `keys`.
    pub(crate) fn on(keys: Document) -> Self {
        Self {
            keys,
            expire_after: None,
            unique: false,
        }
    }

    /// A TTL index on the date `field`, removing a document once its date
    /// has passed.
    pub(crate) fn expiring(field: &str) -> Self {
        let mut keys = Document::new();
        keys.insert(field, 1_i32);
        Self {
            keys,
            expire_after: Some(Duration::ZERO),
            unique: false,
        }
    }

    /// A unique index on `keys`.
    pub(crate) fn unique(keys: Document) -> Self {
        Self {
            keys,
            expire_after: None,
            unique: true,
        }
    }

    /// The index as the server lists it: `key`, and `expireAfterSeconds`
    /// or `unique` when they are set.
    pub(crate) fn rendered(&self) -> Document {
        let mut rendered = Document::new();
        rendered.insert("key", self.keys.clone());
        if let Some(expire_after) = self.expire_after {
            rendered.insert(
                "expireAfterSeconds",
                Bson::Int64(i64::try_from(expire_after.as_secs()).unwrap_or(i64::MAX)),
            );
        }
        if self.unique {
            rendered.insert("unique", true);
        }
        rendered
    }

    fn model(&self) -> IndexModel {
        let mut options = IndexOptions::default();
        options.expire_after = self.expire_after;
        if self.unique {
            options.unique = Some(true);
        }
        IndexModel::builder()
            .keys(self.keys.clone())
            .options(options)
            .build()
    }
}

/// Create `indexes` on `collection`. Creating an index that exists with the
/// same options does nothing, so a store runs this once per process; an
/// index of the same name with other options is the server's error.
pub(crate) async fn create_indexes(
    store: &str,
    collection: &Collection<Document>,
    indexes: Vec<StoreIndex>,
) -> Result<(), FrameworkError> {
    collection
        .create_indexes(indexes.iter().map(StoreIndex::model))
        .await
        .map(|_| ())
        .map_err(|error| store_error(store, "index creation", error))
}
