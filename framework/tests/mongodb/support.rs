//! What the MongoDB tests share: a child process with a chosen MongoDB
//! environment, a URI no server answers, and the test server's URL.

/// Every variable the MongoDB configuration reads, cleared in each child so
/// the developer's own environment cannot reach a test.
pub const MONGODB_VARIABLES: [&str; 2] = ["MONGODB_URI", "MONGODB_DATABASE"];

/// A URI no server answers. Nothing listens on port 1, and the short
/// timeouts make the first call that needs the server fail within a second
/// instead of the driver's thirty.
pub const UNREACHABLE_URI: &str =
    "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=300&connectTimeoutMS=300";

/// A password the tests put in URIs, to assert no error or debug output
/// shows it.
pub const PASSWORD: &str = "hunter2-secret";

/// Run the test `child` (its path in this binary) alone in a child process
/// whose MongoDB environment is exactly `variables`, and fail unless it ran
/// and passed.
pub fn run_alone_with(child: &str, variables: &[(&str, &str)]) {
    let child = {
        // The child inherits the rest of the environment; the lock waits
        // out any test between setting a variable and restoring it.
        let _env = crate::env_lock::lock_env();
        let mut command = crate::own_process::child_command(child);
        for name in MONGODB_VARIABLES {
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

/// The URL of the throwaway database the `mongodb_` tests may write to.
pub fn test_url() -> String {
    std::env::var("MONGODB_TEST_URL").expect(
        "set MONGODB_TEST_URL to a mongodb:// URL naming a throwaway database \
         to run the mongodb_ tests",
    )
}

/// The database `url` names in its path.
pub fn database_of(url: &str) -> String {
    suprnova::mongodb::driver::options::ConnectionString::parse(url)
        .expect("MONGODB_TEST_URL is a MongoDB connection string")
        .default_database
        .expect("MONGODB_TEST_URL names a database in its path")
}
