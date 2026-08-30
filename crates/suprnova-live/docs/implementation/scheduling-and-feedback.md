# Scheduling and feedback

## Island scheduler

There is one bounded scheduler per island. It owns intent order, queue limits,
parallel limits, cancellation, transport settlement, response eligibility,
application order, recovery count, and final disposition. Islands do not share
a queue, so one slow component does not serialize unrelated components.

The closed policies are FIFO, replace-pending, drop-duplicate, latest-only,
and bounded parallel groups. Latest-only may suppress or abort eligible older
work according to its contract. Parallel transport never permits out-of-order
DOM application: an accepted response waits for the earliest still-eligible
intent. Queue overflow, duplicated intent objects, retired islands, stale
responses, and invalid policies finish with closed dispositions.

Model timing feeds the same scheduler. Debounce and throttle use injected
monotonic clocks; submit/action flushes the appropriate pending fields. Retries
retain the action identity and use bounded attempts/backoff. A seed's promotion
nonce is generated on first server intent, never on discovery or eager boot.

## Feedback and validation

Feedback is projected from actual model, scheduler, transport, application, and
recovery state. The closed states are idle, dirty, queued, loading, validating,
success, interrupted, offline, retrying, and error. Target modifiers may show,
hide, add the matching class, disable a native disableable control, set
`aria-busy`, or announce through polite/assertive live regions.

Loading and validating have a 150 ms reveal delay and 200 ms minimum visible
time; retrying has a 200 ms minimum; terminal success/interruption/error remains
for at least 100 ms and resets after two seconds. These timing rules prevent
flicker without claiming completion before the scheduler does. Baseline hidden,
disabled, class, ARIA, role, and text state is restored when targets retire.

Server validation errors bind to their registered field paths and do not masquerade
as transport errors. New local edits remain dirty even when an older response
contains a value for that field. Accessible announcements use closed messages
and never repeat server payloads or exception text.

## Failure and recovery

Transport timeouts, offline state, incompatible protocols, rejected/stale
responses, morph-preflight failures, morph exceptions, and successor mismatch
have distinct dispositions. A terminal redirect skips morphing and uses native
navigation. A successful nonterminal response follows: validation, preflight,
morph, focus restoration, successor metadata commit, events, effects, child
parameter scheduling, URL reflection, then final feedback.

If the server has advanced but the browser cannot commit the new DOM and
metadata, the runtime never retries the original action from the old snapshot.
It claims one of a bounded number of recovery attempts and requests a fresh
render for that island. Exhaustion leaves the existing SSR DOM visible, marks a
closed failure state, and requires an explicit page navigation/reload. Retiring
an island aborts or suppresses all work and releases every feedback observer.
