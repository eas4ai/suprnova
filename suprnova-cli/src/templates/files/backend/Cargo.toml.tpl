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
# (`suprnova migrate`, `schedule:work`, `web:run`, …) fails before doing
# any work.
default-run = "{package_name}"

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
suprnova = {{ git = "https://github.com/eas4ai/suprnova.git", tag = "{framework_tag}" }}
tokio = {{ version = "1", features = ["full"] }}
sea-orm-migration = {{ version = "2.0", features = ["sqlx-sqlite", "sqlx-postgres", "runtime-tokio-native-tls"] }}
sea-orm = {{ version = "2.0", features = ["sqlx-sqlite", "sqlx-postgres", "runtime-tokio-native-tls", "macros", "with-chrono", "postgres-use-serial-pk"] }}
serde = {{ version = "1.0", features = ["derive"] }}
async-trait = "0.1"
clap = {{ version = "4", features = ["derive"] }}
chrono = {{ version = "0.4", features = ["serde"] }}
validator = {{ version = "0.20", features = ["derive"] }}
tracing = "0.1"
