
  # RustFS - S3-compatible Object Storage
  rustfs:
    image: rustfs/rustfs:latest
    container_name: {project_name}_rustfs
    restart: unless-stopped
    # Loopback only - see the note on the postgres service.
    ports:
      - "${RUSTFS_HOST_BIND:-127.0.0.1}:${RUSTFS_API_PORT:-9000}:9000"     # S3 API
      - "${RUSTFS_HOST_BIND:-127.0.0.1}:${RUSTFS_CONSOLE_PORT:-9001}:9001"  # Console UI
    environment:
      # Generated per project rather than RustFS's stock default pair,
      # which is the first one any scanner tries.
      RUSTFS_ACCESS_KEY: ${RUSTFS_ACCESS_KEY:-suprnova}
      RUSTFS_SECRET_KEY: ${RUSTFS_SECRET_KEY:-{rustfs_password}}
    # The image runs as uid 10001 and ships /data owned by it; a named
    # volume mounted there inherits that ownership on first use.
    volumes:
      - rustfs_data:/data
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:9000/health"]
      interval: 30s
      timeout: 20s
      retries: 3
