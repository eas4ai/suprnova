# Feature map: `manual/queues.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 259 checked.

## Command line

### App runner (`suprnova::Application`; the project binary)

- [ ] command `app queue:work` · framework/src/app/mod.rs:166
  - Run the queue worker daemon (drains the configured queue driver)
- [ ] command `app queue:pause` · framework/src/app/mod.rs:186
  - Pause job processing for a queue (or every queue with `--all`). Mirrors `php artisan queue:pause`
  - argument `[QUEUE]`: Queue to pause. Required unless `--all` is given
- [ ] command `app queue:resume` · framework/src/app/mod.rs:197
  - Resume job processing for a paused queue (or every queue with `--all`). Mirrors `php artisan queue:resume` (alias `queue:continue`)
  - argument `[QUEUE]`: Queue to resume. Required unless `--all` is given

## Endpoints and tables

### operator-managed; default name of `QUEUE_DB_TABLE` for `QUEUE_DRIVER=database`

- [ ] table `jobs (operator)` · framework/src/queue/mod.rs:1355

### operator-managed; schema documented on `DatabaseBatchRepository`

- [ ] table `job_batches (operator)` · framework/src/queue/batch.rs:410
- [ ] table `job_batch_settlements (operator)` · framework/src/queue/batch.rs:420

### operator-managed; schema documented on `DatabaseFailedJobStore`

- [ ] table `failed_jobs (operator)` · framework/src/queue/failed.rs:241

## Rust API: suprnova

### `suprnova::queue::batch`

- [ ] fn `suprnova::queue::batch::current_repository` · framework/src/queue/batch.rs:1172
- [ ] fn `suprnova::queue::batch::install_repository` · framework/src/queue/batch.rs:1163
- [ ] fn `suprnova::queue::batch::register_callback` · framework/src/queue/batch.rs:1144
- [ ] struct `suprnova::Batch` · framework/src/queue/batch.rs:32 (also `suprnova::queue::Batch`, `suprnova::queue::batch::Batch`)
  - Public fields: `id`, `name`, `total_jobs`, `pending_jobs`, `failed_jobs`, `failed_job_ids`, `options`, `created_at`, `cancelled_at`, `finished_at`
  - [ ] fn `suprnova::Batch::finished` · framework/src/queue/batch.rs:57
  - [ ] fn `suprnova::Batch::cancelled` · framework/src/queue/batch.rs:62
  - [ ] fn `suprnova::Batch::processed_jobs` · framework/src/queue/batch.rs:68
  - [ ] fn `suprnova::Batch::progress` · framework/src/queue/batch.rs:73
- [ ] struct `suprnova::BatchOptions` · framework/src/queue/batch.rs:84 (also `suprnova::queue::BatchOptions`, `suprnova::queue::batch::BatchOptions`)
  - Public fields: `then_callbacks`, `catch_callbacks`, `finally_callbacks`, `allow_failures`
- [ ] struct `suprnova::DatabaseBatchRepository` · framework/src/queue/batch.rs:456 (also `suprnova::queue::DatabaseBatchRepository`, `suprnova::queue::batch::DatabaseBatchRepository`)
  - Implements: `suprnova::BatchRepository`
  - [ ] fn `suprnova::DatabaseBatchRepository::new` · framework/src/queue/batch.rs:474
  - [ ] fn `suprnova::DatabaseBatchRepository::with_tables` · framework/src/queue/batch.rs:493
- [ ] struct `suprnova::MemoryBatchRepository` · framework/src/queue/batch.rs:234 (also `suprnova::queue::MemoryBatchRepository`, `suprnova::queue::batch::MemoryBatchRepository`)
  - Implements: `suprnova::BatchRepository`
  - [ ] fn `suprnova::MemoryBatchRepository::new` · framework/src/queue/batch.rs:240
- [ ] struct `suprnova::PendingBatch` · framework/src/queue/batch.rs:1211 (also `suprnova::queue::PendingBatch`, `suprnova::queue::batch::PendingBatch`)
  - Public fields: `name`, `options`
  - [ ] fn `suprnova::PendingBatch::new` · framework/src/queue/batch.rs:1235
  - [ ] fn `suprnova::PendingBatch::name` · framework/src/queue/batch.rs:1246
  - [ ] fn `suprnova::PendingBatch::add` · framework/src/queue/batch.rs:1254
  - [ ] fn `suprnova::PendingBatch::then` · framework/src/queue/batch.rs:1278
  - [ ] fn `suprnova::PendingBatch::catch` · framework/src/queue/batch.rs:1284
  - [ ] fn `suprnova::PendingBatch::finally` · framework/src/queue/batch.rs:1291
  - [ ] fn `suprnova::PendingBatch::allow_failures` · framework/src/queue/batch.rs:1298
  - [ ] fn `suprnova::PendingBatch::len` · framework/src/queue/batch.rs:1304
  - [ ] fn `suprnova::PendingBatch::is_empty` · framework/src/queue/batch.rs:1309
  - [ ] fn `suprnova::PendingBatch::dispatch` · framework/src/queue/batch.rs:1345
- [ ] struct `suprnova::TerminalCallbackClaim` · framework/src/queue/batch.rs:114 (also `suprnova::queue::TerminalCallbackClaim`, `suprnova::queue::batch::TerminalCallbackClaim`)
  - Public fields: `finished_at`, `cancelled_at`
- [ ] struct `suprnova::UpdatedBatchJobCounts` · framework/src/queue/batch.rs:101 (also `suprnova::queue::UpdatedBatchJobCounts`, `suprnova::queue::batch::UpdatedBatchJobCounts`)
  - Public fields: `pending_jobs`, `failed_jobs`
- [ ] trait `suprnova::BatchCallback` · framework/src/queue/batch.rs:1119 (also `suprnova::queue::BatchCallback`, `suprnova::queue::batch::BatchCallback`)
  - [ ] fn `suprnova::BatchCallback::name` · framework/src/queue/batch.rs:1121 (required)
  - [ ] fn `suprnova::BatchCallback::handle` · framework/src/queue/batch.rs:1133 (required)
- [ ] trait `suprnova::BatchRepository` · framework/src/queue/batch.rs:125 (also `suprnova::queue::BatchRepository`, `suprnova::queue::batch::BatchRepository`)
  - Implemented here by: `DatabaseBatchRepository`, `MemoryBatchRepository`
  - [ ] fn `suprnova::BatchRepository::store` · framework/src/queue/batch.rs:132 (required)
  - [ ] fn `suprnova::BatchRepository::find` · framework/src/queue/batch.rs:134 (required)
  - [ ] fn `suprnova::BatchRepository::increment_total_jobs` · framework/src/queue/batch.rs:140 (required)
  - [ ] fn `suprnova::BatchRepository::record_successful_job` · framework/src/queue/batch.rs:155 (required)
  - [ ] fn `suprnova::BatchRepository::record_failed_job` · framework/src/queue/batch.rs:168 (required)
  - [ ] fn `suprnova::BatchRepository::cancel` · framework/src/queue/batch.rs:175 (required)
  - [ ] fn `suprnova::BatchRepository::is_cancelled` · framework/src/queue/batch.rs:177 (required)
  - [ ] fn `suprnova::BatchRepository::mark_finished` · framework/src/queue/batch.rs:184 (required)
  - [ ] fn `suprnova::BatchRepository::claim_terminal_callbacks` · framework/src/queue/batch.rs:199 (provided)
  - [ ] fn `suprnova::BatchRepository::delete` · framework/src/queue/batch.rs:209 (required)
- [ ] const `suprnova::DEFAULT_BATCH_SETTLEMENTS_TABLE` · framework/src/queue/batch.rs:402 (also `suprnova::queue::DEFAULT_BATCH_SETTLEMENTS_TABLE`, `suprnova::queue::batch::DEFAULT_BATCH_SETTLEMENTS_TABLE`)
- [ ] const `suprnova::DEFAULT_BATCHES_TABLE` · framework/src/queue/batch.rs:400 (also `suprnova::queue::DEFAULT_BATCHES_TABLE`, `suprnova::queue::batch::DEFAULT_BATCHES_TABLE`)

### `suprnova::queue::chain`

- [ ] fn `suprnova::queue::chain::next_link_id` · framework/src/queue/chain.rs:166
- [ ] struct `suprnova::ChainLink` · framework/src/queue/chain.rs:22 (also `suprnova::queue::ChainLink`, `suprnova::queue::chain::ChainLink`)
  - Public fields: `job_name`, `payload`, `max_tries`, `timeout_secs`, `fail_on_timeout`, `backoff`, `queue`
  - [ ] fn `suprnova::ChainLink::from_job` · framework/src/queue/chain.rs:52
  - [ ] fn `suprnova::ChainLink::to_envelope` · framework/src/queue/chain.rs:70
  - [ ] fn `suprnova::ChainLink::to_envelope_after` · framework/src/queue/chain.rs:108
- [ ] struct `suprnova::PendingChain` · framework/src/queue/chain.rs:173 (also `suprnova::queue::PendingChain`, `suprnova::queue::chain::PendingChain`)
  - [ ] fn `suprnova::PendingChain::new` · framework/src/queue/chain.rs:185
  - [ ] fn `suprnova::PendingChain::add` · framework/src/queue/chain.rs:191
  - [ ] fn `suprnova::PendingChain::len` · framework/src/queue/chain.rs:207
  - [ ] fn `suprnova::PendingChain::is_empty` · framework/src/queue/chain.rs:211
  - [ ] fn `suprnova::PendingChain::dispatch` · framework/src/queue/chain.rs:217

### `suprnova::queue::database`

- [ ] struct `suprnova::DatabaseQueueDriver` · framework/src/queue/database.rs:27 (also `suprnova::queue::DatabaseQueueDriver`, `suprnova::queue::database::DatabaseQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::DatabaseQueueDriver::new` · framework/src/queue/database.rs:46

### `suprnova::queue::debounce`

- [ ] struct `suprnova::Debounced` · framework/src/queue/debounce.rs:29 (also `suprnova::queue::Debounced`, `suprnova::queue::debounce::Debounced`)
  - Public fields: `owner`, `max_wait_exceeded`
- [ ] struct `suprnova::DebounceOptions` · framework/src/queue/debounce.rs:53 (also `suprnova::queue::DebounceOptions`, `suprnova::queue::debounce::DebounceOptions`)
  - Public fields: `window`, `max_wait`, `id`
  - [ ] fn `suprnova::DebounceOptions::new` · framework/src/queue/debounce.rs:65
  - [ ] fn `suprnova::DebounceOptions::max_wait` · framework/src/queue/debounce.rs:74
  - [ ] fn `suprnova::DebounceOptions::id` · framework/src/queue/debounce.rs:81

### `suprnova::queue::driver`

- [ ] struct `suprnova::Reservation` · framework/src/queue/driver.rs:42 (also `suprnova::queue::Reservation`, `suprnova::queue::driver::Reservation`)
  - Public fields: `envelope`, `token`
- [ ] struct `suprnova::ReservationToken` · framework/src/queue/driver.rs:14 (also `suprnova::queue::ReservationToken`, `suprnova::queue::driver::ReservationToken`)
  - Public tuple fields: 1
- [ ] enum `suprnova::QueueFilterCapability` · framework/src/queue/driver.rs:59 (also `suprnova::queue::QueueFilterCapability`, `suprnova::queue::driver::QueueFilterCapability`)
  - Variants: `Supported`, `Unsupported`, `Unknown`
- [ ] enum `suprnova::Settled` · framework/src/queue/driver.rs:21 (also `suprnova::queue::Settled`, `suprnova::queue::driver::Settled`)
  - Variants: `Atomically`, `Stale`, `Unsupported`
- [ ] trait `suprnova::QueueDriver` · framework/src/queue/driver.rs:72 (also `suprnova::queue::QueueDriver`, `suprnova::queue::driver::QueueDriver`)
  - Implemented here by: `DatabaseQueueDriver`, `FailoverQueueDriver`, `MemoryQueueDriver`, `NullQueueDriver`, `RedisQueueDriver`, `SyncQueueDriver`
  - [ ] fn `suprnova::QueueDriver::push` · framework/src/queue/driver.rs:74 (required)
  - [ ] fn `suprnova::QueueDriver::pop` · framework/src/queue/driver.rs:79 (required)
  - [ ] fn `suprnova::QueueDriver::queue_filter_capability` · framework/src/queue/driver.rs:89 (provided)
  - [ ] fn `suprnova::QueueDriver::reservation_deadline` · framework/src/queue/driver.rs:107 (provided)
  - [ ] fn `suprnova::QueueDriver::pop_from` · framework/src/queue/driver.rs:131 (provided)
  - [ ] fn `suprnova::QueueDriver::ack` · framework/src/queue/driver.rs:150 (required)
  - [ ] fn `suprnova::QueueDriver::nack` · framework/src/queue/driver.rs:162 (required)
  - [ ] fn `suprnova::QueueDriver::release` · framework/src/queue/driver.rs:197 (provided)
  - [ ] fn `suprnova::QueueDriver::settle` · framework/src/queue/driver.rs:251 (provided)
  - [ ] fn `suprnova::QueueDriver::size` · framework/src/queue/driver.rs:266 (provided)
  - [ ] fn `suprnova::QueueDriver::pending_size` · framework/src/queue/driver.rs:276 (provided)
  - [ ] fn `suprnova::QueueDriver::delayed_size` · framework/src/queue/driver.rs:285 (provided)
  - [ ] fn `suprnova::QueueDriver::reserved_size` · framework/src/queue/driver.rs:291 (provided)
  - [ ] fn `suprnova::QueueDriver::pending_jobs` · framework/src/queue/driver.rs:311 (provided)
  - [ ] fn `suprnova::QueueDriver::delayed_jobs` · framework/src/queue/driver.rs:323 (provided)
  - [ ] fn `suprnova::QueueDriver::reserved_jobs` · framework/src/queue/driver.rs:336 (provided)
  - [ ] fn `suprnova::QueueDriver::clear` · framework/src/queue/driver.rs:349 (provided)
  - [ ] fn `suprnova::QueueDriver::bulk_push` · framework/src/queue/driver.rs:359 (provided)
  - [ ] fn `suprnova::QueueDriver::name` · framework/src/queue/driver.rs:367 (provided)

### `suprnova::queue::envelope`

- [ ] fn `suprnova::queue::envelope::queue_matches` · framework/src/queue/envelope.rs:28
- [ ] struct `suprnova::Envelope` · framework/src/queue/envelope.rs:55 (also `suprnova::queue::Envelope`, `suprnova::queue::envelope::Envelope`)
  - Public fields: `schema_version`, `id`, `job_name`, `queue`, `payload`, `dispatched_at`, `available_at`, `attempts`, `max_tries`, `backoff`, `timeout_secs`, `fail_on_timeout`, `idempotency_key`, `unique_lock_owner`, `debounce_id`, `debounce_owner`, `batch_id`, `chain_remaining`
  - [ ] fn `suprnova::Envelope::from_json` · framework/src/queue/envelope.rs:179
  - [ ] fn `suprnova::Envelope::to_json` · framework/src/queue/envelope.rs:188
- [ ] enum `suprnova::EnvelopeError` · framework/src/queue/envelope.rs:166 (also `suprnova::queue::EnvelopeError`, `suprnova::queue::envelope::EnvelopeError`)
  - Variants: `UnsupportedSchemaVersion`, `Decode`
- [ ] const `suprnova::queue::CURRENT_SCHEMA_VERSION` · framework/src/queue/envelope.rs:13 (also `suprnova::queue::envelope::CURRENT_SCHEMA_VERSION`)
- [ ] const `suprnova::queue::envelope::DEFAULT_QUEUE` · framework/src/queue/envelope.rs:20

### `suprnova::queue::errors`

- [ ] struct `suprnova::ManuallyFailed` · framework/src/queue/errors.rs:42 (also `suprnova::queue::ManuallyFailed`, `suprnova::queue::errors::ManuallyFailed`)
  - Public fields: `job_name`, `reason`
- [ ] struct `suprnova::MaxAttemptsExceeded` · framework/src/queue/errors.rs:17 (also `suprnova::queue::MaxAttemptsExceeded`, `suprnova::queue::errors::MaxAttemptsExceeded`)
  - Public fields: `job_name`, `attempts`, `reason`
- [ ] struct `suprnova::TimeoutExceeded` · framework/src/queue/errors.rs:30 (also `suprnova::queue::TimeoutExceeded`, `suprnova::queue::errors::TimeoutExceeded`)
  - Public fields: `job_name`, `timeout`

### `suprnova::queue::events`

- [ ] struct `suprnova::queue::events::JobAttempted` · framework/src/queue/events.rs:118
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobDebounced` · framework/src/queue/events.rs:448
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobExceptionOccurred` · framework/src/queue/events.rs:132
  - Public fields: `job`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobFailed` · framework/src/queue/events.rs:148
  - Public fields: `job`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobIdentity` · framework/src/queue/events.rs:25
  - Public fields: `id`, `job_name`, `attempts`, `max_tries`, `connection`
- [ ] struct `suprnova::queue::events::JobProcessed` · framework/src/queue/events.rs:101
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobProcessing` · framework/src/queue/events.rs:87
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobQueued` · framework/src/queue/events.rs:69
  - Public fields: `id`, `job_name`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobQueueing` · framework/src/queue/events.rs:53
  - Public fields: `job_name`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobReleased` · framework/src/queue/events.rs:186
  - Public fields: `job`, `delay_secs`, `reason`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobReleasedAfterException` · framework/src/queue/events.rs:165
  - Public fields: `job`, `exception`, `delay_secs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobTimedOut` · framework/src/queue/events.rs:204
  - Public fields: `job`, `timeout`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::Looping` · framework/src/queue/events.rs:220
  - Public fields: `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueueFailedOver` · framework/src/queue/events.rs:375
  - Public fields: `connection`, `job_name`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuePaused` · framework/src/queue/events.rs:333
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueueResumed` · framework/src/queue/events.rs:349
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuesPaused` · framework/src/queue/events.rs:309
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuesResumed` · framework/src/queue/events.rs:321
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::UniqueJobSkipped` · framework/src/queue/events.rs:290
  - Public fields: `job_name`, `unique_id`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerInterrupted` · framework/src/queue/events.rs:262
  - Public fields: `connection`, `processed`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerQueuePaused` · framework/src/queue/events.rs:400
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerQueueResumed` · framework/src/queue/events.rs:428
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerStarting` · framework/src/queue/events.rs:233
  - Public fields: `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerStopping` · framework/src/queue/events.rs:246
  - Public fields: `connection`, `processed`
  - Implements: `suprnova::Event`

### `suprnova::queue::failed`

- [ ] struct `suprnova::DatabaseFailedJobStore` · framework/src/queue/failed.rs:255 (also `suprnova::queue::DatabaseFailedJobStore`, `suprnova::queue::failed::DatabaseFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::DatabaseFailedJobStore::new` · framework/src/queue/failed.rs:264
- [ ] struct `suprnova::FailedJob` · framework/src/queue/failed.rs:34 (also `suprnova::queue::FailedJob`, `suprnova::queue::failed::FailedJob`)
  - Public fields: `id`, `connection`, `queue`, `job_name`, `envelope_json`, `exception`, `failed_at`
- [ ] struct `suprnova::MemoryFailedJobStore` · framework/src/queue/failed.rs:91 (also `suprnova::queue::MemoryFailedJobStore`, `suprnova::queue::failed::MemoryFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::MemoryFailedJobStore::new` · framework/src/queue/failed.rs:97
- [ ] struct `suprnova::NullFailedJobStore` · framework/src/queue/failed.rs:194 (also `suprnova::queue::NullFailedJobStore`, `suprnova::queue::failed::NullFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::NullFailedJobStore::new` · framework/src/queue/failed.rs:198
- [ ] trait `suprnova::FailedJobStore` · framework/src/queue/failed.rs:54 (also `suprnova::queue::FailedJobStore`, `suprnova::queue::failed::FailedJobStore`)
  - Implemented here by: `DatabaseFailedJobStore`, `MemoryFailedJobStore`, `NullFailedJobStore`
  - [ ] fn `suprnova::FailedJobStore::log` · framework/src/queue/failed.rs:56 (required)
  - [ ] fn `suprnova::FailedJobStore::all` · framework/src/queue/failed.rs:65 (required)
  - [ ] fn `suprnova::FailedJobStore::ids` · framework/src/queue/failed.rs:68 (required)
  - [ ] fn `suprnova::FailedJobStore::find` · framework/src/queue/failed.rs:71 (required)
  - [ ] fn `suprnova::FailedJobStore::forget` · framework/src/queue/failed.rs:74 (required)
  - [ ] fn `suprnova::FailedJobStore::flush` · framework/src/queue/failed.rs:78 (required)
  - [ ] fn `suprnova::FailedJobStore::count` · framework/src/queue/failed.rs:81 (required)

### `suprnova::queue::failover`

- [ ] struct `suprnova::FailoverQueueDriver` · framework/src/queue/failover.rs:111 (also `suprnova::queue::FailoverQueueDriver`, `suprnova::queue::failover::FailoverQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::FailoverQueueDriver::new` · framework/src/queue/failover.rs:154

### `suprnova::queue::inspect`

- [ ] struct `suprnova::InspectedJob` · framework/src/queue/inspect.rs:28 (also `suprnova::queue::InspectedJob`, `suprnova::queue::inspect::InspectedJob`)
  - Public fields: `id`, `queue`, `name`, `attempts`, `payload`, `created_at`
  - [ ] fn `suprnova::InspectedJob::from_envelope` · framework/src/queue/inspect.rs:50

### `suprnova::queue::job`

- [ ] enum `suprnova::BackoffSchedule` · framework/src/queue/job.rs:11 (also `suprnova::queue::BackoffSchedule`, `suprnova::queue::job::BackoffSchedule`)
  - Variants: `Fixed`, `Exponential`, `Sequence`
- [ ] trait `suprnova::Job` · framework/src/queue/job.rs:59 (also `suprnova::prelude::Job`, `suprnova::queue::Job`, `suprnova::queue::job::Job`)
  - Implemented here by: `SendMailJob`, `SendNotificationJob`
  - [ ] fn `suprnova::Job::job_name` · framework/src/queue/job.rs:62 (required)
  - [ ] fn `suprnova::Job::handle` · framework/src/queue/job.rs:67 (required)
  - [ ] fn `suprnova::Job::queue` · framework/src/queue/job.rs:83 (provided)
  - [ ] fn `suprnova::Job::connection` · framework/src/queue/job.rs:100 (provided)
  - [ ] fn `suprnova::Job::delay` · framework/src/queue/job.rs:117 (provided)
  - [ ] fn `suprnova::Job::max_tries` · framework/src/queue/job.rs:125 (provided)
  - [ ] fn `suprnova::Job::backoff` · framework/src/queue/job.rs:133 (provided)
  - [ ] fn `suprnova::Job::timeout` · framework/src/queue/job.rs:141 (provided)
  - [ ] fn `suprnova::Job::fail_on_timeout` · framework/src/queue/job.rs:150 (provided)
  - [ ] fn `suprnova::Job::after_commit` · framework/src/queue/job.rs:175 (provided)
  - [ ] fn `suprnova::Job::unique_id` · framework/src/queue/job.rs:191 (provided)
  - [ ] fn `suprnova::Job::unique_for` · framework/src/queue/job.rs:204 (provided)
  - [ ] fn `suprnova::Job::unique_until_processing` · framework/src/queue/job.rs:225 (provided)
  - [ ] fn `suprnova::Job::debounce_for` · framework/src/queue/job.rs:251 (provided)
  - [ ] fn `suprnova::Job::max_debounce_wait` · framework/src/queue/job.rs:266 (provided)
  - [ ] fn `suprnova::Job::debounce_id` · framework/src/queue/job.rs:284 (provided)
  - [ ] fn `suprnova::Job::middleware` · framework/src/queue/job.rs:295 (provided)

### `suprnova::queue::memory`

- [ ] struct `suprnova::MemoryQueueDriver` · framework/src/queue/memory.rs:94 (also `suprnova::queue::MemoryQueueDriver`, `suprnova::queue::memory::MemoryQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::MemoryQueueDriver::new` · framework/src/queue/memory.rs:182

### `suprnova::queue::middleware`

- [ ] struct `suprnova::FailOnException` · framework/src/queue/middleware.rs:375 (also `suprnova::queue::FailOnException`, `suprnova::queue::middleware::FailOnException`)
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::FailOnException::new` · framework/src/queue/middleware.rs:382
  - [ ] fn `suprnova::FailOnException::on_substring` · framework/src/queue/middleware.rs:396
- [ ] struct `suprnova::RateLimited` · framework/src/queue/middleware.rs:191 (also `suprnova::queue::RateLimited`, `suprnova::queue::middleware::RateLimited`)
  - Public fields: `max_attempts`, `decay`, `key`, `release_after`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::RateLimited::new` · framework/src/queue/middleware.rs:204
  - [ ] fn `suprnova::RateLimited::by` · framework/src/queue/middleware.rs:214
  - [ ] fn `suprnova::RateLimited::release_after` · framework/src/queue/middleware.rs:220
- [ ] struct `suprnova::Skip` · framework/src/queue/middleware.rs:338 (also `suprnova::queue::Skip`, `suprnova::queue::middleware::Skip`)
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::Skip::when` · framework/src/queue/middleware.rs:344
  - [ ] fn `suprnova::Skip::unless` · framework/src/queue/middleware.rs:349
- [ ] struct `suprnova::SkipIfBatchCancelled` · framework/src/queue/middleware.rs:429 (also `suprnova::queue::SkipIfBatchCancelled`, `suprnova::queue::middleware::SkipIfBatchCancelled`)
  - Implements: `suprnova::JobMiddleware`
- [ ] struct `suprnova::ThrottlesExceptions` · framework/src/queue/middleware.rs:260 (also `suprnova::queue::ThrottlesExceptions`, `suprnova::queue::middleware::ThrottlesExceptions`)
  - Public fields: `max_attempts`, `decay`, `backoff`, `key`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::ThrottlesExceptions::new` · framework/src/queue/middleware.rs:274
  - [ ] fn `suprnova::ThrottlesExceptions::backoff` · framework/src/queue/middleware.rs:285
  - [ ] fn `suprnova::ThrottlesExceptions::by` · framework/src/queue/middleware.rs:291
- [ ] struct `suprnova::WithoutOverlapping` · framework/src/queue/middleware.rs:64 (also `suprnova::queue::WithoutOverlapping`, `suprnova::queue::middleware::WithoutOverlapping`)
  - Public fields: `key`, `release_after`, `expires_after`, `prefix`, `share_key`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::WithoutOverlapping::new` · framework/src/queue/middleware.rs:81
  - [ ] fn `suprnova::WithoutOverlapping::release_after` · framework/src/queue/middleware.rs:93
  - [ ] fn `suprnova::WithoutOverlapping::dont_release` · framework/src/queue/middleware.rs:100
  - [ ] fn `suprnova::WithoutOverlapping::expire_after` · framework/src/queue/middleware.rs:106
  - [ ] fn `suprnova::WithoutOverlapping::with_prefix` · framework/src/queue/middleware.rs:112
  - [ ] fn `suprnova::WithoutOverlapping::shared` · framework/src/queue/middleware.rs:119
- [ ] trait `suprnova::JobMiddleware` · framework/src/queue/middleware.rs:47 (also `suprnova::queue::JobMiddleware`, `suprnova::queue::middleware::JobMiddleware`)
  - Implemented here by: `FailOnException`, `RateLimited`, `Skip`, `SkipIfBatchCancelled`, `ThrottlesExceptions`, `WithoutOverlapping`
  - [ ] fn `suprnova::JobMiddleware::handle` · framework/src/queue/middleware.rs:51 (required)
- [ ] type `suprnova::JobMiddlewareNext` · framework/src/queue/middleware.rs:39 (also `suprnova::queue::JobMiddlewareNext`, `suprnova::queue::middleware::Next`)

### `suprnova::queue::null`

- [ ] struct `suprnova::NullQueueDriver` · framework/src/queue/null.rs:19 (also `suprnova::queue::NullQueueDriver`, `suprnova::queue::null::NullQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::NullQueueDriver::new` · framework/src/queue/null.rs:23

### `suprnova::queue::outcome`

- [ ] enum `suprnova::JobOutcome` · framework/src/queue/outcome.rs:20 (also `suprnova::queue::JobOutcome`, `suprnova::queue::outcome::JobOutcome`)
  - Variants: `Completed`, `Released`, `Failed`, `Deleted`
  - [ ] fn `suprnova::JobOutcome::is_release` · framework/src/queue/outcome.rs:52
  - [ ] fn `suprnova::JobOutcome::is_terminal` · framework/src/queue/outcome.rs:57

### `suprnova::queue::redis`

- [ ] struct `suprnova::RedisQueueDriver` · framework/src/queue/redis.rs:1485 (also `suprnova::queue::RedisQueueDriver`, `suprnova::queue::redis::RedisQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::RedisQueueDriver::connect` · framework/src/queue/redis.rs:1647

### `suprnova::queue::retry`

- [ ] fn `suprnova::queue::retry::next_delay` · framework/src/queue/retry.rs:16

### `suprnova::queue::routing`

- [ ] struct `suprnova::QueueRoute` · framework/src/queue/routing.rs:66 (also `suprnova::queue::QueueRoute`, `suprnova::queue::routing::QueueRoute`)
  - Public fields: `connection`, `queue`

### `suprnova::queue::sync`

- [ ] struct `suprnova::SyncQueueDriver` · framework/src/queue/sync.rs:27 (also `suprnova::queue::SyncQueueDriver`, `suprnova::queue::sync::SyncQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::SyncQueueDriver::new` · framework/src/queue/sync.rs:31

### `suprnova::queue::worker`

- [ ] fn `suprnova::queue::worker::dispatch_by_name` · framework/src/queue/worker.rs:131
- [ ] fn `suprnova::queue::worker::register_job` · framework/src/queue/worker.rs:89
- [ ] fn `suprnova::queue::worker::registered_job_names` · framework/src/queue/worker.rs:333
- [ ] fn `suprnova::queue::worker::run_through_middleware` · framework/src/queue/worker.rs:278
- [ ] fn `suprnova::queue::worker::run_worker` · framework/src/queue/worker.rs:533
- [ ] struct `suprnova::queue::worker::WorkerConfig` · framework/src/queue/worker.rs:352
  - Public fields: `visibility_timeout`, `poll_interval`, `max_jobs`, `queues`

### `suprnova::queue`

- [ ] fn `suprnova::queue::bootstrap_default` · framework/src/queue/mod.rs:1269
- [ ] fn `suprnova::queue::bootstrap_from_env` · framework/src/queue/mod.rs:1292
- [ ] struct `suprnova::EnvelopeOverrides` · framework/src/queue/mod.rs:92 (also `suprnova::queue::EnvelopeOverrides`)
  - Public fields: `queue`, `connection`, `timeout`, `fail_on_timeout`, `max_tries`, `backoff`, `after_commit`
- [ ] struct `suprnova::Queue` · framework/src/queue/mod.rs:124 (also `suprnova::prelude::Queue`, `suprnova::queue::Queue`)
  - [ ] fn `suprnova::Queue::route` · framework/src/queue/mod.rs:163
  - [ ] fn `suprnova::Queue::try_route` · framework/src/queue/mod.rs:176
  - [ ] fn `suprnova::Queue::route_for` · framework/src/queue/mod.rs:184
  - [ ] fn `suprnova::Queue::forward` · framework/src/queue/mod.rs:234
  - [ ] fn `suprnova::Queue::forward_on` · framework/src/queue/mod.rs:261
  - [ ] fn `suprnova::Queue::try_forward` · framework/src/queue/mod.rs:269
  - [ ] fn `suprnova::Queue::forward_for` · framework/src/queue/mod.rs:281
  - [ ] fn `suprnova::Queue::push` · framework/src/queue/mod.rs:309
  - [ ] fn `suprnova::Queue::push_after_commit` · framework/src/queue/mod.rs:328
  - [ ] fn `suprnova::Queue::push_later` · framework/src/queue/mod.rs:349
  - [ ] fn `suprnova::Queue::later` · framework/src/queue/mod.rs:393
  - [ ] fn `suprnova::Queue::push_with` · framework/src/queue/mod.rs:406
  - [ ] fn `suprnova::Queue::later_with` · framework/src/queue/mod.rs:415
  - [ ] fn `suprnova::Queue::push_debounced` · framework/src/queue/mod.rs:614
  - [ ] fn `suprnova::Queue::push_unique` · framework/src/queue/mod.rs:664
  - [ ] fn `suprnova::Queue::push_unique_later` · framework/src/queue/mod.rs:672
  - [ ] fn `suprnova::Queue::later_unique` · framework/src/queue/mod.rs:681
  - [ ] fn `suprnova::Queue::bulk` · framework/src/queue/mod.rs:868
  - [ ] fn `suprnova::Queue::batch` · framework/src/queue/mod.rs:904
  - [ ] fn `suprnova::Queue::chain` · framework/src/queue/mod.rs:909
  - [ ] fn `suprnova::Queue::size` · framework/src/queue/mod.rs:915
  - [ ] fn `suprnova::Queue::pending_size` · framework/src/queue/mod.rs:920
  - [ ] fn `suprnova::Queue::delayed_size` · framework/src/queue/mod.rs:925
  - [ ] fn `suprnova::Queue::reserved_size` · framework/src/queue/mod.rs:930
  - [ ] fn `suprnova::Queue::pending_jobs` · framework/src/queue/mod.rs:940
  - [ ] fn `suprnova::Queue::delayed_jobs` · framework/src/queue/mod.rs:947
  - [ ] fn `suprnova::Queue::reserved_jobs` · framework/src/queue/mod.rs:954
  - [ ] fn `suprnova::Queue::clear` · framework/src/queue/mod.rs:960
  - [ ] fn `suprnova::Queue::restart` · framework/src/queue/mod.rs:973
  - [ ] fn `suprnova::Queue::restart_signal` · framework/src/queue/mod.rs:981
  - [ ] fn `suprnova::Queue::pause` · framework/src/queue/mod.rs:997
  - [ ] fn `suprnova::Queue::resume` · framework/src/queue/mod.rs:1011
  - [ ] fn `suprnova::Queue::pause_all` · framework/src/queue/mod.rs:1026
  - [ ] fn `suprnova::Queue::resume_all` · framework/src/queue/mod.rs:1042
  - [ ] fn `suprnova::Queue::is_paused` · framework/src/queue/mod.rs:1051
  - [ ] fn `suprnova::Queue::paused_queues` · framework/src/queue/mod.rs:1066
  - [ ] fn `suprnova::Queue::set_failed_store` · framework/src/queue/mod.rs:1087
  - [ ] fn `suprnova::Queue::failed_store` · framework/src/queue/mod.rs:1094
  - [ ] fn `suprnova::Queue::retry_failed` · framework/src/queue/mod.rs:1106
  - [ ] fn `suprnova::Queue::retry_all_failed` · framework/src/queue/mod.rs:1132
  - [ ] fn `suprnova::Queue::set_batch_repository` · framework/src/queue/mod.rs:1166
  - [ ] fn `suprnova::Queue::batch_repository` · framework/src/queue/mod.rs:1171
  - [ ] fn `suprnova::Queue::set_connection_name` · framework/src/queue/mod.rs:1177
  - [ ] fn `suprnova::Queue::connection_name` · framework/src/queue/mod.rs:1185
  - [ ] fn `suprnova::Queue::set_driver` · framework/src/queue/mod.rs:1198
  - [ ] fn `suprnova::Queue::driver_name` · framework/src/queue/mod.rs:1217
  - [ ] fn `suprnova::Queue::driver` · framework/src/queue/mod.rs:1228
