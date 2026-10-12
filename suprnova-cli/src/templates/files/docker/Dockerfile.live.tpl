# ==========================================
# Stage 1: Build the Rust application
# ==========================================
#
# A Live kit project (`suprnova new --frontend live`) has no `frontend/`
# and no Node toolchain: its pages are server-rendered Live documents, and
# the application itself serves the Live browser runtime and the library
# components' stylesheets and scripts. So there is no Node stage, and no
# `public/assets` build to carry over.
FROM rust:1.94.0-slim-bookworm AS backend-builder

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
# image (the scaffold's .gitignore does not exclude it).
COPY Cargo.toml Cargo.lock* ./

# Build dependencies only, for layer caching. EVERY binary the manifest
# declares needs a stub here - cargo resolves all targets, so a missing
# `src/bin/console.rs` fails this stage outright rather than merely
# missing the cache.
RUN mkdir -p cmd src/bin \
    && echo "fn main() {}" > cmd/main.rs \
    && echo "fn main() {}" > src/bin/console.rs
RUN cargo build --release \
    && rm -rf src cmd

# Copy actual source code
COPY cmd/ ./cmd/
COPY src/ ./src/

# The Askama templates: the documents, layouts and components under
# `templates/live/`, and the library components under
# `templates/suprnova-ui/`. The Rust build checks every view against them
# at compile time.
COPY templates/ ./templates/

# Schema dumps (`suprnova schema:dump`), when the project has any, for the
# runtime stage. `databas[e]` matches nothing in a project without a
# database directory, and Cargo.toml gives the COPY a source either way;
# .dockerignore keeps everything in database/ but schema/ out.
COPY Cargo.toml databas[e] ./database-context/
RUN mkdir -p database/schema \
    && if [ -d database-context/schema ]; then cp -R database-context/schema/. database/schema/; fi \
    && rm -rf database-context

# Build the application (single unified binary)
RUN rm ./target/release/deps/{package_name}* 2>/dev/null || true && cargo build --release

# ==========================================
# Stage 2: Runtime Image
# ==========================================
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# The client the auto-migration loads a schema dump with, into a database
# that has run no migration: postgresql-client for Postgres (the database
# docker-compose.yml runs), mariadb-client for MySQL or MariaDB, or empty
# for SQLite. Change it with `--build-arg DB_CLIENT=...`.
ARG DB_CLIENT=postgresql-client

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    ${DB_CLIENT} \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 appuser

# Copy the compiled binary
COPY --from=backend-builder /app/{package_name}/target/release/{package_name} ./app

# Schema dumps, loaded into an empty database before the migrations newer
# than them run
COPY --from=backend-builder /app/{package_name}/database/schema ./database/schema

# The library components' stylesheets and scripts. `try_live_ui_assets()`
# reads them from `templates/suprnova-ui/` under the working directory on
# each request, and the application refuses to start without them.
COPY templates/suprnova-ui/ ./templates/suprnova-ui/

# The kit's stylesheet and any other static file, served from `public/`,
# and the localization catalogs `Localization::bootstrap` reads at boot.
COPY public/ ./public/
COPY lang/ ./lang/

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
