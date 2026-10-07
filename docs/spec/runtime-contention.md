# Runtime contention

Status: Draft
Prefix: RTC

Drafted 2026-10-07 from issues #146 to #149, each confirmed against main
`4241b80` and answered on the issue with the change written here. The four
share one cause: work that holds a Tokio worker, a blocking-pool thread or
a lock longer than it must, so unrelated requests wait behind it. The
Observed section describes main `09144c48b`; only the requirements are
contract once Agreed. The issues cite "Principles for fast Tokio
applications" (dial9-rs) for the Tokio guidance behind each change.

The `runtime-contention` mechanism runs the framework's in-source unit
tests and its `hashing`, `http`, `rate_limit`, `memory` and
`magnetar_integration` test binaries under nextest, filtered to the `rtc_`
tests plus the whole `hashing` and `rate_limit` binaries, so each test is
its own process and a test that installs a hasher or builds a runtime of
its own disturbs no other.

## Observed at 09144c48b

- `framework/src/hashing/mod.rs`: `hash_async`, `hash_with_cost_async`,
  `verify_async` and `rehash_for_laravel_async` each call
  `tokio::task::spawn_blocking` with no limit, and Magnetar's
  `run_hash_work` (`crates/suprnova-magnetar/src/password/hash.rs`) does
  the same. Tokio's blocking pool holds 512 threads by default; an Argon2
  hash under the framework's defaults takes 64 MiB, Magnetar's Argon2id
  19 MiB.
- `framework/src/http/file_response.rs`: a file above the 1 MiB buffered
  limit streams through `FileByteStream`, which spawns one blocking task
  per 64 KiB chunk (`FILE_CHUNK_SIZE`); a 1 GiB download is 16,384 trips
  through the pool.
- `framework/src/rate_limit/memory.rs`: one `Mutex<HashMap<String, Bucket>>`;
  `try_acquire` and `retry_after` lock the whole map, `try_acquire`
  allocates the key on every call, and the periodic sweep and
  `purge_inactive` run `retain` over the whole map under that lock.
- `framework/src/server.rs`: `WS_TASKS` is a `tokio::sync::Mutex<JoinSet<()>>`;
  the upgrade path locks it to reap and spawn, and the shutdown drain
  holds it across the `select!`, the 5 s deadline and `abort_all`.

## Requirements

[RTC-001] Password hash work in the framework's `hashing` module and in
Magnetar (a hash, a verify, a rehash) MUST run on the blocking pool under
one process-wide limit on how many pieces run at once. The limit is
`HASH_MAX_CONCURRENCY`, defaulting to the host's available parallelism; a
value that is not a whole number of at least 1 MUST be refused when the
hashing configuration loads. Work past the limit MUST wait as a task
holding no thread, and the permit MUST travel with the work, so it is
released when the work returns even when the caller stopped waiting.
Outside a runtime the work runs inline, as today.
Falsifier: with the limit at two and a hasher that waits for the test's signal, a third piece of work enters the hasher before one of the first two returns, or a fourth piece sent through Magnetar enters while the framework's two hold the permits; a piece whose caller dropped its future still holds its permit after the work returns; or `HASH_MAX_CONCURRENCY=0` or `HASH_MAX_CONCURRENCY=two` loads.
Mechanism: `runtime-contention`.
Rationale: issue #146; a burst of sign-ins is then bounded to the limit times one hash's memory, and the excess waits as tasks rather than as threads holding 64 MiB each.
Status: Draft

[RTC-002] A file response above the buffered limit MUST be read by one
blocking task for the whole body: it reads chunks of at least 256 KiB in a
loop and hands each to the response over a channel holding at most four
chunks, so a body costs one blocking-pool trip rather than one per chunk,
a client that stops reading holds at most those chunks in memory, and a
response that is dropped stops its reader. The response MUST keep today's
exact `Content-Length` and bytes, and MUST still end the body short, so
the connection aborts, on a read error or a file that shrank.
Falsifier: on a runtime whose blocking pool has one thread, a blocking task started after a 4 MiB download began finishes before the download's last chunk; a chunk under 256 KiB reaches the body before the last one; a body whose consumer stops after the first chunk has its file read further than five chunks past what the consumer took; a reader is still running after its response was dropped; the body's bytes or `Content-Length` differ from the file's; or a file truncated while streaming completes a body of its declared length.
Mechanism: `runtime-contention`.
Rationale: issue #147; the per-chunk round trips were most of a large download's cost and competed with every other blocking-pool user.
Status: Draft

[RTC-003] The in-memory rate limiter MUST keep its buckets so that a
request waits on at most the buckets that share its part of the map,
never on the whole map: the periodic sweep and `purge_inactive` MUST
examine one part at a time, releasing the rest, so requests on other keys
keep completing while a sweep runs; and a request for a key that already
has a bucket MUST NOT allocate for the key. Acceptance, rejection and
retry-after answers, the sweep's self-termination with the limiter's last
reference, and poison recovery MUST stay as they are.
Falsifier: while a sweep over 200,000 buckets runs, fewer than 100 requests on other keys complete strictly inside the sweep's span; a request on an existing key that the window rejects allocates; a sequence of hits gets an acceptance, rejection or retry-after answer that differs from today's; or the sweep task survives the last reference to the limiter.
Mechanism: `runtime-contention`.
Rationale: issue #148; one lock for every key gave every worker a P99 spike at each sweep, growing with the number of keys an attacker can rotate.
Status: Draft

[RTC-004] The server MUST register a WebSocket handler task and drain the
registered tasks at shutdown without holding a lock across an await:
registration holds a synchronous lock only to reap finished handles and
insert, and the shutdown drain MUST take the set out under that brief lock
and wait for it holding no lock, up to its 5 s deadline and then aborting
what still runs, so a registration during the drain completes at once.
Falsifier: with a handler that never finishes registered and the drain started, a registration issued after the drain began completes only after the drain does; the drain returns before its deadline while a registered handler is still running; or a handler still running at the deadline is not aborted.
Mechanism: `runtime-contention`.
Rationale: issue #149; the async mutex existed only for the drain, and the drain held it for up to 5 s against every upgrade in flight.
Status: Draft
