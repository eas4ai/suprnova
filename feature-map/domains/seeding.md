# Feature map: `manual/seeding.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 10 checked.

## Command line

### Console binary (framework-registered commands)

- [ ] command `console db:seed` · framework/src/console/builtins/db_seed.rs:36
  - Run seeders (all by default, or one via --class=<Name>)
  - argument `[__suprnova_trailing_args]...`

## Rust API: suprnova

### `suprnova::seed`

- [ ] fn `suprnova::seed::count` · framework/src/seed/mod.rs:200
- [ ] fn `suprnova::seed::is_registered` · framework/src/seed/mod.rs:215
- [ ] fn `suprnova::seed::register` · framework/src/seed/mod.rs:130
- [ ] fn `suprnova::seed::run_all` · framework/src/seed/mod.rs:150
- [ ] fn `suprnova::seed::run_one` · framework/src/seed/mod.rs:178
- [ ] fn `suprnova::seed::without_events` · framework/src/seed/mod.rs:286
- [ ] trait `suprnova::Seeder` · framework/src/seed/mod.rs:109 (also `suprnova::seed::Seeder`)
  - [ ] fn `suprnova::Seeder::name` · framework/src/seed/mod.rs:113 (required)
  - [ ] fn `suprnova::Seeder::run` · framework/src/seed/mod.rs:120 (required)
