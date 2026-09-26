# Feature map: `manual/console.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 17 checked.

## Rust API: suprnova

### `suprnova::console::output`

- [ ] fn `suprnova::two_column_detail` · framework/src/console/output.rs:37 (also `suprnova::console::output::two_column_detail`, `suprnova::console::two_column_detail`)
- [ ] const `suprnova::console::DETAIL_WIDTH` · framework/src/console/output.rs:18 (also `suprnova::console::output::DETAIL_WIDTH`)

### `suprnova::console::typed` (private module; items are public through re-exports)

- [ ] trait `suprnova::TypedCommand` · framework/src/console/typed.rs:48 (also `suprnova::console::TypedCommand`)
  - Implemented here by: `eloquent::console::prune::PruneArgs`
  - [ ] fn `suprnova::TypedCommand::run` · framework/src/console/typed.rs:51 (required)

### `suprnova::console`

- [ ] fn `suprnova::dispatch_argv` · framework/src/console/mod.rs:123 (also `suprnova::console::dispatch_argv`)
- [ ] fn `suprnova::console::dispatch_argv_with_init` · framework/src/console/mod.rs:142
- [ ] fn `suprnova::console::find` · framework/src/console/mod.rs:85
- [ ] fn `suprnova::console::list` · framework/src/console/mod.rs:92
- [ ] fn `suprnova::console::set_version` · framework/src/console/mod.rs:80
- [ ] struct `suprnova::CommandEntry` · framework/src/console/mod.rs:51 (also `suprnova::console::CommandEntry`)
  - Public fields: `name`, `description`, `clap_builder`, `handler`
- [ ] type `suprnova::CommandHandler` · framework/src/console/mod.rs:44 (also `suprnova::console::CommandHandler`)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::Command` · suprnova-macros/src/lib.rs:536 (re-exported as `suprnova::Command`)
  - Form: derive `#[derive(Command)]`
  - Helper attributes: `#[console]`
  - [ ] argument `#[console(name)]` · suprnova-macros/src/console_derive.rs:76
  - [ ] argument `#[console(description)]` · suprnova-macros/src/console_derive.rs:77
- [ ] proc macro `suprnova_macros::command` · suprnova-macros/src/lib.rs:563 (re-exported as `suprnova::command`)
  - Form: attribute `#[command]`
  - [ ] argument `name = "..."` · suprnova-macros/src/command.rs:75
  - [ ] argument `description = "..."` · suprnova-macros/src/command.rs:76
