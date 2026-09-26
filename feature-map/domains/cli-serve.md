# Feature map: `manual/cli-serve.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 2 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova serve` · suprnova-cli/src/main.rs:60
  - Start the development servers (backend + frontend)
  - option `-p, --port <PORT>`: Backend port. Overrides SERVER_PORT/.env and pins the port exactly (no free-port scan). Defaults to SERVER_PORT, else 8765, scanning upward if that port is busy
- [ ] command `suprnova web:run` · suprnova-cli/src/main.rs:137
  - Run the web server (app runtime)
