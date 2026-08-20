APP_NAME="Suprnova Application"
APP_ENV=local
APP_DEBUG=true
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Generate one with
# `suprnova key:generate` and copy it into your `.env`. Required in
# production — Suprnova fails closed on boot when APP_ENV is not
# local/dev/test and APP_KEY is unset.
APP_KEY=

# Backend + Vite ports. Distinctive defaults to dodge the universally
# squatted 8080/5173; `suprnova serve` scans upward if they're busy.
SERVER_HOST=127.0.0.1
SERVER_PORT=8765

VITE_PORT=5765

# SUPRNOVA_FRONTEND=svelte   # svelte | react | vue — which frontend the HTML shell renders; the scaffolded .env sets it for you

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
# Cookie-name prefix for the session and remember-me cookies.
# "__Host-" is the production hardening: the browser refuses the cookie
# unless it is Secure, Path=/, and Domain-less, so a compromised sibling
# subdomain cannot shadow it. It requires HTTPS (SESSION_SECURE=true)
# and is incompatible with SESSION_DOMAIN. Leave empty for local HTTP
# development - with SESSION_SECURE=false a prefixed cookie would be
# silently dropped by the browser.
SESSION_COOKIE_PREFIX=

# Localization. `LocaleMiddleware` detects the per-request locale
# (session -> cookie -> Accept-Language) and falls back to APP_LOCALE;
# APP_FALLBACK_LOCALE is used when a message id is missing from the
# detected locale's catalog. Add a locale by creating lang/<locale>/
# with the same message ids as lang/en/.
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# Per-locale fallback parents consulted before APP_FALLBACK_LOCALE, e.g.
# a variant catalog inheriting from its closest relative first:
# APP_LOCALE_PARENTS=pt-PT=pt-BR

# Mail
#
# These are the names the framework's transport actually reads. Leave the
# credentials unset for a local catcher (maildev / mailpit / mailhog on
# 1025); set BOTH MAIL_SMTP_USER and MAIL_SMTP_PASS for authenticated
# STARTTLS. MAIL_SMTP_ENCRYPTION is derived from them when left blank —
# set it to `tls` for a relay expecting implicit TLS on 465. Production
# refuses to boot on an unencrypted connection.
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
