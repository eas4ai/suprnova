# AGENTS.md instructions

<INSTRUCTIONS>
@/home/shawn/.codex/RTK.md
@/home/shawn/.codex/TILTH.md
@/home/shawn/.codex/PARTNERSHIP.md
@/home/shawn/.claude/DEVELOPMENT_PRACTICES.md
@/home/shawn/.claude/BEST_PRACTICES.md

## Working agreement (Same Page)

This project is spec-driven. The normative set lives at
`docs/specs/suprnova-live/`: `00-overview.md` is the architecture keystone and
spec map; numbered specs own their domains; `ux.md` owns interaction;
`glossary.md` owns vocabulary; and `conventions.md` owns implementation and
verification rules. When project vocabulary conflicts with a model prior, the
glossary wins.

Read the imported development and production practices before working. A future
repository-local `DEVELOPMENT_PRACTICES.md` or `BEST_PRACTICES.md` overrides its
user-level counterpart.

## Repository layout

| Path | Role |
|---|---|
| `docs/specs/suprnova-live/` | Normative architecture, domain, UX, glossary, and convention contracts |
| `docs/specs/suprnova-live/iterations/005.md` | Current confirmed scope contract |
| `src/` | Rust engine source created by iteration 001 |
| `browser/` | Strict TypeScript runtime and cross-language conformance package |
| `scripts/check-specs.mjs` | Structural, link, decision-order, iteration, and optional-archive drift gate |
| `scripts/gate.sh` | Unattended project gate created by iteration 001 |
| `reference/` | Ignored, non-normative pinned sources and comparative evidence |
| `/home/shawn/workspace2/suprnova` | Internal-crate destination authorized by iteration 005; modify only in the isolated integration worktree |
| `/home/shawn/workspace2/suprnova-magnetar` | Separate active project; not part of Live scope |

Suprnova Live is being developed here as a future internal Suprnova crate. It is
not a third-party crate. Development remains here until workspace separation is
a material integration, testing, or coherent-change blocker. A confirmed
integration iteration then moves product code, normative specs, and the checker
together; never maintain two authoritative copies.

## Scope and re-anchor rules

- The current iteration contract is
  `docs/specs/suprnova-live/iterations/005.md`. Implement only its `In` section;
  its `Out` section is named build-order sequencing, not permission to omit
  agreed final functionality.
- New ideas go through `/next-iteration`: surface and capture them, never
  implement them ad hoc and never silently discard them.
- When incoming direction contradicts a confirmed spec, return to the owning
  document and confirm the change before acting.
- Add new or collided terms to `glossary.md` as soon as their project-specific
  meaning is confirmed.
- Preserve unrelated and concurrent work. Iteration 005 authorizes Suprnova
  changes only in its isolated integration worktree; do not edit Magnetar.
- Keep exactly one todo item in progress. A task is not done until its code and
  proportionate verification both pass.
- Clippy findings are reviewed and resolved without blanket `-D warnings`.
  Intentional suppressions are narrow and use a written `reason`.
- Prefer codebase-memory graph tools for code discovery when available; use
  `tilth` next, and fall back to literal search only when structural tools are
  insufficient.

## Verification

For specification-only changes:

```bash
node scripts/check-specs.mjs
git diff --check
```

The ZIP is an optional Fable handoff artifact. If it is absent, do not create it
merely to satisfy the checker; if it is present, regenerate it before checking:

```bash
(cd docs/specs && zip -X -q -FS -r suprnova-live.zip suprnova-live -i '*.md' -x 'suprnova-live/iterations/next/*')
node scripts/check-specs.mjs
```

After iteration 001 creates the Rust and browser workspaces, run the affected
targeted checks while iterating and the following complete gate before calling
the iteration done:

```bash
CARGO_INCREMENTAL=0 cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features
CARGO_INCREMENTAL=0 cargo test --all-targets --all-features --no-fail-fast
CARGO_INCREMENTAL=0 cargo test --doc --all-features
(cd browser && npm ci)
(cd browser && npm run format:check)
(cd browser && npm run lint)
(cd browser && npm run typecheck)
(cd browser && npm test)
(cd browser && npm run build)
(cd browser && npm run budget)
CARGO_INCREMENTAL=0 scripts/gate.sh
```

Never run heavy Cargo builds concurrently with another build in Suprnova,
Magnetar, or this workspace. Report a check as passing only when that exact
command ran successfully.
</INSTRUCTIONS>
