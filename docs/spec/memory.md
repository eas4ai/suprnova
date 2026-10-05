# Memory footprint

Status: Agreed 2026-10-03
Prefix: MEM

The developer asked on 2026-10-03 to improve the memory footprint: every
item of the read-only allocation audit of 2026-10-02
(`.review/memory-audit-2026-10-02/report.md`, findings M01 to M29 and its
watchlist) validated against the code, and every validated one addressed;
and heap profiling an administrator switches on with a feature, with dhat.
Validation found 30 items that hold, 14 that hold in part and two that are
accurate but intentional; none was wrong. The requirements below are the
items that hold, grouped by what a reader can observe, and the defects the
validation found beside them.

Left as they are, each for its reason: the render cache's byte limit counts
encoded bytes only, as its documentation says, and counting more would
change eviction; the in-memory lease store keeps a token counter per key,
because Live spec 18 requires fencing tokens that only ever increase; the
query log grows while it is on and `DB::getQueryLog` returns a copy, as in
Laravel; captured process output has no byte limit, which is the documented
API; filesystem append keeps its atomic read and rewrite rather than a
backend's own append; the ImageMagick driver's output has no byte limit,
and adding one would be a new limit; and the brute-force lockout map keeps
every current lockout, since dropping one would raise its event again.

[MEM-001] A long-running process MUST NOT keep memory for work that is over.
The in-memory cache the framework binds MUST remove the expired entries no
one reads again, sweeping every `CACHE_SWEEP_INTERVAL` seconds (default 60;
0 turns the sweep off). The event dispatcher MUST reap finished
queued-listener tasks as it spawns new ones. Magnetar's single-flight map
MUST hold a key only while a caller holds or waits on its lock, a cancelled
caller included. The toast component MUST forget a toast that has left its
region. `DB::flush_query_log` MUST release the log's buffer.
Falsifier: 1,000 cache entries that expired without being read are still held two sweep intervals later, or a forever entry is gone; a dispatcher with a concurrency of 1 holds more than 2 tasks after 200 queued-listener runs and one more dispatch; Magnetar's single-flight map holds a key after every caller of it finished or was cancelled; the toast component keeps a timer for a toast a morph removed; or the query log's capacity is not 0 after a flush.
Mechanism: `mem-footprint`.
Rationale: audit findings M06, M28, M07, M29 and M26; each grows for the life of a server or a page.
Status: Agreed 2026-10-03

[MEM-002] A value the framework or the Live engine keeps MUST NOT reserve
capacity for what it never holds: the feature-flag identity map for its
distinct features, not its rows; a composite render-cache shell for its
literal bytes; an in-memory vector search for its k results; a request body
collected from several frames for its length; `Collection::pluck` for the
values it found when fewer than half the models had one; `find_all` for the
matches it found, never more than its limit; Live's resource queue keeps
its storage when a removal removes nothing; and Mailgun's form for the
fields the message has.
Falsifier: the identity map of 10,000 rows over 5 features has a capacity of 64 or more; a composite shell with cuts has a capacity above its length; a top-3 search over 1,000 items holds capacity above 3; a body sent as three 1,000-byte frames has a capacity above 3,000; plucking a missing field from 1,000 models leaves capacity; `find_all` with one match reserves more than 4; a queue removal that matches nothing moves the queue's storage; or Mailgun's fields have spare capacity.
Mechanism: `mem-footprint`.
Rationale: audit findings M05, M09, M04 and M20 and the watchlist's `http/body.rs`, `collection.rs`, `find_all` and `mailgun.rs`.
Status: Agreed 2026-10-03

[MEM-003] The framework, the Live engine and Magnetar MUST NOT make a full
copy of data they already own or only read on these paths: file downloads
(no buffer for a read that is not ready), the first Inertia page (one
buffer, every `/` in the page JSON still escaped), CORS wildcard paths
(compiled once per configuration), CSRF exemption rules, `Context::push`,
the queue worker's job payload, the SQS driver's decode, retries and
reservations, runtime casts, the last channel of a broadcast, image
transformations and encoding, Live upload reads, action arguments, signed
snapshots, composite headers and document paths, eager relation loading
(each key read from its field, not from the whole row serialized), process
results (moved where the call consumes the process; text decoded once when
the output is valid UTF-8), filesystem append and prepend, Magnetar's hex
encoding, and Mailgun's form-encoded fields. Every output MUST stay byte for
byte what it is today.
Falsifier: streaming a 4 MiB file allocates 6 MiB or more; an initial Inertia visit with a 1 MiB prop allocates at least 1 MiB more than the same visit as an Inertia request; 1,000 requests to a wildcard CORS path allocate at least one block per request more than to an exact path; a value `Context::push` promotes, a queue job's payload, the last broadcast channel's payload, a pure action argument or a full upload read is at a new address; the SQS decode, a retried SQS request, a runtime cast, an image step, a signed snapshot, a composite header, a document path, an eager load, a process result, an append or a hex encoding allocates the size of its data once more than the copy-free path does; or any response, frame, digest, snapshot, image, file or encoded string differs from today's.
Mechanism: `mem-footprint`.
Rationale: audit findings M01, M02, M08, M12 to M19 and M21 to M25, the watchlist's CSRF, runtime-cast, broadcast, Magnetar hex and Mailgun entries, and the copies validation found beside them (tokio's per-read buffer under M08, the mounted path under M21, the SQS reservation copy, 18 generated loader sites under M22).
Status: Agreed 2026-10-03

[MEM-004] The Live browser runtime MUST read a server-sent event stream
without copying the bytes it already holds for each new network chunk, MUST
NOT encode a record's data again to check its length, and MUST NOT allocate
a 16 KiB buffer for every upload control response. The size limit of one
record MUST apply to the record, so a network chunk larger than the limit
that holds only complete records is read. The distributed bundles MUST be
rebuilt from the changed sources.
Falsifier: a record fed one byte at a time copies its bytes once per byte; a 200 KB chunk of 100 small complete records ends the stream; an upload control response allocates 16 KiB; a record over the limit is accepted; or the committed bundles differ from a build of the sources.
Mechanism: `mem-footprint`.
Rationale: audit findings M10 and M11; validation found the chunk limit applied to the whole network chunk.
Status: Agreed 2026-10-03

[MEM-005] The tooling MUST NOT keep or copy what it does not use: the docs
builder keeps no chapter after writing it, `suprnova live:add` plans without
copying the component's files, the macros' name suggestions keep two rows,
not a matrix, and `live_component` renders its template in one pass with the
same output.
Falsifier: the docs builder's peak heap grows with the corpus beyond one chapter; `live:add` clones the component's files for its plan; the edit distance allocates a row per character; or `live_component`'s output differs from today's.
Mechanism: `mem-footprint`.
Rationale: audit finding M03, M24 and the watchlist's Levenshtein and `live_component` entries; build-time and one-shot costs.
Status: Agreed 2026-10-03

[MEM-006] The defects validation found beside the audit's items MUST be
fixed. The in-memory vector search MUST NOT panic when a score is NaN.
A `#[json_resource]` MUST keep its attributes in declaration order when it
drops a missing one, as Laravel does. The brute-force lockout map MUST
sweep at most once each time it has doubled since its last sweep, and its
comments MUST state the bound it keeps. The queue, schedule and workflow
workers MUST wait for their queued listeners when they shut down, as the
server does. `Cache::bootstrap` MUST keep a cache store the application
bound, as the localization chapter says it does. A toast whose timers are
paused by hover or focus MUST resume them when that toast leaves the
region.
Falsifier: a vector search panics on a NaN score; a resource with a missing first attribute reorders the rest; 2,000 live lockouts sweep on every new lockout; a worker exits while a queued listener runs; a bound cache store is replaced at serve time; or removing the hovered toast leaves the others paused.
Mechanism: `mem-footprint`.
Rationale: validation of the audit, 2026-10-03.
Status: Agreed 2026-10-03

[MEM-007] With the framework's `heap-profiling` feature, an application
MUST profile its heap with dhat: the framework installs dhat's allocator and
starts its profiler when the application runs, and when the process ends
normally, a command that finishes or a server that shuts down gracefully,
it writes `dhat-heap.json`, or the file `SUPRNOVA_HEAP_PROFILE` names, with
the total bytes allocated, the peak heap and the heap still live at the end,
for DHAT's viewer. Without the feature no dhat code is compiled. The
workspace and the application scaffold MUST offer a `profiling` Cargo
profile: the release profile with debug symbols, for samply or perf.
Falsifier: with the feature, a command that runs and exits writes no profile, or one without those totals; without it, dhat is in the framework's dependency tree; or the workspace or a new scaffold has no `profiling` profile that inherits the release profile and keeps debug symbols.
Mechanism: `mem-footprint`.
Rationale: the developer, 2026-10-03: profiling an administrator switches on with a feature flag, with dhat; oxc's profiling guide pairs dhat with a release build that keeps debug symbols.
Status: Agreed 2026-10-03
