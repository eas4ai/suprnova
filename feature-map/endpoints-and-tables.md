# Suprnova feature map: HTTP endpoints and database tables

Source: repository at d03b4f1. Curated from the route-registration and migration code; each entry's line is looked up from a code excerpt at generation time.

A checked box means the documentation for that item has been remediated against the source.

## Counts

- Framework-owned endpoints: 14
- Tables: 47 entries (44 distinct names)

## HTTP endpoints the framework owns

"Live methods" is the fixed set `LIVE_HTTP_METHODS` in `framework/src/live/routes.rs`.

- [ ] endpoint `/_suprnova/health` · framework/src/server.rs:1847
  - any; always, answered by the server before routing; `?db=true` adds a database probe
- [ ] endpoint `/_suprnova/health/live` · framework/src/server.rs:1848
  - any; always; liveness, touches nothing
- [ ] endpoint `/_suprnova/health/ready` · framework/src/server.rs:1849
  - any; always; readiness, probes dependencies; `SERVER_HEALTH_READINESS_TOKEN` gates it
- [ ] endpoint `/_suprnova/lang/{locale}.ftl` · framework/src/server.rs:2007
  - any; feature `localization`; serves the locale's Fluent catalog
- [ ] endpoint `/__live/action` · framework/src/live/routes.rs:157
  - GET, POST and the other Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/upload` · framework/src/live/routes.rs:169
  - Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/async/subscriptions` · framework/src/live/routes.rs:182
  - Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/async/memberships` · framework/src/live/routes.rs:187
  - Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/async/events` · framework/src/live/routes.rs:192
  - Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/async/socket` · framework/src/live/routes.rs:208
  - GET (WebSocket, same-origin); `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/assets/{identity}/{file}` · framework/src/live/routes.rs:224
  - Live methods; `Router::try_live` / `try_live_with`
- [ ] endpoint `/__live/assets/{file}` · framework/src/live/routes.rs:231
  - Live methods; `Router::try_live` / `try_live_with`; answers a stale asset identity
- [ ] endpoint `/suprnova-ui/{component}/{file}` · framework/src/live/routes.rs:122
  - Live methods; `Router::try_live_ui_assets` / `try_live_ui_assets_from`
- [ ] endpoint `/webhooks/payments/{provider}` · framework/src/payments/webhook_route.rs:1003
  - POST; `suprnova::payments::webhook_routes(db)`, merged by the app

## Database tables


### framework migration `CreateRbacTables`

- [ ] table `roles (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:15
- [ ] table `permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:26
- [ ] table `role_permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:37
- [ ] table `model_roles (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:45
- [ ] table `model_permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:54

### framework migration `CreateFeaturesTable`

- [ ] table `features (framework)` · framework/src/features/migrations/m_create_features_table.rs:44

### framework migration `CreateWorkflowsTable`

- [ ] table `workflows (framework)` · framework/src/workflow/migrations/m_create_workflows_table.rs:109

### framework migration `CreateWorkflowStepsTable`

- [ ] table `workflow_steps (framework)` · framework/src/workflow/migrations/m_create_workflow_steps_table.rs:105

### framework migration `auth_flows::two_factor::migration::Migration`

- [ ] table `two_factor_credentials (framework)` · framework/src/auth_flows/two_factor/migration.rs:42

### framework fn `auth_flows::create_auth_flow_tokens_table`

- [ ] table `auth_flow_tokens (framework)` · framework/src/auth_flows/token_store.rs:119

### framework migration `CreatePaymentsTables`

- [ ] table `payments_customers (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:32
- [ ] table `payments_payment_methods (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:104
- [ ] table `payments_subscriptions (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:187
- [ ] table `payments_subscription_items (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:289
- [ ] table `payments_transactions (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:385
- [ ] table `payments_webhook_events (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:501

### framework migration `render_cache::migration::Migration` / `TierMigration`

- [ ] table `suprnova_render_generations (framework)` · framework/src/render_cache/migration.rs:168
- [ ] table `suprnova_render_generation_log (framework)` · framework/src/render_cache/migration.rs:178
- [ ] table `suprnova_render_epochs (framework)` · framework/src/render_cache/migration.rs:189
- [ ] table `suprnova_render_entries (framework)` · framework/src/render_cache/migration.rs:485
- [ ] table `suprnova_render_leases (framework)` · framework/src/render_cache/migration.rs:498
- [ ] table `suprnova_live_instances (framework)` · framework/src/render_cache/migration.rs:509
- [ ] table `suprnova_live_promotions (framework)` · framework/src/render_cache/migration.rs:520

### Magnetar default schema

- [ ] table `app_users (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] table `auth_sessions (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] table `auth_methods (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] table `auth_linked_accounts (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] table `auth_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] table `auth_ceremonies (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] table `auth_lockouts (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] table `auth_two_factor (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] table `auth_remember_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] table `auth_provider_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] table `auth_lifecycle_deliveries (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] table `auth_migration_runs (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] table `auth_migration_identities (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] table `magnetar_migration_state (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:154

### `suprnova new` scaffold migration

- [ ] table `users (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_users_table.rs.tpl:60
- [ ] table `sessions (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_sessions_table.rs.tpl:59
- [ ] table `remember_tokens (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_remember_tokens_table.rs.tpl:82
- [ ] table `auth_flow_tokens (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_auth_flow_tokens_table.rs.tpl:35
- [ ] table `workflows (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_workflows_table.rs.tpl:88
- [ ] table `workflow_steps (scaffold)` · suprnova-cli/src/templates/files/backend/migrations/create_workflow_steps_table.rs.tpl:78

### operator-managed; default name of `QUEUE_DB_TABLE` for `QUEUE_DRIVER=database`

- [ ] table `jobs (operator)` · framework/src/queue/mod.rs:1355

### operator-managed; schema documented on `DatabaseFailedJobStore`

- [ ] table `failed_jobs (operator)` · framework/src/queue/failed.rs:241

### operator-managed; schema documented on `DatabaseBatchRepository`

- [ ] table `job_batches (operator)` · framework/src/queue/batch.rs:410
- [ ] table `job_batch_settlements (operator)` · framework/src/queue/batch.rs:420
