# Rust
/target

# Cargo.lock is deliberately NOT ignored. This is an application, not a
# library: Cargo's own guidance is that binaries commit their lockfile so
# every machine, CI runner, and production image resolves the same
# dependency graph. The Dockerfile copies it for exactly that reason.

# Node
frontend/node_modules
frontend/dist
frontend/bootstrap/ssr

# Build outputs
/public/assets

# The Vite hot file `suprnova serve` keeps while the dev server runs.
/public/hot

# IDE
.idea
.vscode

# Environment
.env
.env.local
.env.*.local

# Mail preview output (MAIL_DRIVER=file) - .eml files carry live
# password-reset and email-verification tokens, so keep them out of
# version control.
storage/mail/

# live:add's lock and the journal it writes while it installs - this
# machine's state, never the project's.
/.suprnova-live.lock
/.suprnova-live-journal.json
