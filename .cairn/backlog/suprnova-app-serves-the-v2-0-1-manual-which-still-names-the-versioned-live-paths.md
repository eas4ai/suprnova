# suprnova.app serves the v2.0.1 manual, which still names the versioned Live paths

Surfaced from: LIVE-002
Captured: 2026-09-13T14:12:27.579Z

The framework repository's manual chapters are clean after 31fb0ead; only the six locale changelog mirrors quote __live/v1, by design, as history. suprnova.app is pinned to the v2.0.1 tag and its synced manual copy (content/docs/<locale>/live.md and the docs-staging copies) still carries __live/v1 paths because the de-versioning landed on main after that tag. Resolution is a site sync from the next framework tag (or a 2.0.2 release), not a manual edit in this repository. Raised by Shawn on 2026-09-13 at 09:57.
