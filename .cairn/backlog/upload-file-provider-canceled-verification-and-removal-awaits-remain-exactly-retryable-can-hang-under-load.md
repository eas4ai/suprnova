# upload_file_provider canceled_verification_and_removal_awaits_remain_exactly_retryable can hang under load

Surfaced from: unstated
Captured: 2026-09-14T14:48:02.982Z

Observed 2026-09-14 in gate run 20260914T141254.181389Z-3423479-91ab9f0c (workspace-tests): the test ran 1390 s until nextest sent SIGTERM at the step timeout; 8141 other tests passed. It passes alone in 6 ms and appears in no other gate run's log. Another session's cargo tests were running on the machine at the time. The test pauses a controlled store at StorePausePoint::Sync and Remove, aborts the awaiting task, releases the store with release.notify_one and awaits wait_until_settled / wait_until_paused (crates/suprnova-live/tests/upload_file_provider.rs, from line 1723); one of those waits did not return. Not part of live-issuance-credentials; recorded, not fixed. Cancellation-safety history: 182345d4.
