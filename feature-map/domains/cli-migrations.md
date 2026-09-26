# Feature map: `manual/cli-migrations.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 9 checked.

## Command line

### App runner (`suprnova::Application`; the project binary)

- [ ] command `app migrate` · framework/src/app/mod.rs:131
  - Run pending database migrations
- [ ] command `app migrate:status` · framework/src/app/mod.rs:133
  - Show migration status
- [ ] command `app migrate:rollback` · framework/src/app/mod.rs:136
  - Rollback the last migration(s)
  - argument `[STEPS]`: Number of migrations to rollback [default: 1]
- [ ] command `app migrate:fresh` · framework/src/app/mod.rs:143
  - Drop all tables and re-run all migrations

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova migrate` · suprnova-cli/src/main.rs:267
  - Run all pending database migrations
- [ ] command `suprnova migrate:rollback` · suprnova-cli/src/main.rs:269
  - Rollback the last database migration(s)
- [ ] command `suprnova migrate:status` · suprnova-cli/src/main.rs:276
  - Show the status of all migrations
- [ ] command `suprnova migrate:fresh` · suprnova-cli/src/main.rs:279
  - Drop all tables and re-run all migrations
- [ ] command `suprnova db:sync` · suprnova-cli/src/main.rs:288
  - Sync database schema to entity files (runs migrations + generates entities)
