
  # MinIO - S3-compatible Object Storage
  minio:
    image: minio/minio:latest
    container_name: {project_name}_minio
    restart: unless-stopped
    command: server /data --console-address ":9001"
    # Loopback only — see the note on the postgres service.
    ports:
      - "${MINIO_HOST_BIND:-127.0.0.1}:${MINIO_API_PORT:-9000}:9000"     # S3 API
      - "${MINIO_HOST_BIND:-127.0.0.1}:${MINIO_CONSOLE_PORT:-9001}:9001"  # Console UI
    environment:
      # Generated per project rather than MinIO's stock root credentials,
      # which are the first pair any scanner tries.
      MINIO_ROOT_USER: ${MINIO_ROOT_USER:-suprnova}
      MINIO_ROOT_PASSWORD: ${MINIO_ROOT_PASSWORD:-{minio_password}}
    volumes:
      - minio_data:/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9000/minio/health/live"]
      interval: 30s
      timeout: 20s
      retries: 3
