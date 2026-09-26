# Feature map: `manual/scheduling.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 117 checked.

## Rust API: suprnova

### `suprnova::schedule::builder`

- [ ] struct `suprnova::TaskBuilder` · framework/src/schedule/builder.rs:34 (also `suprnova::schedule::TaskBuilder`, `suprnova::schedule::builder::TaskBuilder`)
  - [ ] fn `suprnova::TaskBuilder::new` · framework/src/schedule/builder.rs:51
  - [ ] fn `suprnova::TaskBuilder::from_async` · framework/src/schedule/builder.rs:72
  - [ ] fn `suprnova::TaskBuilder::from_task` · framework/src/schedule/builder.rs:103
  - [ ] fn `suprnova::TaskBuilder::cron` · framework/src/schedule/builder.rs:138
  - [ ] fn `suprnova::TaskBuilder::try_cron` · framework/src/schedule/builder.rs:152
  - [ ] fn `suprnova::TaskBuilder::every_minute` · framework/src/schedule/builder.rs:158
  - [ ] fn `suprnova::TaskBuilder::every_two_minutes` · framework/src/schedule/builder.rs:164
  - [ ] fn `suprnova::TaskBuilder::every_five_minutes` · framework/src/schedule/builder.rs:170
  - [ ] fn `suprnova::TaskBuilder::every_ten_minutes` · framework/src/schedule/builder.rs:176
  - [ ] fn `suprnova::TaskBuilder::every_fifteen_minutes` · framework/src/schedule/builder.rs:182
  - [ ] fn `suprnova::TaskBuilder::every_thirty_minutes` · framework/src/schedule/builder.rs:188
  - [ ] fn `suprnova::TaskBuilder::hourly` · framework/src/schedule/builder.rs:194
  - [ ] fn `suprnova::TaskBuilder::hourly_at` · framework/src/schedule/builder.rs:214
  - [ ] fn `suprnova::TaskBuilder::try_hourly_at` · framework/src/schedule/builder.rs:226
  - [ ] fn `suprnova::TaskBuilder::every_two_hours` · framework/src/schedule/builder.rs:232
  - [ ] fn `suprnova::TaskBuilder::every_three_hours` · framework/src/schedule/builder.rs:238
  - [ ] fn `suprnova::TaskBuilder::every_four_hours` · framework/src/schedule/builder.rs:244
  - [ ] fn `suprnova::TaskBuilder::every_six_hours` · framework/src/schedule/builder.rs:250
  - [ ] fn `suprnova::TaskBuilder::daily` · framework/src/schedule/builder.rs:256
  - [ ] fn `suprnova::TaskBuilder::daily_at` · framework/src/schedule/builder.rs:279
  - [ ] fn `suprnova::TaskBuilder::try_daily_at` · framework/src/schedule/builder.rs:294
  - [ ] fn `suprnova::TaskBuilder::twice_daily` · framework/src/schedule/builder.rs:315
  - [ ] fn `suprnova::TaskBuilder::try_twice_daily` · framework/src/schedule/builder.rs:327
  - [ ] fn `suprnova::TaskBuilder::at` · framework/src/schedule/builder.rs:353
  - [ ] fn `suprnova::TaskBuilder::try_at` · framework/src/schedule/builder.rs:366
  - [ ] fn `suprnova::TaskBuilder::weekly` · framework/src/schedule/builder.rs:372
  - [ ] fn `suprnova::TaskBuilder::weekly_on` · framework/src/schedule/builder.rs:387
  - [ ] fn `suprnova::TaskBuilder::days` · framework/src/schedule/builder.rs:402
  - [ ] fn `suprnova::TaskBuilder::weekdays` · framework/src/schedule/builder.rs:408
  - [ ] fn `suprnova::TaskBuilder::weekends` · framework/src/schedule/builder.rs:414
  - [ ] fn `suprnova::TaskBuilder::sundays` · framework/src/schedule/builder.rs:420
  - [ ] fn `suprnova::TaskBuilder::mondays` · framework/src/schedule/builder.rs:426
  - [ ] fn `suprnova::TaskBuilder::tuesdays` · framework/src/schedule/builder.rs:432
  - [ ] fn `suprnova::TaskBuilder::wednesdays` · framework/src/schedule/builder.rs:438
  - [ ] fn `suprnova::TaskBuilder::thursdays` · framework/src/schedule/builder.rs:444
  - [ ] fn `suprnova::TaskBuilder::fridays` · framework/src/schedule/builder.rs:450
  - [ ] fn `suprnova::TaskBuilder::saturdays` · framework/src/schedule/builder.rs:456
  - [ ] fn `suprnova::TaskBuilder::monthly` · framework/src/schedule/builder.rs:462
  - [ ] fn `suprnova::TaskBuilder::monthly_on` · framework/src/schedule/builder.rs:483
  - [ ] fn `suprnova::TaskBuilder::try_monthly_on` · framework/src/schedule/builder.rs:495
  - [ ] fn `suprnova::TaskBuilder::quarterly` · framework/src/schedule/builder.rs:501
  - [ ] fn `suprnova::TaskBuilder::yearly` · framework/src/schedule/builder.rs:507
  - [ ] fn `suprnova::TaskBuilder::name` · framework/src/schedule/builder.rs:519
  - [ ] fn `suprnova::TaskBuilder::description` · framework/src/schedule/builder.rs:527
  - [ ] fn `suprnova::TaskBuilder::timezone` · framework/src/schedule/builder.rs:562
  - [ ] fn `suprnova::TaskBuilder::try_timezone` · framework/src/schedule/builder.rs:577
  - [ ] fn `suprnova::TaskBuilder::without_overlapping` · framework/src/schedule/builder.rs:602
  - [ ] fn `suprnova::TaskBuilder::without_overlapping_for` · framework/src/schedule/builder.rs:613
  - [ ] fn `suprnova::TaskBuilder::on_one_server` · framework/src/schedule/builder.rs:653
  - [ ] fn `suprnova::TaskBuilder::on_one_server_for` · framework/src/schedule/builder.rs:665
  - [ ] fn `suprnova::TaskBuilder::run_in_background` · framework/src/schedule/builder.rs:675

### `suprnova::schedule::expression`

- [ ] struct `suprnova::CronExpression` · framework/src/schedule/expression.rs:63 (also `suprnova::schedule::CronExpression`, `suprnova::schedule::expression::CronExpression`)
  - [ ] fn `suprnova::CronExpression::parse` · framework/src/schedule/expression.rs:287
  - [ ] fn `suprnova::CronExpression::is_due` · framework/src/schedule/expression.rs:313
  - [ ] fn `suprnova::CronExpression::is_due_at` · framework/src/schedule/expression.rs:332
  - [ ] fn `suprnova::CronExpression::next_run_after` · framework/src/schedule/expression.rs:407
  - [ ] fn `suprnova::CronExpression::expression` · framework/src/schedule/expression.rs:430
  - [ ] fn `suprnova::CronExpression::at` · framework/src/schedule/expression.rs:474
  - [ ] fn `suprnova::CronExpression::try_at` · framework/src/schedule/expression.rs:505
  - [ ] fn `suprnova::CronExpression::every_minute` · framework/src/schedule/expression.rs:521
  - [ ] fn `suprnova::CronExpression::every_n_minutes` · framework/src/schedule/expression.rs:533
  - [ ] fn `suprnova::CronExpression::try_every_n_minutes` · framework/src/schedule/expression.rs:549
  - [ ] fn `suprnova::CronExpression::hourly` · framework/src/schedule/expression.rs:559
  - [ ] fn `suprnova::CronExpression::hourly_at` · framework/src/schedule/expression.rs:569
  - [ ] fn `suprnova::CronExpression::try_hourly_at` · framework/src/schedule/expression.rs:580
  - [ ] fn `suprnova::CronExpression::daily` · framework/src/schedule/expression.rs:590
  - [ ] fn `suprnova::CronExpression::daily_at` · framework/src/schedule/expression.rs:605
  - [ ] fn `suprnova::CronExpression::try_daily_at` · framework/src/schedule/expression.rs:620
  - [ ] fn `suprnova::CronExpression::weekly` · framework/src/schedule/expression.rs:638
  - [ ] fn `suprnova::CronExpression::weekly_on` · framework/src/schedule/expression.rs:643
  - [ ] fn `suprnova::CronExpression::on_days` · framework/src/schedule/expression.rs:648
  - [ ] fn `suprnova::CronExpression::monthly` · framework/src/schedule/expression.rs:654
  - [ ] fn `suprnova::CronExpression::monthly_on` · framework/src/schedule/expression.rs:665
  - [ ] fn `suprnova::CronExpression::try_monthly_on` · framework/src/schedule/expression.rs:677
  - [ ] fn `suprnova::CronExpression::quarterly` · framework/src/schedule/expression.rs:685
  - [ ] fn `suprnova::CronExpression::yearly` · framework/src/schedule/expression.rs:690
  - [ ] fn `suprnova::CronExpression::weekdays` · framework/src/schedule/expression.rs:695
  - [ ] fn `suprnova::CronExpression::weekends` · framework/src/schedule/expression.rs:700
- [ ] enum `suprnova::DayOfWeek` · framework/src/schedule/expression.rs:10 (also `suprnova::schedule::DayOfWeek`, `suprnova::schedule::expression::DayOfWeek`)
  - Variants: `Sunday`, `Monday`, `Tuesday`, `Wednesday`, `Thursday`, `Friday`, `Saturday`
  - [ ] fn `suprnova::DayOfWeek::from_chrono` · framework/src/schedule/expression.rs:29

### `suprnova::schedule::task`

- [ ] struct `suprnova::TaskEntry` · framework/src/schedule/task.rs:154 (also `suprnova::schedule::TaskEntry`, `suprnova::schedule::task::TaskEntry`)
  - Public fields: `name`, `expression`, `task`, `description`, `without_overlapping`, `run_in_background`, `overlap_ttl`, `on_one_server`, `one_server_ttl`, `timezone`, `state`
  - [ ] fn `suprnova::TaskEntry::is_due` · framework/src/schedule/task.rs:196
  - [ ] fn `suprnova::TaskEntry::run` · framework/src/schedule/task.rs:225
  - [ ] fn `suprnova::TaskEntry::schedule_description` · framework/src/schedule/task.rs:239
- [ ] struct `suprnova::schedule::task::TaskState` · framework/src/schedule/task.rs:46
  - [ ] fn `suprnova::schedule::task::TaskState::new` · framework/src/schedule/task.rs:71
  - [ ] fn `suprnova::schedule::task::TaskState::skip_count` · framework/src/schedule/task.rs:77
- [ ] trait `suprnova::Task` · framework/src/schedule/task.rs:137 (also `suprnova::schedule::Task`, `suprnova::schedule::task::Task`)
  - [ ] fn `suprnova::Task::handle` · framework/src/schedule/task.rs:139 (required)
- [ ] trait `suprnova::schedule::TaskHandler` · framework/src/schedule/task.rs:95 (also `suprnova::schedule::task::TaskHandler`)
  - [ ] fn `suprnova::schedule::TaskHandler::handle` · framework/src/schedule/task.rs:97 (required)
- [ ] type `suprnova::schedule::BoxedFuture` · framework/src/schedule/task.rs:89 (also `suprnova::schedule::task::BoxedFuture`)
- [ ] type `suprnova::schedule::BoxedTask` · framework/src/schedule/task.rs:83 (also `suprnova::schedule::task::BoxedTask`)
- [ ] type `suprnova::TaskResult` · framework/src/schedule/task.rs:86 (also `suprnova::schedule::TaskResult`, `suprnova::schedule::task::TaskResult`)
- [ ] const `suprnova::schedule::task::DEFAULT_ON_ONE_SERVER_TTL` · framework/src/schedule/task.rs:32
- [ ] const `suprnova::schedule::task::DEFAULT_WITHOUT_OVERLAPPING_TTL` · framework/src/schedule/task.rs:20

### `suprnova::schedule`

- [ ] struct `suprnova::Schedule` · framework/src/schedule/mod.rs:138 (also `suprnova::schedule::Schedule`)
  - [ ] fn `suprnova::Schedule::new` · framework/src/schedule/mod.rs:181
  - [ ] fn `suprnova::Schedule::timezone` · framework/src/schedule/mod.rs:208
  - [ ] fn `suprnova::Schedule::validate_single_server_locking` · framework/src/schedule/mod.rs:238
  - [ ] fn `suprnova::Schedule::task` · framework/src/schedule/mod.rs:287
  - [ ] fn `suprnova::Schedule::call` · framework/src/schedule/mod.rs:315
  - [ ] fn `suprnova::Schedule::add` · framework/src/schedule/mod.rs:342
  - [ ] fn `suprnova::Schedule::try_add` · framework/src/schedule/mod.rs:360
  - [ ] fn `suprnova::Schedule::tasks` · framework/src/schedule/mod.rs:380
  - [ ] fn `suprnova::Schedule::len` · framework/src/schedule/mod.rs:385
  - [ ] fn `suprnova::Schedule::is_empty` · framework/src/schedule/mod.rs:390
  - [ ] fn `suprnova::Schedule::due_tasks` · framework/src/schedule/mod.rs:395
  - [ ] fn `suprnova::Schedule::run_due_tasks` · framework/src/schedule/mod.rs:409
  - [ ] fn `suprnova::Schedule::run_due_tasks_into` · framework/src/schedule/mod.rs:424
  - [ ] fn `suprnova::Schedule::run_all_tasks` · framework/src/schedule/mod.rs:434
  - [ ] fn `suprnova::Schedule::run_all_tasks_into` · framework/src/schedule/mod.rs:444
  - [ ] fn `suprnova::Schedule::find` · framework/src/schedule/mod.rs:452
  - [ ] fn `suprnova::Schedule::run_task` · framework/src/schedule/mod.rs:457
- [ ] type `suprnova::schedule::ScheduledTaskJoin` · framework/src/schedule/mod.rs:101

### `suprnova`

- [ ] macro `suprnova::schedule_task` · framework/src/schedule/mod.rs:591
