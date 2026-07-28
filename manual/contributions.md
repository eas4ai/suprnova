# Contribution Guide

Suprnova is open-source under the MIT License, and the most valuable
contribution is a **good report**. The project does not accept pull
requests: the framework is maintainer-authored end to end, and every
change lands through the maintainers so the whole surface keeps one
shape. That's a deliberate, permanent posture — not a pre-1.0 phase.

MIT means you never need permission to take the code further yourself:
**fork freely**. A fork that grows in its own direction is a healthy
outcome, not a rivalry.

What that means in practice:

- **Bug reports** — welcome, via
  [GitHub issues](https://github.com/entrepeneur4lyf/suprnova/issues).
- **Feature requests** — welcome, via issues. Describe the use case, not
  the implementation; there's often a planned shape already (usually the
  Laravel equivalent).
- **Doc bugs** — welcome, via issues. If a chapter says an API exists and
  you can't find it, that's a doc bug — say which chapter and what you
  expected.
- **Security issues** — privately, by email (see below). Never as public
  issues.
- **Pull requests** — not accepted. PRs are closed with a pointer to
  this chapter; open an issue instead so the fix can land upstream, or
  fork and carry the change yourself.

## Filing a bug report that gets fixed fast

The gold standard is a reproduction from a fresh scaffold:

```bash
suprnova new repro-app --frontend vue --no-interaction
# …smallest change that shows the bug…
```

Include:

1. **What you did** — the commands and the code, trimmed to the minimum
2. **What you expected** — one sentence
3. **What happened instead** — the actual output or error, pasted verbatim
4. **Versions** — the framework tag (`suprnova --version`, or the `tag =`
   in your `Cargo.toml`) and your Rust version (`rustc --version`)

A failing test is even better than prose. If you can express the bug as a
test against the framework, paste it into the issue — it will usually
become the regression test the fix lands with.

## Building from source (to investigate a report)

You don't need this to *file* an issue, but reproducing against the
workspace often sharpens a report:

```bash
git clone https://github.com/entrepeneur4lyf/suprnova.git
cd suprnova
cargo check --workspace          # type-check everything
cargo test --workspace           # run the full suite (~3400 tests)
```

The workspace layout: `framework/` (the `suprnova` crate),
`suprnova-cli/` (the `suprnova` binary), `suprnova-macros/` (proc
macros), `app/` (internal dogfood app), `crates/` (payments and web-push
adapters), and `manual/` (this manual).

## The bar the code is held to

Not contributor rules — but knowing the standard helps you calibrate
reports (a panic from library code, a missing failure-mode test, or an
API that forces `unwrap()` is always report-worthy):

- **Full implementations only.** No TODOs, no partial scaffolds. A fix
  lands with the regression test that pins it.
- **Public-surface code returns `Result`, doesn't panic.** Where a
  Laravel-style infallible name ships, a `try_*` sibling ships with it.
- **No `unsafe` outside environment bootstrap.** The framework has exactly
  two `unsafe` blocks in non-test code, both in
  `config/env.rs::load_dotenv`, both wrapping `std::env::set_var` /
  `remove_var` — which became `unsafe` in edition 2024 — and both carrying
  a SAFETY note for the boot-time single-thread invariant they rely on.
  Everything else is test-only. New `unsafe` anywhere else needs a written
  justification in review, and `unsafe` in a driver, handler, or macro
  expansion will not be accepted.
- **`cargo fmt` and clippy under `-D warnings` are canonical.**

See [Error Model](error-model.md) for the full error contract.

## Security

Report security issues privately to
**shawn.payments@gmail.com** (the project maintainer). We'll
acknowledge within a few days, work the fix on a private branch, and
coordinate disclosure with you.

Do not file security issues as public GitHub issues until a fix has
shipped.

## License

MIT, with attribution to the upstream
[Kit project](https://github.com/dayemsiddiqui/kit) we forked from.
