# Upgrading to 4.3

This chapter moves an application from Suprnova 4.2.1 to 4.3.0. Each section
matches an entry in the 4.3.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.2 to 4.3.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below. Nothing in 4.3.0 needs an edit in an existing
application.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.3.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.3.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.3.0 suprnova-cli
   cargo check --all-targets
   ```

2. Run your tests.

## Fixed entries with no upgrade step

- **A local embedding model keeps its memory bounded on long documents.** Nothing to change.
  An application that lowered `SEARCH_EMBED_BATCH` to stay within memory can
  return it to the default of 64.

## Next

- [Search](search.md) documents the embedders and `SEARCH_EMBED_BATCH`.
- [Upgrading to 4.2](upgrade-4.2.md) covers the preceding release.
