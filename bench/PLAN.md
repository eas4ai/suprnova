# Suprnova benchmark plan

What a framework buyer cares about: throughput on real routes, the
architectural advantage of async, ORM performance at scale, real-time
capacity, and stability over time.

This replaces an earlier layer-isolation design (bare router, router +
middleware, middleware + session, …). That design answered "where does
each microsecond go" — useful for internal profiling, wrong for proving
value, and it had no concurrency model at all, so it could not answer
"how many users can this serve."

Method carried forward from Phase 1: open-loop generation for published
latency, closed-loop for capacity discovery, median of >= 3 runs, full
environment recording, and the compose stack in `bench/compose/`.

---

## Ground rules

**Nothing is capped.** Not the SUT, not the database, not the load
generator. The host is otherwise idle; there is no contention to
schedule around. An earlier revision confined the generator to six
cores, it spent all six, and the entire sweep measured the generator
rather than the server:

```
generator_peak_cpu_pct: 586 (of 600 available)
```

Generator CPU is still sampled every run — recording is not throttling,
it is how a reader tells a server number from a generator number. If the
generator ever exceeds half the machine, the run is flagged and the fix
is a second host, never a smaller budget for either side.

**Both stacks get the whole machine, one at a time.** Never
concurrently: a Laravel run interleaved with a Suprnova run introduces
"what else was on the host" as a variable.

**One host means one caveat that bounds every tier.** The generator
shares the machine with the SUT, so nothing here crosses a real network
— loopback has no NIC, no meaningful congestion control, and a 64 KB
MTU. For small JSON bodies that is negligible. For large responses it is
not, and any figure whose units are bytes per second is a property of
this host's memory bandwidth rather than of serving. Latency and request
rate on small bodies survive; byte throughput on large ones does not.
Where that distinction matters the tier says so explicitly.

**Run durations follow what time actually changes.** Sustained runs are
for measurements where time is a variable. Where a scenario reaches
steady state in seconds and then reports the same number indefinitely, a
long run buys nothing.

| Duration | Scenarios | Why |
|---|---|---|
| 2 h | soak | RSS slope, FD drift, latency drift — the entire point |
| 1 h | one Tier 1 latency run | crosses two 30-min sqlx `max_lifetime` cycles, so connection recycling is observed rather than assumed |
| 5-10 min | everything else | steady state arrives in seconds; minute 2 and minute 60 report the same figure |
| 30-60 s | discovery sweeps | throwaway — only the argmax is used, nothing is quoted |

Percentile volume is not a reason to run long: at 50k rps, five minutes
is 15 million requests, which is 15,000 samples inside the p99.9 tail.

Total campaign: roughly 7 hours per stack, 14 hours for both.

An earlier revision of this plan set one hour for every measurement,
which totalled 44 hours per stack. That figure came from multiplying a
duration by a scenario count rather than asking which scenarios time
affects. It is recorded here because the same reflex — applying a rule
uniformly instead of where it applies — is what put a 30-second cap on
warmup and a cpuset on the load generator.

---

## What in this document is verified

An earlier draft of this plan asserted an API that does not exist
(`DB::parallelize()`), an Octane worker default off by roughly 2.7x, and
a load-generator allocation contradicted by our own logged
`586 (of 600)`. All three were confidently written and all three were
one command away from being caught.

So this section states which claims were checked against a source, and
which are still assumptions.

**Checked:** host topology (`lscpu`) · Octane's per-core worker default
and 500-request recycle (`reference/docs-13.x/octane.md`) ·
`Octane::concurrently()` semantics and its 1024-task cap (same) ·
`Concurrency::run()` drivers, and that `fork` cannot run in web requests
(`reference/docs-13.x/concurrency.md`) · that `DB::parallelize()` does
not exist · sqlx pool defaults, `idle_timeout` 10 min and `max_lifetime`
30 min (`sqlx-core-0.9.0/src/pool/options.rs:161`) · that the framework
sets neither · the five model relations Tier 3.2 needs ·
`Post::for_author` / `Post::all_public` · what the three stub routes
actually return · vegeta 12.13.0's `-targets` support.

**Still assumed, and load-bearing if wrong:** that PHP's Tracing JIT
stabilises within a 60-second warmup · that a Swoole worker holds its
database connection for a request's duration · that Reverb is the
representative Laravel real-time deployment · that SSE sustains a higher
connection ceiling than WebSockets.

Each of those is a premise behind a tier's design, not a result. Check
before publishing anything that depends on one.

---

## Host

AMD EPYC 4545P, 16 physical cores / **32 logical CPUs**, 128 GB RAM,
2 TB NVMe. Confirmed by `lscpu`, recorded per run by `env-record.sh`.

No core allocation table. Both the SUT and the generator run unpinned
across all 32 threads.

### Runtime configuration

**Suprnova**
- Single process, Tokio multi-threaded runtime, default worker thread
  count (= 32)
- Work-stealing scheduler: any worker thread picks up any ready task
- `DB_MAX_CONNECTIONS=150`, `DB_MIN_CONNECTIONS=16`
- System allocator (see "Allocator" below)
- `--release`

**Laravel Octane (Swoole)**
- `--workers=auto` — Octane's documented default is one request worker
  per CPU core (`reference/docs-13.x/octane.md:371`), so 32 here
- `--task-workers=4` — bounded deliberately: `Octane::concurrently()`
  dispatches to task workers as separate processes, and Tier 2.1 fires
  five concurrent queries per request, so an unbounded pool would
  explode under load
- `config:cache`, `route:cache`, `view:cache`, `APP_DEBUG=false`,
  `composer install --no-dev --optimize-autoloader`, OPcache on,
  Tracing JIT on

A `--workers=12` variant is recorded as a labelled secondary run. It is
not the headline. Handicapping Octane below its own default is the
fastest way to get a published result dismissed, and it is the same
error as capping the SUT, aimed at the other stack.

**The process asymmetry is the story before any traffic runs.** Suprnova:
one process, 32 worker threads, shared memory, no IPC. Octane: 36
processes, 36x memory duplication, the OS scheduler arbitrating between
them.

### Allocator

Suprnova runs the **system allocator** — what ships today. jemalloc is
not added for the benchmark. Swapping allocators changes every number in
the suite, so measuring an allocator the framework does not ship would
measure something nobody runs. All memory metrics come from `/proc`,
which needs no dependency. Allocator comparison, if wanted, is its own
experiment.

### Baseline idle snapshot

After boot and warmup, before any timed measurement: RSS per process
(Suprnova: one figure; Octane: the sum across all workers), total VSZ,
thread count, FD count, open pool connections.

This is the cost of existing. Report it beside every throughput result —
"X rps at Y MB" is a complete claim; "X rps" is not.

---

## 0 — Warmup, steady state, and metrics

### 0.1 Warmup

Both stacks have cold-start costs. Different costs, same discipline.

*Suprnova:* pool fill (SeaORM's pool is lazy), allocator arena
population, OS page table population, branch predictor and i-cache
training.

*Laravel:* OPcache bytecode compilation, Tracing JIT (observes hot
traces over ~100 invocations, stabilises around request 200-500), Swoole
coroutine pool, pool fill.

1. **Warm every route in the mix** at least once — pool connections,
   OPcache entries, lazy-init paths.
2. **60-second warmup at c=32** against the full mix. Laravel needs it
   for JIT; Suprnova gets the identical duration for fairness.
3. **Verify steady state.** A 30-second canary at the target rate,
   recording per-second p99. If p99 over the first 5s exceeds 2x the
   p99 over the last 5s, warmup was insufficient — extend and repeat.
   Warmup is verified, not assumed.
4. **Record RSS before and after warmup.** The delta is cold-start
   allocation cost, and it is a real operational number.

### 0.2 System metrics

Sampled every 5s during every timed run, CSV, one row per sample:

```
ts_unix,
rss_kb, vsz_kb, minor_faults, major_faults,
cpu_user_pct, cpu_sys_pct,
voluntary_ctx_switches, involuntary_ctx_switches,
num_fds, num_threads, tcp_established, tcp_time_wait,
db_pool_active, db_pool_idle, db_pool_wait_count
```

| Metric | Source |
|---|---|
| RSS / VSZ / threads / ctx switches | `/proc/<pid>/status` |
| Minor / major faults, CPU user / sys | `/proc/<pid>/stat` |
| FD count | `/proc/<pid>/fd` |
| TCP states | `ss -tan state established`, `... time-wait` |
| DB pool | `/debug/pool-stats` (bench feature) |

Octane's figures are summed across all worker processes; the collector
resolves the worker set each sample rather than caching PIDs, because
`--max-requests` recycles workers mid-run.

**Per-core utilisation** via `mpstat -P ALL 5` to `per-core.csv`. This
answers "is work spread across cores or is one pegged" — the async
scaling story, told by observation rather than by constraining cores.

**`perf stat`** once per phase at steady state, not continuously:

```bash
perf stat -e instructions,cycles,cache-misses,cache-references,\
context-switches,cpu-migrations,page-faults -p <pid> -- sleep 10
```

Gives IPC, cache miss rate, and context-switch rate alongside throughput.

### 0.3 Mutation accounting

Every run declares which tables it mutates. Row counts are asserted
before and after; a read-only workload that moved a row count fails the
run.

This is not bureaucracy. An earlier throughput run wrote a session row
per request — 31 million rows and 8 GB — which meant the two application
tiers were measuring a write nobody intended, against a table that grew
under the measurement, so step 1 and step 10 ran against different
databases. At one hour per run the same mistake is 200x larger.

Mutating scenarios truncate and reseed between runs.

---

## Tier 1 — Real traffic

The headline numbers, over a weighted mix resembling social traffic.

### Route work required first

Three routes in the dogfood app are demo stubs that do not do what a
benchmark needs. They are rewritten before Tier 1 runs:

| Route | Today | Becomes |
|---|---|---|
| `GET /users` | two hardcoded users in a `json_response!` | paginated read of the seeded `users` table, Inertia response |
| `GET /users/{id}` | `format!("User {}", id)` | real fetch by PK with `profile` eager-loaded, Inertia response |
| `GET /api/users` | 100-row in-memory `Vec`, 200 `format!` calls/request | cursor pagination over the seeded table |

The static-asset entry is dropped — the dogfood app has no static-file
route, and inventing one to benchmark it would measure a route no user
of this framework has.

### Mix

| Weight | Route | Exercises |
|---:|---|---|
| 20% | `GET /` | Inertia page, no DB. Framework + middleware + render |
| 15% | `GET /api/posts` | DB read + JSON. `filter("is_public", true).order_by_asc("id")` |
| 15% | `GET /api/users` | cursor pagination over 10k rows |
| 10% | `GET /api/posts/{id}` | PK fetch + Gate authorization + JSON |
| 10% | `GET /users/{id}` | PK fetch + eager-loaded profile, Inertia |
| 10% | `POST /api/posts` | session + auth + validate + INSERT + 201 |
| 10% | `GET /users` | paginated collection, Inertia |
| 5% | `POST /api/ping` | rate-limit middleware |
| 5% | `GET /does-not-exist` | 404 path |

Inertia routes carry `X-Inertia: true` and `X-Inertia-Version`, so both
stacks return JSON prop payloads rather than HTML documents and the
comparison is serialization against serialization.

`POST /api/posts` needs a session cookie. Warmup logs in once, captures
the cookie, and injects it into the targets file. **Login is not part of
the measurement** — password hashing would otherwise dominate and the
benchmark would measure argon2 against bcrypt.

### Procedure

1. **Per-route capacity** (oha, closed-loop). Concurrency sweep
   `c ∈ {1,2,4,8,16,32,64,128,256,512}`, 30s per step, one route at a
   time. oha takes a single URL, so it cannot drive the mix — but
   per-route knees are publishable on their own and feed Tier 2's pool
   math.

2. **Mix knee** (vegeta, open-loop). Rate ramp against a targets file
   holding the whole mix, weights expressed by repeating lines. In open
   loop the knee has an unambiguous definition: the rate at which
   achieved_rps decouples from target_rps. No judgment call about where
   a curve bends.

3. **Published latency** (vegeta, open-loop). One hour each at 50%, 70%,
   and 90% of the mix knee, after verified warmup. Report p50 / p95 /
   p99 / p99.9 and success rate.

4. **Past saturation.** 110% and 130% of the knee: graceful degradation
   (rising latency, everything eventually served), bounded queueing
   (some timeouts), or collapse (connection refused, 502, OOM). This is
   maturity, not speed, and it is the number that predicts a bad night.

5. **Parallelism scaling.** Not via cpuset. Each stack exposes its own
   operator-facing knob — Tokio worker threads, Octane `--workers` — and
   those are varied with the machine fully uncapped. That is
   configuration, which every operator tunes, rather than a constraint
   on the host.

Closed-loop is legitimate for step 1: coordinated omission distorts
latency samples, not the completion rate. Every published percentile
comes from the open-loop passes.

### Pass criteria

- Correct status codes and response shapes throughout
- No OOM, no connection-refused below the knee
- Percentiles from vegeta only
- Row-count deltas match the declared mutation set

---

## Tier 2 — Concurrency behaviour

Three situations where the two concurrency models are structurally
different: several independent I/O operations inside one request,
connections scarcer than in-flight requests, and a slow downstream.

Whether that structural difference produces a measurable gap is the
question, not the premise. A tier named "async advantage" that then
reports an advantage has proven nothing.

### 2.1 Concurrent database queries

**Suprnova** — `/bench/dashboard`, five independent queries via
`tokio::try_join!`:

```rust
let (user, posts, comments, tags, roles) = tokio::try_join!(
    User::find(user_id),
    Post::for_author(user_id),
    Comment::query().filter("commentable_id", user_id).get(),
    Tag::all(),
    User::query().find(user_id)?.roles().get(),
)?;
```

`for_author` and `all_public` already exist on the model.

**Laravel** — two arms:

- Sequential Eloquent. What every Laravel app actually ships.
- `Octane::concurrently([...])`. Swoole task workers, a separate
  process per task, capped at 1024 tasks.

Not `Concurrency::run()`: its default `process` driver spawns a PHP CLI
process per closure, and its faster `fork` driver **cannot be used in
web requests at all** — PHP does not support forking there
(`reference/docs-13.x/concurrency.md`). And not `DB::parallelize()`,
which does not exist.

**Measurement.** Wall clock at c=1, then throughput and p99 at c=32.

**Pool interaction.** Each in-flight dashboard holds five connections,
not one. The pool size is stated per scenario, and Tier 2.2's small
pools are read with that in mind — at `DB_MAX_CONNECTIONS=10`, two
concurrent dashboards exhaust it.

### 2.2 Connection pool contention

Pool size ∈ {5, 10, 20, 50, 100}, concurrency fixed at 128, both stacks.

The question: when connections are scarcer than in-flight requests, does
throughput fall off a cliff or bend? Suprnova's tasks yield at `.await`
while waiting for a connection, so the OS thread is free; whether
Octane's workers block at the process level as well as the pool level
depends on how Swoole's hooks handle the driver, which this measures
rather than assumes.

Plot throughput against pool size. No predicted shape — if Octane bends
as gracefully as Suprnova, that is the finding.

### 2.3 Slow downstream

`/bench/external?delay=N` — an HTTP call to a local echo server held at
0 / 50 / 200 / 500 ms. A payment gateway, a mail API, an LLM endpoint.

With 32 workers and a 200 ms downstream, a strictly blocking
worker-per-request model ceilings at `32 / 0.2 = 160 rps` regardless of
framework speed. Whether Octane actually hits that depends on Swoole's
runtime hooks being enabled and on the HTTP client yielding — so the
hook configuration is recorded and the test is written to find out
rather than to assume. In Tokio the wait is an idle future and the
thread serves other requests.

Throughput against delay, at c=256.

---

## Tier 3 — ORM stress

Suprnova ports Eloquent's patterns to Rust. This tier stresses where
Eloquent is known to hurt. It is the justification for the port.

### Seed data

One shared, deterministic seeder (seeded RNG) populates both databases
identically. Truncate and vacuum first.

| Table | Rows |
|---|---:|
| `users` | 10,000 |
| `posts` | 50,000 |
| `comments` | 200,000 |
| `taggables` | 150,000 |
| `profiles` | 10,000 |
| `role_user` | 10,000 |
| `tags` | 100 |
| `roles` | 5 |

Post-seed verification: row counts match; `EXPLAIN ANALYZE` on every
bench query on both sides confirms equivalent index usage; one response
per route dumped from each stack and diffed field by field.

### 3.1 Large result hydration

`GET /bench/users/all` — 10,000 users hydrated into models and
serialized. Stresses per-row allocation cost in both ORMs at a volume
where any per-row overhead is visible.

Response time, RSS delta during the request. The per-row cost is a
measurement, not an assumption — neither side's allocation strategy is
asserted in advance.

### 3.2 Deep eager loading

`GET /bench/posts/{id}/deep`:

```rust
Post::query()
    .with("user")            // BelongsTo
    .with("user.profile")    // HasOne, nested
    .with("user.roles")      // BelongsToMany, nested
    .with("comments")        // MorphMany
    .with("tags")            // MorphToMany
    .find(id)
```

All five relations exist on the models today.

A correct eager loader fires 5-6 queries regardless of result size; an
incorrect one fires N+1. **Query count is measured, not assumed**, and
reported beside response time.

Variants: single post, and an author page loading a user's posts with
the same relation tree.

### 3.3 Paginated feed

`GET /bench/posts?page=50&per_page=20&with=user,tags` — depth pagination
plus eager loading on the result page. This is the social feed query.

Response time, query count, body size.

### 3.4 Concurrent writes

`POST /bench/posts/bulk` — 10 posts inserted in one transaction. 50
concurrent clients = 500 concurrent inserts, sustained.

Throughput, p99, transaction failure rate, deadlock count. Truncate and
reseed after.

---

## Tier 4 — Real-time

### Comparison note

Laravel does not serve WebSockets in-process. The honest comparison is
Suprnova (one process: HTTP + WS + SSE) against Octane + Reverb (two
processes). Both get the whole machine. Report the shape plainly rather
than pretending it is like for like.

### 4.1 Connection ceiling

Custom Rust client (`tokio-tungstenite`) opens connections at 100/sec
until the server refuses or latency degrades. No memory cap — the
framework uses what it uses and the cost is reported.

- Maximum stable connections
- **RSS per connection** after subtracting baseline — the transferable
  number, which a reader multiplies by their own instance size
- Ping/pong RTT at 1k / 5k / 10k / 20k
- CPU while holding N idle connections (should be near zero)

### 4.2 Broadcast fan-out

1k / 5k / 10k subscribers on one channel, a publisher emitting a
timestamped event every 100 ms for 60s.

Publish-to-last-subscriber time, p50 / p99 per-subscriber delivery,
dropped messages from lagging subscribers.

### 4.3 Message throughput

1,000 connections each sending every 100 ms = 10k inbound/sec, each
broadcast to all subscribers. Sustained inbound rate before drops,
delivery rate, p99 delivery latency, CPU and RSS.

### 4.4 Mixed HTTP + WebSocket

5,000 WS connections held open receiving a broadcast per second, while
the Tier 1 mix runs at 70% of its knee.

- HTTP percentiles with vs without WS load (real-time cost to HTTP)
- WS delivery latency with vs without HTTP load (HTTP bleed into
  real-time)
- RSS, CPU, context switches

Proves whether one shared runtime is an advantage (no IPC, shared
scheduler) or a liability (contention between the two paths).

### 4.5 SSE ceiling and reconnection storm

Ceiling as in 4.1. Then, with 5,000 SSE connections open, sever all of
them at once — a proxy restart. Time until all reconnect and resume via
`Last-Event-ID`, CPU spike during the storm, and whether any connection
is refused.

---

## Tier 5 — Media serving

Images, read path only. Uploads stay out of scope.

A social app is mostly image bytes by volume, and serving them has a
different profile from everything above: large response bodies, slow
clients, and a memory story that depends entirely on whether the
framework streams or buffers. `HttpResponse::stream_bytes`
(`framework/src/http/response.rs:133`) means both paths exist and can be
compared rather than assumed.

### The Laravel framing, stated up front

**Most Laravel deployments never serve images through PHP.** nginx
`X-Accel-Redirect`, `X-Sendfile`, or a CDN handles it. Benchmarking
Octane serving image bytes measures a configuration almost nobody ships,
and reporting a win there would be dishonest.

So the question is not "whose number is bigger." It is **"does this
architecture let you delete a component?"** If Suprnova serves media at
production throughput with flat memory, that is one fewer tier in the
stack — a real operational claim. If it does not, the offload is
required for both and that is equally worth knowing.

Laravel is therefore reported twice: served through Octane (the
framework-to-framework figure) and with nginx offload (what people
actually run). Both labelled.

### Fixtures

Stored images at three sizes, seeded alongside the database: 20 KB
(thumbnail), 200 KB (standard), 2 MB (full-resolution). Identical files
on both sides.

### 5.1 Throughput by size — diagnostic only, not publishable

Serve each size at increasing concurrency, reporting requests/sec and
bytes/sec.

**These figures do not describe real-world image serving and must not be
quoted as if they did.** Three fixture files hit repeatedly stay
resident in page cache, so no disk is involved; the generator shares the
host, so no NIC is involved and loopback is another memcpy. What this
measures is the framework's copy path — how many times the bytes are
moved between page cache, userspace, the response body, and the socket —
plus syscall count per response.

That is a useful diagnostic: a framework that copies twice will show it
here, and the delta against 5.2's streaming handler localises where.
It is not a serving rate. Quoting a loopback bytes/sec as throughput
would repeat this benchmark's earlier failure of measuring the harness
and reporting it as the server.

A genuine I/O test needs a fixture set larger than RAM and a generator
on a second host. Both are real changes; neither is in scope here.

### 5.2 Memory under concurrent serving

The architectural tell. N concurrent requests for 2 MB images against
both the buffered handler and the `stream_bytes` handler.

If RSS scales with `concurrency × file size`, the path buffers. If it
stays flat, it streams. Run both handlers so the difference is
attributable to the response type rather than inferred.

### 5.3 Conditional requests

Real image traffic is dominated by revalidation, not first fetches. A
304 should cost almost nothing — unless the framework reads the file
before deciding it does not need to send it.

Throughput of `If-None-Match` / `If-Modified-Since` hits against
unconditional 200s. The ratio is a large real-world lever and a cheap
one to get wrong.

### 5.4 Slow clients

The architectural scenario with real bytes, and the most common
production form of it. N clients each fetching a 2 MB image at
throttled bandwidth (100 KB/s ≈ 20s per transfer) — every mobile
network, all day.

A strictly worker-per-request model pins a worker for the full 20
seconds. An async runtime holds a future. Whether Swoole's hooks change
that is measured, not assumed.

Report: concurrent slow transfers sustained, memory held per transfer,
and whether normal traffic is affected while they run.

### 5.5 Storage driver

Local filesystem versus S3 for the same image. S3-backed serving is an
upstream fetch per request — Tier 2.3's slow-downstream shape, with a
payload attached. Run only if S3 credentials are present; otherwise
local only, and say so rather than omitting the row.

---

## Tier 6 — Soak

**Two hours.** The floor is set by sqlx's 30-minute `max_lifetime`: a
soak has to span several connection-recycle cycles or it never observes
the most likely source of a periodic latency artifact. Two hours gives
four cycles and clears allocator settling with room to spare.

One 8-hour run before publishing, once. Two hours cannot separate a leak
below ~1 MB/h from sampling noise; eight can. It does not need repeating
per change.

### Load

- Tier 1 mix at 60% of the knee, **reads only**
- Authenticated session flows at 5% of volume (login, page, write,
  logout) via `drill`, whose cookie jar and YAML flows fit this exactly
- 500 WS connections, broadcast every 5s
- 200 SSE connections, event every 10s

Writes are excluded from the soak mix, and the session flow's writes are
bounded by a delete cycle. At 10k rps a 10% write share would insert 7.2
million posts in two hours — 144x the seeded table — shifting query
plans mid-run and presenting as a fake regression. Leak detection
requires a stationary database.

### Metrics

§0.2 every 30s, plus a latency percentile snapshot per interval (never
averaged across the run — drift has to stay visible) and WS/SSE delivery
latency per interval.

### Pass criteria

- RSS flat after warmup. A logarithmic bump in the first 30 minutes is
  expected (cache warming, arena fill); linear growth over hours is a
  leak
- FD count stable — any monotonic climb is a leak
- p99 not drifting upward
- WS/SSE delivery not degrading
- No OOM
- Zero dropped WS messages

### Octane runs

| Run | `--max-requests` | Question |
|---|---|---|
| A | 500 (default) | What a real deployment does. Memory looks flat because workers recycle before a leak can show |
| B | disabled | The honest architecture comparison — Suprnova has no equivalent escape hatch |

Both reported. A is operational, B is architectural.

---

## Bench routes

Tiers 2-3 need routes the dogfood app does not have. They live under
`/bench` behind `#[cfg(feature = "bench")]` so they cannot compile into
a production binary.

```
GET  /bench/dashboard          Tier 2.1  concurrent queries
GET  /bench/external?delay=N   Tier 2.3  slow downstream
GET  /bench/users/all          Tier 3.1  large result set
GET  /bench/posts/{id}/deep    Tier 3.2  deep eager load
GET  /bench/posts/paginated    Tier 3.3  paginated + relations
POST /bench/posts/bulk         Tier 3.4  concurrent writes
GET  /bench/ws-echo            Tier 4.3  WS echo
GET  /bench/sse-feed           Tier 4.5  SSE feed
GET  /debug/pool-stats         §0.2      pool gauges
```

The Laravel app implements the same routes with the same queries and
response shapes. Payload parity is diffed field by field before any
timing run.

---

## Parity contract

The comparison is void unless all hold, and each is asserted rather than
assumed:

1. **Same database** — Postgres 18, same host, same schema, same rows,
   same indexes. Row counts asserted.
2. **Same queries** — where Suprnova eager-loads, Eloquent uses
   `with()`. Query logs captured on both and query counts diffed per
   route.
3. **Same payload** — equivalent Inertia JSON prop structures, diffed.
4. **Same host, same conditions** — nothing capped on either side, same
   network mode, never run concurrently.
5. **Both production-built** — Suprnova `--release`; Laravel
   `config:cache`, `route:cache`, `view:cache`, `--no-dev
   --optimize-autoloader`, `APP_DEBUG=false`, OPcache and JIT on.
6. **Equal warmup** — 60 seconds both sides, steady state verified.
7. **Login excluded from measurement** on both sides, with hash work
   factors recorded.

---

## Tooling

| Tool | Purpose |
|---|---|
| `ab` | smoke checks while developing bench routes |
| `oha` | closed-loop per-route capacity |
| `vegeta` | open-loop rate ramp and every published percentile |
| `drill` | authenticated multi-step session flows in the soak |
| `bench/tools/ws-flood` | WS ceiling, fan-out, message throughput |
| `bench/tools/sse-flood` | SSE ceiling, reconnection storm |
| `mpstat` | per-core utilisation |
| `perf` | IPC, cache misses, context switches |
| seeder | deterministic data on both sides |
| `collect.sh` | metrics sampler |
| `env-record.sh` | environment stanza |

Result parsers have no exception handling, deliberately. A benchmark
that cannot read its own output must stop, not substitute a number — an
earlier parser returned zeros on a parse failure and printed ten rows of
`0 rps` for a server serving 346,000.

---

## Reporting

Per tier: setup, method, raw data path, results, interpretation.
Headline figures are the median of >= 3 runs with the spread shown.

Required in every report: the environment stanza, warmup verification,
the generator CPU check, mutation assertions, and the full metrics CSV.

Graphs: throughput vs concurrency; throughput vs parallelism setting;
latency at 50/70/90% of knee; throughput vs pool size; throughput vs
downstream delay; RSS over time; p99 over time; WS connections vs RSS;
fan-out latency vs subscriber count.

---

## Execution order

1. Seed and verify parity
2. **Tier 1** — establishes the knee every other tier references
3. **Tier 2** — async advantage
4. **Tier 3** — ORM stress
5. **Tier 4** — real-time
6. **Tier 5** — media serving
7. **Tier 6** — soak, last, because it holds the box

Suprnova first, then Laravel, identical conditions, never interleaved.

---

## Not benchmarked

WebSocket message ordering (correctness, not performance) · queue
throughput (Phase 1 covered correctness) · **file upload** (media is
benchmarked on the read path only, per Tier 5) · image processing and
resizing · JS SSR · cold start · compile time.

Real concerns, but not framework-against-framework performance
questions. Each would be its own experiment.
