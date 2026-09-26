# Feature map: `manual/bus.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 15 checked.

## Rust API: suprnova

### `suprnova::bus::command`

- [ ] trait `suprnova::bus::command::Command` · framework/src/bus/command.rs:14
  - [ ] type `suprnova::bus::command::Command::Output` · framework/src/bus/command.rs:16
  - [ ] fn `suprnova::bus::command::Command::command_name` · framework/src/bus/command.rs:19 (required)
- [ ] trait `suprnova::bus::command::Handler` · framework/src/bus/command.rs:27
  - [ ] fn `suprnova::bus::command::Handler::handle` · framework/src/bus/command.rs:30 (required)

### `suprnova::bus`

- [ ] struct `suprnova::Bus` · framework/src/bus/mod.rs:83 (also `suprnova::bus::Bus`, `suprnova::prelude::Bus`)
  - [ ] fn `suprnova::Bus::register` · framework/src/bus/mod.rs:96
  - [ ] fn `suprnova::Bus::dispatch` · framework/src/bus/mod.rs:147
  - [ ] fn `suprnova::Bus::chain` · framework/src/bus/mod.rs:180
  - [ ] fn `suprnova::Bus::batch` · framework/src/bus/mod.rs:201
- [ ] enum `suprnova::Dispatched` · framework/src/bus/mod.rs:23 (also `suprnova::bus::Dispatched`)
  - Variants: `Executed`, `Captured`
  - [ ] fn `suprnova::Dispatched::unwrap_executed` · framework/src/bus/mod.rs:32
  - [ ] fn `suprnova::Dispatched::is_executed` · framework/src/bus/mod.rs:42
  - [ ] fn `suprnova::Dispatched::is_captured` · framework/src/bus/mod.rs:47
  - [ ] fn `suprnova::Dispatched::executed` · framework/src/bus/mod.rs:52
