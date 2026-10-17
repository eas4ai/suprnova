# Upgrading to 4.3

This chapter moves an application from Suprnova 4.2.1 to 4.3.0. Each section
matches an entry in the 4.3.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.2 to 4.3.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**. One
change can need an edit if you build settings structs field by field, one fix
adds a table that your migrations create, and one changes which suggestions
the starter recommender makes. The graph recommender and media embeddings are
new: nothing changes until you turn them on.

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

2. Run your migrations, so the `search_source_versions` table exists before a
   searchable model without `updated_at` is saved or loaded.
3. Run your tests.

## Changed entries

### Recommendation, search and explanation structs carry new fields.

**Before:** `RecommendConfig`, `SearchConfig`, `IndexTimeSettings` and
`RecommendationExplanation` had the fields 4.2 documented.

**Now:** each has new public fields: the graph settings and `min_peers` on
`RecommendConfig`, `media_max_bytes` on `SearchConfig`, `embed_media` on
`IndexTimeSettings`, and `engine`, `evidence` and `fallback_reason` on
`RecommendationExplanation`.

**Do:** where your code builds one of them with every field spelled out, end
the literal with `..Default::default()`. Code that reads fields or already
uses `..Default::default()` needs no edit.

## Fixed entries that change behaviour

### A model without `updated_at` keeps a search version per row.

**Before:** a searchable model without `updated_at` put version 0 in every
search document, so nothing downstream could tell one revision of a row from
the next.

**Now:** each save raises a version the framework keeps per row, in the save's
transaction, and Eloquent loads the kept versions beside the rows it returns.
A model with `updated_at` is unchanged.

**Do:** run your migrations. The table comes with
`suprnova::search::migrations()`, which your migrator already appends. Until
it exists, saving or loading such a model on the `database` search engine
fails with an error naming `search_source_versions`.

### A starter suggestion stands on at least three peers.

**Before:** a single peer who shared an item with the viewer could produce a
collaborative suggestion, and its explanation showed a peer count of 1.

**Now:** a collaborative suggestion needs `RECOMMEND_MIN_PEERS` distinct peers,
default 3; a candidate with fewer arrives through a later tier or not at all,
so an application with few active users shows fewer collaborative suggestions.

**Do:** nothing, unless your page reads `peer_count` or `fallback_tier` and
assumed small values. Lower `RECOMMEND_MIN_PEERS` only if your users accept
that a suggestion can reveal one other user's activity.

## Fixed entries with no upgrade step

- **A local embedding model keeps its memory bounded on long documents.** Nothing to change.
  An application that lowered `SEARCH_EMBED_BATCH` to stay within memory can
  return it to the default of 64.

## Next

- [Search](search.md) documents the embedders and `SEARCH_EMBED_BATCH`.
- [Upgrading to 4.2](upgrade-4.2.md) covers the preceding release.
