pub mod admin;
pub mod auth_2fa;
pub mod auth_reset;
pub mod auth_verify;
pub mod avatar_upload;
/// Benchmark-only routes. Absent from a binary built without
/// `--features bench`, which is the point — see the module docs.
#[cfg(feature = "bench")]
pub mod bench;
pub mod config_example;
pub mod home;
pub mod paginated_users;
pub mod ping;
pub mod posts;
pub mod sse_example;
pub mod todo;
pub mod user;
pub mod welcome;
