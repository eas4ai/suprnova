# Eloquent global scopes should observe the tenant they read -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-04
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

A registered global scope runs inside every `Model::query` call for its
model, and `ScopeRegistry::register`'s own documentation invites the scope
to read per-request state such as the current tenant id out of a
thread-local, a `tokio::task_local!`, or an atomic. None of those is an
instrumented accessor, so the request-scoped dependency collector records
nothing when a tenant-scoped global scope narrows the rows a render reads.
The render's own data is partitioned by tenant while the classification
sees no `TenantObserved` reason, so nothing narrows the class and the value
guard has no observed tenant to compare against the key. Unlike
`Request::live_tenant`, which is instrumented and does record the tenant it
resolves, this seam is invisible, and the only present remedy is for the
application author to know it and declare `Tenant` variance by hand on
every route whose models carry such a scope. The middleware's honest
boundary now names this rather than leaving it implied.

A future revision may have global-scope evaluation observe the identity it
reads, the way `Request::live_tenant` already does, so the value comparison
covers it instead of relying on the author remembering.

## Acceptance criteria

- Global-scope evaluation that reads a tenant records a tenant observation
  into the active collector, with the tenant as comparable material rather
  than a bare reason.
- A route whose model carries a tenant-scoped global scope and which
  declares no `Tenant` variance is declined rather than published, and the
  privacy leak suite proves it with a positive control that the same route
  declaring `Tenant` does cache and partitions.
- A scope that reads no per-request state records nothing and keeps caching
  exactly as it does today.
- A scope that reads per-request state the recording cannot resolve to a
  known variance dimension narrows to `Uncacheable` rather than being
  assumed harmless.
