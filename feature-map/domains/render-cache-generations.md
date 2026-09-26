# Feature map: `manual/render-cache-generations.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 51 checked.

## Rust API: suprnova

### `suprnova::render_cache::collector`

- [ ] fn `suprnova::render_cache::collector::begin_authorization_decision` · framework/src/render_cache/collector.rs:769
- [ ] fn `suprnova::render_cache::collector::current_report` · framework/src/render_cache/collector.rs:986
- [ ] fn `suprnova::render_cache::collector::end_authorization_decision` · framework/src/render_cache/collector.rs:787
- [ ] fn `suprnova::render_cache::collector::is_active` · framework/src/render_cache/collector.rs:487
- [ ] fn `suprnova::render_cache::collector::observe` · framework/src/render_cache/collector.rs:530
- [ ] fn `suprnova::render_cache::collector::observe_feature_read` · framework/src/render_cache/collector.rs:683
- [ ] fn `suprnova::render_cache::collector::observe_foreign_connection_read` · framework/src/render_cache/collector.rs:748
- [ ] fn `suprnova::render_cache::collector::observe_live_document_bootstrap_nonce` · framework/src/render_cache/collector.rs:919
- [ ] fn `suprnova::render_cache::collector::observe_live_document_digest` · framework/src/render_cache/collector.rs:943
- [ ] fn `suprnova::render_cache::collector::observe_live_document_mount` · framework/src/render_cache/collector.rs:834
- [ ] fn `suprnova::render_cache::collector::observe_live_document_no_store` · framework/src/render_cache/collector.rs:862
- [ ] fn `suprnova::render_cache::collector::observe_live_document_shell_island` · framework/src/render_cache/collector.rs:897
- [ ] fn `suprnova::render_cache::collector::observe_live_document_stitch_invalid` · framework/src/render_cache/collector.rs:961
- [ ] fn `suprnova::render_cache::collector::observe_live_document_stitch_slot` · framework/src/render_cache/collector.rs:878
- [ ] fn `suprnova::render_cache::collector::observe_locale_value` · framework/src/render_cache/collector.rs:734
- [ ] fn `suprnova::render_cache::collector::observe_principal_read` · framework/src/render_cache/collector.rs:694
- [ ] fn `suprnova::render_cache::collector::observe_principal_value` · framework/src/render_cache/collector.rs:705
- [ ] fn `suprnova::render_cache::collector::observe_record_read` · framework/src/render_cache/collector.rs:632
- [ ] fn `suprnova::render_cache::collector::observe_record_read_json` · framework/src/render_cache/collector.rs:664
- [ ] fn `suprnova::render_cache::collector::observe_secret_context_read` · framework/src/render_cache/collector.rs:825
- [ ] fn `suprnova::render_cache::collector::observe_session_read` · framework/src/render_cache/collector.rs:741
- [ ] fn `suprnova::render_cache::collector::observe_table_read` · framework/src/render_cache/collector.rs:590
- [ ] fn `suprnova::render_cache::collector::observe_tenant_read` · framework/src/render_cache/collector.rs:713
- [ ] fn `suprnova::render_cache::collector::observe_tenant_value` · framework/src/render_cache/collector.rs:721
- [ ] fn `suprnova::render_cache::collector::observe_undeclared` · framework/src/render_cache/collector.rs:973
- [ ] fn `suprnova::render_cache::collector::observe_unkeyed_write` · framework/src/render_cache/collector.rs:608
- [ ] fn `suprnova::render_cache::collector::observe_unobservable_read` · framework/src/render_cache/collector.rs:577
- [ ] fn `suprnova::render_cache::collector::permission_version_identity` · framework/src/render_cache/collector.rs:119
- [ ] fn `suprnova::render_cache::collector::record_identity` · framework/src/render_cache/collector.rs:657
- [ ] fn `suprnova::render_cache::collector::resolvable_reads` · framework/src/render_cache/collector.rs:816
- [ ] fn `suprnova::render_cache::collector::strip_classification_reasons_for_test` · framework/src/render_cache/collector.rs:1019 (feature: `testing`)
- [ ] struct `suprnova::render_cache::collector::CollectedContext` · framework/src/render_cache/collector.rs:140
  - Public fields: `principal_read`, `principal_material`, `tenant_read`, `tenant_material`, `locale_material`, `session_read`, `principal_reads`, `tenant_reads`, `locale_reads`, `authorization`, `secret_context_read`, `foreign_connection_read`, `overflowed`
- [ ] struct `suprnova::render_cache::collector::Collector` · framework/src/render_cache/collector.rs:379
  - [ ] fn `suprnova::render_cache::collector::Collector::scope` · framework/src/render_cache/collector.rs:390
- [ ] struct `suprnova::render_cache::collector::CollectorReport` · framework/src/render_cache/collector.rs:251
  - Public fields: `observed`, `context`, `gate`, `slot_reads`, `handler_began`, `undeclared`, `live_document`, `strip_classification_reasons`
  - [ ] fn `suprnova::render_cache::collector::CollectorReport::storable` · framework/src/render_cache/collector.rs:304
  - [ ] fn `suprnova::render_cache::collector::CollectorReport::fold_gate_into_content` · framework/src/render_cache/collector.rs:321
- [ ] struct `suprnova::render_cache::collector::ConsultWindow` · framework/src/render_cache/collector.rs:755
- [ ] struct `suprnova::render_cache::collector::GateReport` · framework/src/render_cache/collector.rs:242
  - Public fields: `context`, `observed`
- [ ] const `suprnova::render_cache::collector::PERMISSION_VERSION_CONFIG_KEY` · framework/src/render_cache/collector.rs:94

### `suprnova::render_cache::orm`

- [ ] fn `suprnova::render_cache::orm::after_bulk_write` · framework/src/render_cache/orm.rs:251
- [ ] fn `suprnova::render_cache::orm::after_bulk_write_with_handle` · framework/src/render_cache/orm.rs:265
- [ ] fn `suprnova::render_cache::orm::after_model_write` · framework/src/render_cache/orm.rs:173
- [ ] fn `suprnova::render_cache::orm::after_model_write_with_tx` · framework/src/render_cache/orm.rs:205
- [ ] fn `suprnova::render_cache::orm::after_row_write` · framework/src/render_cache/orm.rs:331
- [ ] fn `suprnova::render_cache::orm::after_row_write_with_handle` · framework/src/render_cache/orm.rs:341
- [ ] fn `suprnova::render_cache::orm::after_table_write` · framework/src/render_cache/orm.rs:277
- [ ] fn `suprnova::render_cache::orm::after_unknown_write` · framework/src/render_cache/orm.rs:297

### `suprnova::render_cache::write_side`

- [ ] fn `suprnova::render_cache::write_side::decide` · framework/src/render_cache/write_side.rs:70
- [ ] fn `suprnova::render_cache::write_side::decision` · framework/src/render_cache/write_side.rs:177
- [ ] enum `suprnova::render_cache::write_side::WriteSideDecision` · framework/src/render_cache/write_side.rs:38
  - Variants: `Open`, `Closed`, `Undecided`
