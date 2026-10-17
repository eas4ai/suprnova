//! What the queue, cache and session tests share (PAR-186 to PAR-188): a
//! connection no server answers, a connection to the test server, and
//! collection names no other test uses.
//!
//! The stores take their connection as a value, so these tests build it
//! themselves rather than through the `Mongo` facade, and need no child
//! process unless they read the environment.

use suprnova::{MongoConnection, MongoConnectionConfig};

use crate::support::{UNREACHABLE_URI, database_of, test_url};

/// A connection to a server that does not answer. Opening it contacts
/// nothing, so a store built on it renders its documents without a server
/// and fails the first operation that needs one within a second.
pub async fn unreachable() -> MongoConnection {
    MongoConnection::connect(
        "unreachable",
        &MongoConnectionConfig::new(UNREACHABLE_URI, "suprnova_stores"),
    )
    .await
    .expect("opening a connection contacts no server")
}

/// A connection to the throwaway database `MONGODB_TEST_URL` names.
pub async fn server() -> MongoConnection {
    let url = test_url();
    let database = database_of(&url);
    MongoConnection::connect("test", &MongoConnectionConfig::new(url, database))
        .await
        .expect("MONGODB_TEST_URL opens")
}

/// A collection name no other test uses, so tests that run at once never
/// share documents.
pub fn unique(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// Drop the collections a test created on the server.
pub async fn drop_collections(connection: &MongoConnection, names: &[&str]) {
    for name in names {
        connection
            .collection::<suprnova::bson::Document>(name)
            .drop()
            .await
            .expect("drop a test collection");
    }
}

/// Every variable the queue, cache and session selection reads, cleared in
/// each child the selection tests start so the developer's own environment
/// cannot choose a driver for them.
const DRIVER_VARIABLES: [&str; 6] = [
    "QUEUE_DRIVER",
    "QUEUE_CONNECTIONS",
    "CACHE_DRIVER",
    "CACHE_DEFAULT_TTL",
    "CACHE_PREFIX",
    "SESSION_DRIVER",
];

/// Run the test `child` alone in a child process whose MongoDB and driver
/// environment is exactly `variables`, and fail unless it ran and passed.
pub fn run_alone_with_drivers(child: &str, variables: &[(&str, &str)]) {
    let child = {
        let _env = crate::env_lock::lock_env();
        let mut command = crate::own_process::child_command(child);
        for name in crate::support::MONGODB_VARIABLES
            .iter()
            .chain(DRIVER_VARIABLES.iter())
        {
            command.env_remove(name);
        }
        for (name, value) in variables {
            command.env(name, value);
        }
        command.spawn().expect("spawn the child process")
    };
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
}
