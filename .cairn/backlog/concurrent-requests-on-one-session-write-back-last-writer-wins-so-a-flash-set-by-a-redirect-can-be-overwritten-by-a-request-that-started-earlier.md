# Concurrent requests on one session write back last-writer-wins, so a flash set by a redirect can be overwritten by a request that started earlier

Surfaced from: FDB-003
Captured: 2026-09-15T15:21:10.199Z

Observed 2026-09-15 in the dogfood browser suite: after the demo login lands on the dashboard, the islands' asynchronous transport requests (subscriptions, memberships) each load the session, and a navigation to the flash route that starts while one is in flight loses its flash when that request's response writes its older session copy back. The session middleware has no per-session serialization or merge of concurrent writes. Laravel offers session blocking (atomic locks) as an opt-in for this case. The dogfood flash case now waits for the dashboard's islands to connect before navigating, which removes the overlap from the case, not the defect.
