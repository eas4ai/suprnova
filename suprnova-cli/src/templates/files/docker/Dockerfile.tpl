# ==========================================
# Stage 1: Build Frontend
# ==========================================
FROM node:20-alpine AS frontend-builder

WORKDIR /app/frontend

# Install dependencies.
#
# The lockfile is optional in the COPY (the glob matches nothing on a
# fresh scaffold) so `npm ci` cannot be used unconditionally — it fails
# outright without one. Prefer it when a lock is present, because that is
# the reproducible path, and fall back to `npm install` when it is not.
# Commit frontend/package-lock.json to get the reproducible build.
COPY frontend/package.json frontend/package-lock.json* ./
RUN if [ -f package-lock.json ]; then npm ci; else npm install; fi

# Copy frontend source and build
COPY frontend/ ./
RUN npm run build

# ==========================================
# Stage 2: Build Rust Backend
# ==========================================
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
# missing the cache.
RUN mkdir -p cmd src/bin \
    && echo "fn main() {}" > cmd/main.rs \
    && echo "fn main() {}" > src/bin/console.rs
RUN cargo build --release \
    && rm -rf src cmd

# Copy actual source code
COPY cmd/ ./cmd/
COPY src/ ./src/

# Copy frontend build output to public directory
COPY --from=frontend-builder /app/frontend/dist ./public/assets

# Build the application (single unified binary)
RUN rm ./target/release/deps/{package_name}* 2>/dev/null || true && cargo build --release

# ==========================================
# Stage 3: Runtime Image
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

# Copy public assets
COPY --from=backend-builder /app/{package_name}/public ./public

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
