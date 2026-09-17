# live:add reports an unedited older library file as edited locally, and its only way forward also overwrites real edits

Surfaced from: UI-017
Captured: 2026-09-17T11:53:47.965Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. suprnova-cli/src/commands/live_add.rs decides kept or replaced by comparing the existing bytes with the new bytes only; the manifest version is parsed and never used, and no installed hash is recorded. After a framework upgrade changes a component, every changed file of an unedited install is kept and labelled edited locally, and --force, the only way to take the new version, also replaces files the developer did edit.
