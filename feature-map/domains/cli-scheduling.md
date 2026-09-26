# Feature map: `manual/cli-scheduling.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 6 checked.

## Command line

### App runner (`suprnova::Application`; the project binary)

- [ ] command `app schedule:work` · framework/src/app/mod.rs:150
  - Run the scheduler daemon (checks every minute)
- [ ] command `app schedule:run` · framework/src/app/mod.rs:153
  - Run all due scheduled tasks once
- [ ] command `app schedule:list` · framework/src/app/mod.rs:156
  - List all registered scheduled tasks

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova schedule:run` · suprnova-cli/src/main.rs:311
  - Run all due scheduled tasks once (typically called by cron every minute)
- [ ] command `suprnova schedule:work` · suprnova-cli/src/main.rs:314
  - Start the scheduler daemon (runs continuously, checks every minute)
- [ ] command `suprnova schedule:list` · suprnova-cli/src/main.rs:317
  - List all registered scheduled tasks
