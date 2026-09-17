# live:add copies the target of a symlink in a third-party component into the project

Surfaced from: UI-017
Outside because: live-protocol-bounds bounds what one Live request and response carry (LIVE-028 to LIVE-030); how live:add reads a third-party component's files is not part of that work.
Captured: 2026-09-17T11:53:48.008Z
Promoted to: live-library-review-remediation

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. suprnova-cli/src/commands/live_add.rs reads each third-party file with fs::read_to_string(directory.join(file)), which follows symlinks; the destination side refuses symlinks through secure_fs::ensure_contained, the source side does not. Reproduced with the built CLI: a component directory whose widget.js was a symlink to a key file installed the key's bytes as templates/acme-ui/widget/widget.js.
