# Feature map: `manual/supervisors.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 14 checked.

## Rust API: suprnova

### `suprnova::supervisor::registry`

- [ ] struct `suprnova::SupervisorEntry` · framework/src/supervisor/registry.rs:30 (also `suprnova::supervisor::SupervisorEntry`, `suprnova::supervisor::registry::SupervisorEntry`)
  - Public fields: `factory`

### `suprnova::supervisor`

- [ ] fn `suprnova::supervisor::run_with_restart_for_testing` · framework/src/supervisor/mod.rs:498
- [ ] fn `suprnova::supervisor::run_with_restart_for_testing_with_cancel` · framework/src/supervisor/mod.rs:507
- [ ] fn `suprnova::supervisor::supervisor_cancel_token` · framework/src/supervisor/mod.rs:155
- [ ] fn `suprnova::supervisor::supervisor_tasks` · framework/src/supervisor/mod.rs:148
- [ ] struct `suprnova::SupervisorRegistry` · framework/src/supervisor/mod.rs:224 (also `suprnova::supervisor::SupervisorRegistry`)
  - [ ] fn `suprnova::SupervisorRegistry::start_all` · framework/src/supervisor/mod.rs:239
  - [ ] fn `suprnova::SupervisorRegistry::spawn` · framework/src/supervisor/mod.rs:279
  - [ ] fn `suprnova::SupervisorRegistry::shutdown` · framework/src/supervisor/mod.rs:305
- [ ] enum `suprnova::RestartPolicy` · framework/src/supervisor/mod.rs:207 (also `suprnova::supervisor::RestartPolicy`)
  - Variants: `OnError`, `Always`, `Never`
- [ ] trait `suprnova::Supervisor` · framework/src/supervisor/mod.rs:174 (also `suprnova::supervisor::Supervisor`)
  - Implemented here by: `SessionGcSupervisor`
  - [ ] fn `suprnova::Supervisor::name` · framework/src/supervisor/mod.rs:176 (required)
  - [ ] fn `suprnova::Supervisor::run` · framework/src/supervisor/mod.rs:193 (required)
  - [ ] fn `suprnova::Supervisor::restart_policy` · framework/src/supervisor/mod.rs:198 (provided)
