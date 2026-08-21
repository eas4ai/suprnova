# Legacy fixture corpus

Every fixture here is source-derived test evidence minted by the *live*
deployed code, never by Magnetar's own dependencies. `manifest.json` is the
owner of record: paths, checksums, revisions, commands, and generation
requirements. `tests/fixtures_manifest.rs` enforces the manifest against this
directory on every run.

## Hash corpus

Two generated fixtures capture the deployed password-hash formats while both
codebases were current:

| Fixture | Minted by | Revision | Parameters |
|---|---|---|---|
| `hashes/suprnova-bcrypt.json` | `suprnova::hashing::hash` (framework default driver) | Suprnova `27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88` | bcrypt cost 12; inputs above 71 bytes are rejected up front |
| `hashes/torii-argon2.json` | `password_auth::generate_hash` (torii password lane) | Torii `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d` | Argon2id `v=19, m=19456 KiB, t=2, p=1` |

### Exact minting procedure

Each generator was a throwaway binary pinned to the live source revision and
run with the recorded command:

- bcrypt: `cargo run --quiet --manifest-path /tmp/suprnova-default-generator/Cargo.toml`,
  whose entire program is a loop over the five input lengths calling
  `suprnova::hashing::hash(&password)`.
- Argon2: `cargo run --quiet --manifest-path /tmp/torii-fixture-generator/Cargo.toml`,
  the same loop calling `password_auth::generate_hash(password.as_bytes())`.

### Input derivation

The inputs are deterministic, non-secret byte strings: the ASCII byte `p`
(0x70) repeated to lengths **32, 71, 72, 73, and 128** - that is,
`"p".repeat(len)`. No live credential or production secret was ever used, so
recording the derivation here leaks nothing. The 72/73/128-byte bcrypt rows
record the framework's up-front rejection instead of a hash, because the
deployed hasher refuses inputs above its 71-byte usable limit.

`tests/password_hash_fixtures.rs` re-derives these inputs and drives the
corpus through Magnetar's dual-format verifier; changing the derivation or
lengths without re-minting the corpus will fail those tests.

## Database fixtures

The three whole-database fixtures are generated, immutable **source-derived**
SQLite artifacts for the migration domain (12), not Magnetar-shaped schemas
or standalone-password inputs. Their owner of record is `manifest.json`;
its revision, command, seed/encryption parameters, and SHA-256 must move
together with a deliberate re-mint.

| Fixture | Source revision and schema | Generator command | SHA-256 |
|---|---|---|---|
| `databases/torii.sqlite` | Torii `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d`; `torii-storage-seaorm` migrations/entities and passkey repository envelope | `cargo run --quiet --manifest-path /tmp/sdd-magnetar-003/fixture-generator/Cargo.toml -- /home/shawn/workspace2/suprnova-magnetar/tests/fixtures/databases` | `df95224f6b5bf65cd6b2c8d7f6c183ca4892284ef0a546d0aaba512344123c1d` |
| `databases/suprnova-web.sqlite` | Suprnova `27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88`; backend templates plus framework auth-flow/2FA migrations | `cargo run --quiet --manifest-path /tmp/sdd-magnetar-003/fixture-generator/Cargo.toml -- /home/shawn/workspace2/suprnova-magnetar/tests/fixtures/databases` | `0e4680a9a63a98c360439e03e7468ab9fa0204d2ea3aeaeaa6a976f482c038ed` |
| `databases/suprnova-api.sqlite` | Suprnova `27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88`; API `app_users` migration template | `cargo run --quiet --manifest-path /tmp/sdd-magnetar-003/fixture-generator/Cargo.toml -- /home/shawn/workspace2/suprnova-magnetar/tests/fixtures/databases` | `e4571b553033cc37dffac84ebdcc9364277d4da548d6f759ebdb0aecc4bf02ad` |

The generator was compiled only against isolated `/tmp/sdd-magnetar-003`
exports made with `git archive` from those exact commits; it did not use
Magnetar's schema or dependencies, nor the different current Suprnova
worktree `HEAD`. Its Torii rows cover the case-normalization collision,
passwordless user, passkey envelope, linked account, verification timestamp,
and session. Its web row has an empty source password field, fixed
verification/session values, and 2FA ciphertext produced by
`Crypt::encrypt_string(CryptPurpose::TwoFactorSecret, ...)` under the
fixture-only all-`0x42` key encoded as
`QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI`; the source API's fresh
AES-GCM nonce is frozen in the immutable checked artifact. The API fixture
contains only the deterministic `app_users.id = 4242` source row.

`tests/fixtures_manifest.rs` recomputes every manifest-owned artifact's
checksum. To inspect the generated source shapes without running a project
suite, use `sqlite3 tests/fixtures/databases/torii.sqlite '.tables'`,
`sqlite3 tests/fixtures/databases/suprnova-web.sqlite '.tables'`, and
`sqlite3 tests/fixtures/databases/suprnova-api.sqlite '.tables'`.
