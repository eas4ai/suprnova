//! Named queue connections.
//!
//! A connection is a name bound to a driver. The driver installed with
//! [`Queue::set_driver`](crate::queue::Queue::set_driver) is the *default*
//! connection, named by
//! [`Queue::connection_name`](crate::queue::Queue::connection_name). Every
//! other connection is registered here with
//! [`Queue::register_connection`](crate::queue::Queue::register_connection),
//! so one process can keep, for example, a Redis queue for fast work and a
//! database queue for durable work.
//!
//! # How a name selects a driver
//!
//! A push resolves a connection name first: a per-push override, then a
//! route, then the job's own [`Job::connection`](crate::queue::Job::connection),
//! then the default connection. [`target`] turns that name into a driver:
//!
//! 1. the default connection, when the name is the default's. The default's
//!    own name always means the driver `Queue::set_driver` installed, so a
//!    connection registered under that name never shadows it;
//! 2. a registered connection of that name;
//! 3. the default connection, when **no** connection is registered. An
//!    application with one driver may use connection names as labels on its
//!    lifecycle events, as it could before connections selected a driver,
//!    and nothing changes for it;
//! 4. otherwise an error. Once one connection is registered a name selects
//!    a driver, and a name that selects none is a mistake that must not
//!    quietly become a push to the default connection.
//!
//! # The label
//!
//! A [`Target`] carries the *label* of the connection it resolved to. The
//! label is what a worker on that connection is started with, so it is the
//! one value both halves of a connection-scoped setting agree on: a
//! connection-scoped forward and a queue's pause are keyed by it on the push
//! and on the claim alike. In case 3 the label is the default connection's
//! name, never the name the job declared, because that is what the one
//! worker there is labelled with.
//!
//! One driver has one label. A name registered for the very driver that is
//! also the default connection is a second name for the default, and it
//! resolves to the default's label. `QUEUE_CONNECTIONS` creates such a name
//! when it lists the driver `QUEUE_DRIVER` selects. Were the two names two
//! labels, a forward or a pause set for one would reach only the pushes and
//! the workers that used that name, and the rest of the same queue would be
//! stranded or would keep running.

use crate::error::FrameworkError;
use crate::lock;
use crate::queue::{Queue, QueueDriver};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

static CONNECTIONS: RwLock<BTreeMap<String, Arc<dyn QueueDriver>>> = RwLock::new(BTreeMap::new());

const LOCK_CONTEXT: &str = "queue connection registry";

/// Where a push that resolved to one connection name goes.
pub(crate) struct Target {
    /// The driver that receives the envelope.
    pub(crate) driver: Arc<dyn QueueDriver>,
    /// The label of the connection, see the module docs.
    pub(crate) label: String,
}

/// Register (or replace) the connection `name`.
pub(crate) fn try_register(name: &str, driver: Arc<dyn QueueDriver>) -> Result<(), FrameworkError> {
    if name.trim().is_empty() {
        return Err(FrameworkError::internal(
            "a queue connection needs a name; the unnamed connection is the default, \
             installed with Queue::set_driver",
        ));
    }
    lock::write(&CONNECTIONS, LOCK_CONTEXT)?.insert(name.to_owned(), driver);
    Ok(())
}

/// The names of the registered connections, in order. The default
/// connection is listed only when it was also registered by name.
pub(crate) fn names() -> Result<Vec<String>, FrameworkError> {
    Ok(lock::read(&CONNECTIONS, LOCK_CONTEXT)?
        .keys()
        .cloned()
        .collect())
}

/// Resolve a connection name to its driver and label. The module docs give
/// the order.
pub(crate) fn target(connection: &str) -> Result<Target, FrameworkError> {
    let default_name = Queue::connection_name();
    if connection == default_name {
        return Ok(Target {
            driver: crate::queue::current_driver()?,
            label: default_name,
        });
    }
    let (registered, registered_names) = {
        let connections = lock::read(&CONNECTIONS, LOCK_CONTEXT)?;
        (
            connections.get(connection).cloned(),
            connections.keys().cloned().collect::<Vec<_>>(),
        )
    };
    match registered {
        Some(driver) => {
            let label = if is_the_default(&driver) {
                default_name
            } else {
                connection.to_owned()
            };
            Ok(Target { driver, label })
        }
        None if registered_names.is_empty() => Ok(Target {
            driver: crate::queue::current_driver()?,
            label: default_name,
        }),
        None => Err(FrameworkError::internal(format!(
            "queue connection `{connection}` is not registered. Registered connections: \
             {}; the default connection is `{default_name}`. Register it with \
             Queue::register_connection, or name one of these",
            registered_names.join(", ")
        ))),
    }
}

/// Whether `driver` is the driver of the default connection, registered a
/// second time under a name of its own.
fn is_the_default(driver: &Arc<dyn QueueDriver>) -> bool {
    crate::queue::current_driver().is_ok_and(|default| Arc::ptr_eq(&default, driver))
}

/// The label a push to `connection` is gated on, for the code that builds an
/// envelope before the driver is resolved. A name [`target`] would refuse
/// gets the default connection's label here: the push fails when it reaches
/// `target`, and until then no value is more right.
pub(crate) fn label_for(connection: &str) -> String {
    target(connection)
        .map(|target| target.label)
        .unwrap_or_else(|_| Queue::connection_name())
}

/// The label that a setting scoped to `connection` applies to: a
/// connection-scoped forward, a queue's pause.
///
/// A name that is the default's, or registered, is its label. Any other
/// name is returned as it is, so a setting scoped to a name that is no
/// connection matches nothing. That holds while no connection is registered
/// too: a forward scoped to another name than the default's stays inert
/// there, as it was before a name selected a driver.
pub(crate) fn scoped_label(connection: &str) -> String {
    let default_name = Queue::connection_name();
    if connection == default_name {
        return default_name;
    }
    let registered = lock::read(&CONNECTIONS, LOCK_CONTEXT)
        .ok()
        .and_then(|connections| connections.get(connection).cloned());
    match registered {
        Some(driver) if is_the_default(&driver) => default_name,
        _ => connection.to_owned(),
    }
}

/// Remove every registered connection. Test support, reached through
/// `queue::testing::forget_connections`: the registry is process-wide, so a
/// test that registers a connection removes it again.
pub(crate) fn clear() {
    CONNECTIONS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}
