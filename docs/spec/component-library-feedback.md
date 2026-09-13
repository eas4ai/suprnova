# Component library - feedback and status

Status: Draft
Prefix: FDB

Refines Live spec 24
(`crates/suprnova-live/docs/specs/suprnova-live/24-feedback-and-status-components.md`):
inline alerts and validation summaries, toasts, loading and skeleton
states, progress, empty states, error and recovery states, success and
confirmation. Inherits every Agreed block of
`component-library-foundations.md`.

Components in complexity order. Presentational: alert and banner (callout
as a variant), skeleton, progress, spinner, empty state. Behavioral: toast
and flash region, notification bell, live feed. No custom-element tier in
this family.

## Presentational tier

[FDB-001] Every alert variant MUST carry a semantic role chosen by
urgency. An alert variant MUST NOT signal severity by color alone.
Falsifier: a shipped alert variant differs from its siblings only in color
tokens.
Mechanism: `.cairn/mechanisms/ui-live-check` (role and text presence) and
the token stylesheet check for a non-color cue per variant.

[FDB-002] Loading presentation MUST represent real queued or loading work
through the `live:loading` vocabulary, with an anti-flicker threshold.
Falsifier: a spinner or skeleton renders with no bound loading target, or
appears before the configured threshold.
Mechanism: `.cairn/mechanisms/ui-live-check`; a browserless harness case
for the threshold.

[FDB-003] The empty state MUST take its reason (empty, no results, no
permission, disconnected) from server-rendered state. The empty state
MUST offer a next action only when one is available.
Falsifier: a shipped empty state offers a create action the principal
cannot perform.
Mechanism: a dogfood document test per reason.

## Behavioral tier

[FDB-004] A toast MUST announce once without stealing focus. The library
MUST present a critical error in a persistent owning surface as well as
any toast.
Falsifier: a critical error is presented only as a toast, or a toast
moves focus.
Mechanism: a browserless harness case; `.cairn/mechanisms/ui-live-check`
for the live-region attributes.

[FDB-005] The live feed and notification bell MUST render a degraded
stream state honestly when the stream is retired or reconnecting.
Falsifier: a retired transport leaves the feed presenting itself as live.
Mechanism: the async-updates fixtures under
`crates/suprnova-live/browser/tests/` extended with the feed component;
an `app/tests/` end-to-end case.
