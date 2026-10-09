[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"
rust-version = "1.94.0"
description = "{description}"
{authors_line}
# Two binaries are declared below, so `cargo run` has to be told which
# one it means. Without `default-run` it refuses outright - it does NOT
# fall back to the binary sharing the package name - and every wrapper
# (`suprnova migrate`, `schedule:work`, `web:run`) fails before doing
# any work.
default-run = "{package_name}"

# The schema defaults match Laravel's MySQL schema: unsigned IDs and nullable
# timestamps. These commented settings choose model casts or signed IDs.
# Neither converts a column. Uncomment a setting when your columns need it.
#
# datetime_cast picks the cast of every `DateTime<Utc>` model field that
# names no cast of its own, `created_at`, `updated_at` and `deleted_at`
# included. Without it the cast is `AsDateTime`, which needs a text column,
# the kind the schema builder's `timestamps()` creates. Each value needs:
#   "native"  `AsNativeDateTime`: a column with a time zone. `TIMESTAMP` (what
#             Laravel's `timestamps()` creates on MySQL) or `DATETIME` on
#             MySQL, `timestamp with time zone` on Postgres, text on SQLite.
#   "naive"   `AsNaiveDateTime`: a column without one. `DATETIME` on MySQL,
#             `timestamp` (what Laravel creates on Postgres) on Postgres,
#             text on SQLite.
# A field whose column differs names its own cast, which wins:
#   #[model(casts = {{ published_at = suprnova::AsDateTime }})]
#
# [package.metadata.suprnova.model]
# datetime_cast = "native"
#
# `id()` and `foreign_id()` create `BIGINT UNSIGNED` on MySQL by default,
# as Laravel's `id()` and `foreignId()` do. Postgres and SQLite keep signed
# `BIGINT`. Models read unsigned MySQL keys into `u64` fields.
# This scaffold's `users.id` is already unsigned on MySQL.
# Set unsigned_ids = false to keep signed IDs in every migration this binary
# runs, including migrations from libraries. `#[suprnova::main]` installs it.
#
# [package.metadata.suprnova.schema]
# unsigned_ids = false

[[bin]]
name = "{package_name}"
path = "cmd/main.rs"

# Per-project console binary - runtime command dispatch (db:seed,
# user-defined `#[command]` async fns, etc.). Same crate, different
# `fn main` because console commands exit when their handler returns
# whereas the server binary loops forever.
[[bin]]
name = "console"
path = "src/bin/console.rs"

[dependencies]
# Production build shape: default features off, the ten non-`testing`
# defaults listed explicitly, so the binaries above never carry a test
# seam. See the manual, "Production build shape" (deployment.md).
suprnova = {{ git = "https://github.com/eas4ai/suprnova.git", tag = "{framework_tag}", default-features = false, features = ["filesystem", "database-sqlite", "database-postgres", "database-mysql", "vector-mariadb", "web-push", "localization", "magnetar-oauth", "media", "queue-sqs"] }}
tokio = {{ version = "1", features = ["full"] }}
sea-orm-migration = {{ version = "2.0", features = ["sqlx-sqlite", "sqlx-postgres", "runtime-tokio-native-tls"] }}
sea-orm = {{ version = "2.0", features = ["sqlx-sqlite", "sqlx-postgres", "runtime-tokio-native-tls", "macros", "with-chrono", "postgres-use-serial-pk"] }}
serde = {{ version = "1.0", features = ["derive"] }}
async-trait = "0.1"
clap = {{ version = "4", features = ["derive"] }}
chrono = {{ version = "0.4", features = ["serde"] }}
validator = {{ version = "0.20", features = ["derive"] }}
tracing = "0.1"

[features]
# Heap profiling with dhat, for measuring this application's memory: build
# with `cargo build --profile profiling --features heap-profiling`, and a
# command that finishes or a server that shuts down gracefully writes
# `dhat-heap.json`. See the manual, "Heap profiling" (deployment.md).
heap-profiling = ["suprnova/heap-profiling"]

# The release profile with debug symbols, for samply, perf, or a heap
# profile with readable stacks.
[profile.profiling]
inherits = "release"
debug = true

[dev-dependencies]
# Turns `testing` back on for `cargo test` and every `--tests` build
# only; it never reaches the binaries above. See the manual,
# "Production build shape" (deployment.md).
suprnova = {{ git = "https://github.com/eas4ai/suprnova.git", tag = "{framework_tag}", features = ["testing"] }}
