# Review - live-session-revocation

## Specification review, 2026-09-13 (before agreement)

Reviewed LIVE-019 and LIVE-020 in `docs/spec/live.md` against the code
that LIVE-016's fix left in place. The host records a session fingerprint
at issuance (`framework/src/live/context.rs`, `scope_facts`) and nothing
else: no membership remembers its session, the session middleware has no
hook toward Live when it destroys a row
(`framework/src/session/middleware.rs`, the rotation branch that calls
`store.destroy`), and delivery (`framework/src/live/async_updates.rs`,
`publish`) asks only the Gate. The fingerprint is a purpose hash of the
session id (`framework/src/session/middleware.rs`, where the Session fact
is recorded), so the id that rotation destroys maps to the fingerprint a
membership can carry.

### Attacked: contradictions

- LIVE-019 against the Live spec 14 note of 2026-09-13 that the clauses
  had no host substrate: the note describes the gap the requirement
  closes; no contradiction, and the entry of 2026-09-14 closes it.
- LIVE-020 against the render-cache collector, which records every
  `session()` read as a caching signal: delivery reads the store through
  `SessionStore::read` directly, outside any request, so no cache
  decision observes it.
- LIVE-020 against LIVE-018's issuance cap: re-verification runs on
  delivery, not issuance, and holds no slot.

### Attacked: falsifiers that would not catch a violation

The LIVE-019 probe must log out through the real session middleware,
otherwise nothing destroys a row and the hook has nothing to fire on. The
fixture therefore runs `SessionMiddleware::with_store` ahead of the fact
recorder and replays the cookie the first response set; a probe that
only cleared request state would pass for the wrong reason.

The LIVE-020 probe must prove the store was consulted, not that the
session expired: it delivers one event before removing the row, so a
membership that had never worked would fail the first assertion, and it
advances the adjustable clock past the interval rather than sleeping.

### Attacked: requirements no mechanism can check

None. Both probes drive the public transport end to end.

## Agreement record

Presented at 21:44 on 2026-09-13 as one set of two requirements and two
decisions, each with a recommended option. The developer at 23:22: "I
think you are capable of making the best decision here. I will accept
your recommendations", then "ok confimed". Recorded as `Status: Agreed
2026-09-13` on both blocks, the two decisions under `docs/decisions/`,
and the commitment's decisions section.

## Agreement record, LIVE-021

Presented at 08:46 on 2026-09-14 after the developer's `ok` on escalation
`live-019` (08:44). The developer at 08:48: "agreed". Recorded as
`Status: Agreed 2026-09-14` on the block.

## Mechanism demonstrations

### LIVE-019, `live-session-revocation`

Safe violating example: the probe as written, asserting the marker never
arrives after `Auth::logout_and_invalidate`, on the tree before the fix.
Baseline receipt `.cairn/evidence/LIVE-019/20260914T035220755Z` (fail):

    an event published after the session was destroyed reached the old stream

After the fix the same probe passes: the session middleware destroys the
rotated-away row, `live::revocation::session_destroyed` retires every
membership carrying that session's fingerprint through the unsubscribe
path, and the publish that follows finds no member. Receipt
`.cairn/evidence/LIVE-019/20260914T040450351Z` (pass) on `9a7ec88d`.

### LIVE-020, `live-session-reverification`

Safe violating example: the probe as written, asserting the marker never
arrives after the store row is removed and the clock passes ten seconds.
Baseline receipt `.cairn/evidence/LIVE-020/20260914T035221868Z` (fail):

    an event published after the store dropped the session reached the stream

After the fix the same probe passes: `publish` builds each candidate with
its attested session store id and the time the session was last known to
exist, asks the bound `SessionStore` when that is ten seconds or older,
and retires a membership the store no longer holds before the Gate is
consulted. Receipt `.cairn/evidence/LIVE-020/20260914T040454535Z` (pass)
on `9a7ec88d`.

## Limits recorded

- **Store outage.** A store that answers with an error leaves the
  membership in place and the question open for the next delivery, the
  same graceful degradation an ordinary request gets from a session store
  outage. Only "no such session" retires. The alternative, retiring on
  every error, would end every stream on a transient outage.
- **Session facts that are not store ids.** A membership is re-verified
  only when its attested session value has the shape of a store id
  (`is_valid_session_id`). The test fixture's `x-test-session` values are
  not, so the existing async suite is unaffected by a store another test
  bound; a production session id always is.
- **Plain `Auth::logout`.** Found here on 2026-09-14: `Auth::logout` keeps
  the session row and id, so LIVE-019's hook did not fire and the
  scaffold's logout controller left streams delivering. Raised as
  escalation `live-019`, answered `ok` at 08:44, and closed by LIVE-021
  below rather than narrowed.
- **Named guards.** LIVE-021 acts on the default guard's user, the
  principal a membership records at issuance; a named guard logging out
  leaves the default user's memberships in place.

### LIVE-021, `live-session-deauthentication`

Safe violating example: the LIVE-019 probe with the scaffold's plain
logout route, on the tree of `cf7332d1`. Baseline receipt
`.cairn/evidence/LIVE-021/20260914T124955920Z` (fail):

    an event published after a plain logout reached the old stream

After the fix the same probe passes: `Auth::clear_authentication`
captures the signed-in user and the session id before clearing either,
and `live::revocation::session_deauthenticated` retires the memberships
that session opened for that user through the unsubscribe path.
