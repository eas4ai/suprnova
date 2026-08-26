# suprnova-RS

A Laravel-inspired web framework for Rust.

Current `main` requires Rust 1.94.0 or newer. The tagged v1.3.6 release has
the same Rust 1.94.0 floor. Suprnova is distributed from Git rather than crates.io.

## Installation

Add suprnova to your `Cargo.toml`:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.6" }
tokio = { version = "1", features = ["full"] }
```

## Cargo Features

Suprnova keeps its historical framework surface in the default feature set.
Applications that disable defaults must select the subsystems and database
drivers they use.

| Feature | Default | Enables |
|---|---:|---|
| `testing` | Yes | Framework test hooks. It does not select a database driver or filesystem backend. |
| `filesystem` | Yes | The OpenDAL-backed `Storage` facade, storage backends, and upload persistence helpers. |
| `database-sqlite` | Yes | SQLite support across SeaORM, migrations, and Magnetar storage. |
| `database-postgres` | Yes | Postgres support across SeaORM, migrations, and Magnetar storage. |
| `database-mysql` | Yes | MySQL support across SeaORM, migrations, and Magnetar storage. |
| `vector-mariadb` | Yes | The direct-SQLx MariaDB vector driver; also enables `database-mysql`. |
| `web-push` | Yes | VAPID/web-push support and the web-push notification channel. |
| `broadcasting-fanout` | No | SeaStreamer-backed cross-process broadcasting fanout. |
| `otel` | No | OpenTelemetry tracing and export integration. |
| `vector-pinecone` | No | The Pinecone vector driver (REST; no extra dependencies). |

For example, a service using only SQLite, Postgres, and broadcasting fanout can
exclude filesystem, MySQL, MariaDB vector, and web-push dependencies:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.6", default-features = false, features = [
    "database-sqlite",
    "database-postgres",
    "broadcasting-fanout",
] }
```

Filesystem layers must use Suprnova's OpenDAL re-export so their types share
the exact pinned source identity:

```rust
use suprnova::opendal::layers::{LoggingLayer, RetryLayer, TimeoutLayer};
```

## Quick Start

```rust
use suprnova::{json_response, text, Router, Server, Request, Response};

#[tokio::main]
async fn main() {
    let router = Router::new()
        .get("/", index)
        .get("/users/{id}", show_user);

    Server::new(router)
        .port(8080)
        .run()
        .await
        .expect("Failed to start server");
}

async fn index(_req: Request) -> Response {
    text("Welcome to suprnova!")
}

async fn show_user(req: Request) -> Response {
    let id = req.param("id")?;  // Returns 400 if missing
    json_response!({
        "id": id,
        "name": format!("User {}", id)
    })
}
```

## Features

- **Simple routing** - GET, POST, PUT, DELETE with route parameters
- **Async handlers** - Built on Tokio for high performance
- **Response builders** - Text, JSON, and custom responses
- **Error handling** - Use `?` operator for automatic 400 responses
- **Laravel-inspired** - Familiar patterns for Laravel developers

## CLI Tool

Use the suprnova CLI to scaffold new projects:

```bash
cargo install --git https://github.com/eas4ai/suprnova.git --tag v1.3.6 suprnova-cli
suprnova new myapp
```

## License

MIT
