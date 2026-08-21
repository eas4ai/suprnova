# Suprnova Live -- 24 Feedback and Status Components

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns official alert, validation summary, toast, status, progress,
spinner, skeleton, empty, error, success, offline/reconnecting, and confirmation
presentation components. It depends on library foundations, scheduling states,
actions/validation, uploads, asynchronous updates, and overlays. It presents
truthful system state without defining whether an action succeeded.

## Capabilities

### Inline alerts and validation summaries

Alerts and validation summaries shall present persistent contextual information,
warnings, errors, and success without abusing urgent live announcements. They
shall associate with the owning form, island, or content region.

Acceptance criteria:
- Informational, success, warning, error, and destructive variants have semantic
  roles chosen by urgency, not color alone.
- Validation summaries list actionable field errors and link/focus controls.
- Repeated morphs do not reannounce unchanged content unnecessarily.
- Dismissal is local unless the application explicitly persists it.
- Critical information remains available after announcement and at zoom/high
  contrast.
- Error content avoids leaking security or internal diagnostics.

UX flow:
1. Server returns validation or contextual feedback -> alert/summary appears in
   the owning region and announces proportionately.
2. Application user corrects/dismisses -> state clears under declared local or
   server policy without hiding unresolved field errors incorrectly.

### Toasts and transient notifications

Toasts shall communicate transient non-blocking outcomes with bounded queueing,
duration, pausing, dismissal, action, deduplication, and assistive-technology
behavior. They shall not be the sole presentation of critical or recoverable
information.

Acceptance criteria:
- Toast severity, accessible announcement, duration, persistence, and action are
  explicit.
- Queue length and duplicate coalescing are bounded.
- Hover/focus can pause dismissal; keyboard users can reach actions without
  focus theft.
- Critical errors and required decisions also appear in a persistent owning
  surface.
- Navigation and restored documents do not replay consumed toasts unexpectedly.
- Server effects request only registered safe toast data.

UX flow:
1. Accepted action emits a suitable transient outcome -> toast appears without
   stealing focus and announces once.
2. Duration ends or application user dismisses/acts -> toast leaves with
   reduced-motion support and queue advances.

### Loading, busy, and skeleton states

Spinner, progress placeholder, skeleton, and busy components shall represent
actual queued/loading work, preserve layout where useful, and avoid presenting
decorative motion as content. Initial SSR and lazy boundaries shall expose
meaningful loading/empty distinctions.

Acceptance criteria:
- Indeterminate and determinate work use appropriate patterns.
- Target-scoped loading does not block unrelated document or island controls.
- Skeletons have accessible alternatives, are hidden from redundant assistive
  output, and do not fabricate misleading content structure.
- Delayed/minimum visibility can prevent flicker without concealing latency.
- Reduced motion suppresses non-essential animation.
- Long-running work exposes cancellation or continuation information where
  available.

UX flow:
1. Work queues/starts -> target shows truthful busy/loading state after its
   configured anti-flicker threshold.
2. Work settles -> loading presentation yields to content, empty, success,
   interrupted, or error state.

### Progress and upload status

Progress bar, meter, step progress, and per-file/aggregate upload components
shall distinguish known completion percentage from indeterminate activity and
shall not infer durable domain success from transfer completion.

Acceptance criteria:
- Progress uses native semantics or equivalent min/max/value labeling.
- Update announcements are throttled to useful milestones.
- Transfer, verification, finalization, and workflow completion remain distinct.
- Regression/reset/cancellation and unknown total have explicit presentation.
- Color is not the sole indicator and high-contrast state remains visible.
- Multiple concurrent progress sources identify their owning task/file.

UX flow:
1. Long-running or upload work reports progress -> component updates visually and
   accessibly at bounded intervals.
2. Transfer reaches 100% -> status moves to verification/pending finalization
   until the server action confirms durable success.

### Empty, no-results, and initial states

Empty-state components shall distinguish a genuinely empty collection, no
filtered results, not-yet-created content, lack of permission, disconnected
data, and loading. They shall offer an appropriate next action only when one is
actually available.

Acceptance criteria:
- Empty reason is selected from authoritative application state.
- No-results state can clear/adjust filters without claiming the collection is
  globally empty.
- Permission-hidden actions are not offered.
- Empty visuals remain optional and do not replace explanatory text.
- Canonical SSR exposes empty state without requiring runtime.
- Morphs between loading/empty/content preserve region identity and focus.

UX flow:
1. Data resolves with no displayable items -> component explains the specific
   empty condition.
2. A valid next action exists -> user can create, clear filters, retry, or
   navigate through its real action/route.

### Error, interrupted, offline, and stale states

Recovery components shall present the classified failure and only the recovery
operations actually safe: retry, refresh island, discard, reconnect, sign in,
or navigate. They shall preserve existing content when the runtime does.

Acceptance criteria:
- Validation, authorization, conflict, transport, offline, protocol, render,
  morph, real-time freshness, and internal error presentations can be
  distinguished where actionability differs.
- Retry is shown only when idempotency/scheduling policy allows it.
- Refresh warns when unsaved local input or uploads may be lost.
- Offline state does not claim every failure is connectivity-related.
- Stale/freshness messaging appears only where materially relevant.
- Correlation/reference identifiers are safe and copyable without secrets.

UX flow:
1. Live reports a classified failure -> owning region retains safe content and
   presents valid recovery choices.
2. Application user chooses recovery -> runtime retries, refreshes, reconnects,
   signs in, discards, or navigates through the owning contract.

### Success and confirmation

Success components and confirmation patterns shall communicate accepted outcomes
proportionately. Destructive or consequential confirmation shall require a
clear application-user choice through accessible overlay or inline patterns,
not a styling-only convention.

Acceptance criteria:
- Success is shown only after accepted server outcome when authority is required.
- Self-evident changes need not add redundant announcements.
- Destructive confirmation names the object/action and consequence and uses
  explicit confirm/cancel controls.
- Confirmation does not smuggle arbitrary action names or skip current
  authorization.
- Undo is offered only when the domain truly supports a bounded compensating
  action.
- Focus and navigation after confirmation follow overlay/action contracts.

UX flow:
1. Application user requests a consequential action -> confirmation presents
   consequence and deliberate choices.
2. Application user confirms/cancels and server settles -> feedback reflects the actual
   outcome, not merely the click.

## Acceptance criteria

- Feedback components represent actual runtime/server states without becoming
  another authority.
- Announcements are accessible, proportional, deduplicated, and persistent when
  action is required.
- Loading, empty, error, stale, and success remain semantically distinct.
- Recovery controls appear only when their owning policy proves them safe.
- Transfer progress never impersonates durable application success.

## Decisions and revisions

- 2026-08-21 -- Feedback is truthful and proportional; rejected optimistic
  success language before server acceptance.
- 2026-08-21 -- Critical errors and required decisions cannot live only in
  transient toasts.
