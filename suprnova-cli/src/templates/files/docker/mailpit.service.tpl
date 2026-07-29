
  # Mailpit - Email Testing
  mailpit:
    image: axllent/mailpit:latest
    container_name: {project_name}_mailpit
    restart: unless-stopped
    # Loopback only, and this one is not negotiable by accident:
    # MP_SMTP_AUTH_ACCEPT_ANY makes Mailpit accept any credentials, so a
    # port published on 0.0.0.0 is an open SMTP relay, and the UI on 8025
    # serves every captured message — including password-reset links —
    # to anyone who can reach it.
    ports:
      - "${MAILPIT_HOST_BIND:-127.0.0.1}:${MAILPIT_SMTP_PORT:-1025}:1025"  # SMTP
      - "${MAILPIT_HOST_BIND:-127.0.0.1}:${MAILPIT_UI_PORT:-8025}:8025"    # Web UI
    environment:
      MP_MAX_MESSAGES: 5000
      MP_SMTP_AUTH_ACCEPT_ANY: 1
      MP_SMTP_AUTH_ALLOW_INSECURE: 1
