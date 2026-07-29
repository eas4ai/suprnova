# ==========================================
# Stage 1: Build Rust API
# ==========================================
#
# An API project (`suprnova new --api`) has no `frontend/` and no `cmd/`.
# Its server binary is `src/main.rs`, and it serves JSON, not Inertia
# pages — so there is nothing to build with node, no page sources for
# `inertia_response!` to validate against, and no `public/assets` to
# carry into the runtime image.
#
# Through v0.7.2, `docker:init` emitted the full-stack Dockerfile for
# every project shape. On an API project its first instruction —
# `COPY frontend/package.json` — failed outright, so `suprnova new --api`
# followed by `docker:init` followed by `docker build` could not succeed.
FROM rust:1.91.1-slim-bookworm AS backend-builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create a new empty shell project for dependency caching
RUN cargo new --bin {package_name}
WORKDIR /app/{package_name}

# Copy manifests. `Cargo.lock*` is a glob so a project that has not
# generated one yet still builds; commit the lockfile for a reproducible
# image (the scaffold's .gitignore no longer excludes it).
COPY Cargo.toml Cargo.lock* ./

# Build dependencies only, for layer caching. EVERY binary the manifest
# declares needs a stub here — cargo resolves all targets, so a missing
# `src/bin/console.rs` fails this stage outright rather than merely
# missing the cache. The API scaffold's server bin is `src/main.rs`,
# which `cargo new --bin` already created.
RUN mkdir -p src/bin \
    && echo "fn main() {}" > src/main.rs \
    && echo "fn main() {}" > src/bin/console.rs
RUN cargo build --release \
    && rm -rf src

# Copy actual source code
COPY src/ ./src/

# Build the application (single unified binary)
RUN rm ./target/release/deps/{package_name}* 2>/dev/null || true && cargo build --release

# ==========================================
# Stage 2: Runtime Image
# ==========================================
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 appuser

# Copy the compiled binary
COPY --from=backend-builder /app/{package_name}/target/release/{package_name} ./app

# No `public/` copy: an API project serves no static assets.

# Set ownership
RUN chown -R appuser:appuser /app

USER appuser

# Environment variables. SERVER_PORT matches Suprnova's default; the app
# also honors $PORT (Heroku/Railway/Render/Fly inject it), which takes
# effect when SERVER_PORT is unset.
ENV APP_ENV=production
ENV SERVER_HOST=0.0.0.0
ENV SERVER_PORT=8765

EXPOSE 8765

# Default: Run web server with auto-migrations
# Override with different commands for other modes:
#   docker run myapp ./app serve --no-migrate  # Skip migrations
#   docker run myapp ./app migrate             # Run migrations only
#   docker run myapp ./app schedule:work       # Run scheduler daemon
CMD ["./app"]
