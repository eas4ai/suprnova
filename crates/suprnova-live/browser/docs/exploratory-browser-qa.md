# Exploratory browser QA

This workflow supplements the automated browser conformance suite. It is useful for discovering
problems and collecting a reproducible report, but it is not release evidence. The pinned Playwright
Chromium, Firefox, and WebKit projects remain the qualification gate. Playwright WebKit MUST NOT be
described as Safari.

## Start the local host

From `browser/`, start the built conformance host in a dedicated terminal:

```sh
npm run host:static
```

The wrapper builds the production artifacts once, then starts the static host. The raw
`node test-host/server.mjs` command never builds or changes `dist`; it validates and serves an
existing completed build and fails before binding if that build is missing, incomplete, or invalid.
Confirm `/health` returns `ok` on `http://127.0.0.1:4173` before exploring a scenario.

## Agent-browser session

Derive a project-local session instead of reusing the default daemon session:

```sh
QA_SESSION="$(agent-browser session id --scope worktree --prefix suprnova-live-qa)"
agent-browser --session "$QA_SESSION" open http://127.0.0.1:4173/scenario/fullFlow
agent-browser --session "$QA_SESSION" snapshot -i
```

Use the snapshot as the before-state record. Prefer the accessibility refs and semantic controls:

```sh
agent-browser --session "$QA_SESSION" find role button click --name "Show local panel"
agent-browser --session "$QA_SESSION" snapshot -i
agent-browser --session "$QA_SESSION" find label "Query" fill "composed"
agent-browser --session "$QA_SESSION" find role button click --name "Run server action"
agent-browser --session "$QA_SESSION" wait --url "**/scenario/fullFlow?state=done"
agent-browser --session "$QA_SESSION" snapshot -i
```

Every DOM-changing action invalidates prior `@eN` refs. Take a fresh snapshot before the next ref
interaction. Exercise disclosure, tab, form, dirty-guard, ordinary-link, server-action, retry,
offline, transition, and bfcache scenarios using their exposed roles and labels rather than brittle
coordinates or generated selectors.

Inspect failures and authority boundaries during the same session:

```sh
agent-browser --session "$QA_SESSION" network requests --filter /live
agent-browser --session "$QA_SESSION" errors
agent-browser --session "$QA_SESSION" console
agent-browser --session "$QA_SESSION" a11y --tags wcag2a,wcag2aa
```

Screenshots are optional exploratory artifacts. Before capture, remove or mask credentials, session
identifiers, snapshot envelopes, tokens, personal data, and sensitive URL parameters. Save only
reviewed, redacted images; never publish an unreviewed screenshot, trace, HAR, or video.

```sh
agent-browser --session "$QA_SESSION" screenshot --annotate /tmp/suprnova-live-redacted.png
agent-browser --session "$QA_SESSION" close
```

Always close the derived session when the pass is complete, including after an error.

## DevTools MCP pass

When Chrome DevTools MCP is available, use a separate project-local browser session and inspect:

- lifecycle transitions across `pagehide`, persisted `pageshow`, `freeze`, and `resume`, including
  document-token continuity and exactly one runtime instance;
- the Memory panel before and after repeated connect, morph, removal, suspend, and restore cycles,
  looking for retained island roots, controllers, listeners, observers, timers, and abort handlers;
- the Performance panel around local-only interaction, one accepted Live action, a hostile rejected
  response, and native navigation, checking for long tasks and unexpected layout or script work;
- Event Listener and DOM breakpoint views for duplicate delegated listeners or observers after
  restore;
- the Application/back-forward-cache diagnostics for explicit rejection reasons and successful
  restoration where the engine supports it;
- Network request payloads and ordering without copying signed snapshots, cookies, or authorization
  material into the report.

Record reproducible observations and exact scenario URLs. DevTools MCP observations remain
exploratory; promote any regression into deterministic Playwright or unit coverage before treating
it as closed.
