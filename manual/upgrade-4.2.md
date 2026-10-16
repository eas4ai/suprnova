# Upgrading to 4.2

This chapter moves an application from Suprnova 4.1.0 to 4.2.0. Each section
matches an entry in the 4.2.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.1 to 4.2.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**.
Search and recommendations are new: nothing changes for an application until
it adopts them. Two fixes change what an existing application sends or
renders, and the Inertia kits' stylesheet fix needs three edits in an
application scaffolded before 4.2.

## Upgrade in this order

1. Point application dependencies and the CLI at `v4.2.0`:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.2.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.2.0 suprnova-cli
   cargo check --all-targets
   ```

2. Read the two fixed entries that change behaviour below, and make the
   Inertia stylesheet edits if your application uses an Inertia kit.
3. Run your tests, then load a page marked `Cache-Control: public` and check
   that it sets no cookie.

## Added entries

### Model search.

**Before:** 4.1 had no search layer.

**Now:** `#[derive(Searchable)]` and `Model::search` search a model through
the database's own full-text index, Qdrant, an in-memory collection or the
null engine, with optional hybrid search over embeddings.

**Do:** Nothing unless you adopt it. To adopt it, add
`suprnova::search::migrations()` to your migrator and follow
[Search](search.md). An index with a `scope` also names the fields that scope
reads with `scope_fields(...)`.

### Starter recommendations.

**Before:** 4.1 had no recommender.

**Now:** `Recommend` records events and reads scoped recommendations.

**Do:** Nothing unless you adopt it. To adopt it, add
`suprnova::recommendations::migrations()` beside the search migrations and
follow [Recommendations](recommendations.md).

## Fixed entries that change behaviour

### Public responses no longer carry the session or `XSRF-TOKEN` cookie (#154).

**Before:** The session and CSRF middleware set their cookies on every
response, including one a handler marked `Cache-Control: public`. A CDN or a
caching proxy could store that response with its cookies and replay them to
other visitors.

**Now:** A response marked `public` or given an `s-maxage` gets no refreshed
session cookie and no `XSRF-TOKEN` while the request leaves the session
unchanged. When the response must deliver a cookie (a new or rotated session,
a rotated remember-me cookie, one your handler set) or embeds the CSRF token
(`csrf_field()`, `csrf_meta_tag()`, the Inertia root template, a Live mount),
it goes out `Cache-Control: private, no-store` instead, with a warning in the
log.

**Do:** Nothing for the fix itself. If your application already strips these
cookies in a middleware registered before `SessionMiddleware`, that middleware
can stay; it no longer finds cookies to strip. If the warning appears for a
page you meant to cache, remove the cookie or the token from that page, or stop
marking it `public`. Purge any shared cache that stored responses with cookies
before you upgraded.

### Inertia kits paint styled under the Vite dev server (#153).

**Before:** The kits imported `app.css` from `main.ts` or `main.tsx`, so under
the Vite dev server a server-rendered first page painted unstyled until that
JavaScript ran.

**Now:** New applications list `src/app.css` as its own Vite entry, and the
first page loads it as a `<link>` with the request's nonce.

**Do:** In an application scaffolded with an Inertia kit before 4.2, make
three edits. Add the entry to the `InertiaConfig` in `src/bootstrap.rs`:

```rust,ignore
InertiaConfig::new()
    .frontend(Frontend::React)
    .entry_points(["src/app.css"])
    // ...your other options...
```

Add it to the build input in `frontend/vite.config.ts`, using your kit's
entry file:

```diff
-          input: 'src/main.tsx',
+          input: ['src/main.tsx', 'src/app.css'],
```

Then remove `import './app.css'` from the top of `frontend/src/main.tsx`, or
`main.ts` for Svelte and Vue.

### Why Suprnova diverges

Laravel's `StartSession` and `VerifyCsrfToken` attach their cookies to every
response, whatever its `Cache-Control` says. Suprnova keeps them off a
response a shared cache may store, so marking a favicon or a sitemap `public`
cannot hand one visitor's session to another.

## Fixed entries with no upgrade step

- **`Head::canonical()` keeps a `//` request path on the app's origin (#155).**
  The canonical link now stays on your `APP_URL` origin; nothing to change.
- **The CLI installs on macOS without warnings (#152).** Install the 4.2 CLI as
  shown above.
- **The reset email greets the user by name under Magnetar.** Nothing to
  change; the email reads the account's name.

## Next

- [Search](search.md) documents indexes, engines and the search builder.
- [Recommendations](recommendations.md) documents the event stream and the
  starter engine.
- [Upgrading to 4.1](upgrade-4.1.md) covers the preceding release.
