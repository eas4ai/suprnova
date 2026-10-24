
  # Error tracker, API and dashboard in one container.
  rustrak:
    image: rustrak/rustrak-server:v0.16.4
    container_name: {project_name}_rustrak
    restart: unless-stopped
    environment:
      # Keep this generated key across restarts to preserve dashboard sessions.
      SESSION_SECRET_KEY: "{rustrak_session_secret}"
      CREATE_SUPERUSER: "${RUSTRAK_CREATE_SUPERUSER:?set email:password for the first admin}"
      PUBLIC_URL: http://127.0.0.1:${RUSTRAK_PORT:-8080}
      RUSTRAK_TELEMETRY: "off"
    ports:
      - "127.0.0.1:${RUSTRAK_PORT:-8080}:8080"
    volumes:
      - rustrak_data:/data
