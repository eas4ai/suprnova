# Feature map: `manual/dev-tls.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 1 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova dev:tls` · suprnova-cli/src/main.rs:115
  - Register an HTTPS dev URL (https://<name>.localhost) and trust portless's CA in your browsers' certificate stores
  - option `-p, --port <PORT>`: Backend port to route to. Defaults to SERVER_PORT, else 8765
