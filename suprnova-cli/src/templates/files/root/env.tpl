APP_NAME="{project_name}"
APP_ENV=local
APP_DEBUG=true
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding) used to encrypt
# session cookies, pagination cursors, and anything that goes through
# `suprnova::Crypt`. Generated at scaffold time by `suprnova new`;
# rotate with `suprnova key:generate`. Required in production —
# Suprnova fails closed on boot when APP_ENV is not local/dev/test
# and APP_KEY is unset.
APP_KEY={app_key}

# Backend + Vite ports. Distinctive defaults to dodge the universally
# squatted 8080/5173. `suprnova serve` treats these as a base and scans
# upward if they're busy, so two Suprnova apps can run at once without a
# clash. Set SERVER_PORT explicitly (or rely on $PORT in production) to
# pin the backend port.
SERVER_HOST=127.0.0.1
SERVER_PORT=8765

VITE_PORT=5765

# Database (SQLite by default, change to postgres://user:pass@localhost:5432/dbname for PostgreSQL)
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Localization. `LocaleMiddleware` detects the per-request locale
# (session -> cookie -> Accept-Language) and falls back to APP_LOCALE;
# APP_FALLBACK_LOCALE is used when a message id is missing from the
# detected locale's catalog. Add a locale by creating lang/<locale>/
# with the same message ids as lang/en/.
APP_LOCALE=en
APP_FALLBACK_LOCALE=en

# Mail
#
# These are the names the framework's transport actually reads. The SMTP
# credentials are a pair: set BOTH MAIL_SMTP_USER and MAIL_SMTP_PASS for
# authenticated STARTTLS, or leave both unset for a local unauthenticated
# catcher (maildev / mailpit / mailhog, which listen on 1025). Setting
# exactly one is treated as a misconfiguration and warns at boot.
#
# MAIL_SMTP_ENCRYPTION is derived from the credentials when left unset —
# `starttls` with them, `none` without — so this file works against a
# local catcher as-is. Set it to `tls` for a relay expecting implicit TLS
# on 465. Production refuses to boot on an unencrypted connection; the
# escape hatch is MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true, which is
# only defensible for a relay on a private network.
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=localhost
MAIL_SMTP_PORT=1025
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
MAIL_SMTP_ENCRYPTION=

# Required. The auth flows (password reset, email verification) refuse to
# send without a real from-address.
MAIL_FROM=hello@example.com
MAIL_FROM_NAME="Suprnova App"
