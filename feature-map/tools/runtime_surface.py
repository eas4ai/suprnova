"""Framework-owned HTTP endpoints and database tables.

Neither has a machine-readable source, so both lists are curated from the
registration and migration code. Every entry names a file and a needle
string; the line is looked up at generation time and a missing needle is a
hard error, so an entry cannot silently outlive the code it describes.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1])
REV = sys.argv[2]
OUT = Path(sys.argv[3])

ENDPOINTS = [
    # (path, methods, installed by, file, needle)
    ("/_suprnova/health", "any", "always, answered by the server before routing; `?db=true` adds a database probe",
     "framework/src/server.rs", '"/_suprnova/health" => Some(Self::Legacy)'),
    ("/_suprnova/health/live", "any", "always; liveness, touches nothing",
     "framework/src/server.rs", '"/_suprnova/health/live" => Some(Self::Live)'),
    ("/_suprnova/health/ready", "any", "always; readiness, probes dependencies; `SERVER_HEALTH_READINESS_TOKEN` gates it",
     "framework/src/server.rs", '"/_suprnova/health/ready" => Some(Self::Ready)'),
    ("/_suprnova/lang/{locale}.ftl", "any", "feature `localization`; serves the locale's Fluent catalog",
     "framework/src/server.rs", '.strip_prefix("/_suprnova/lang/")?'),
    ("/__live/action", "GET, POST and the other Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "LIVE_UPDATE_PATH,\n        super::action::handle"),
    ("/__live/upload", "Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "LIVE_UPLOAD_PATH,\n        super::upload::handle"),
    ("/__live/async/subscriptions", "Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "super::async_transport::subscriptions"),
    ("/__live/async/memberships", "Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "super::async_transport::memberships"),
    ("/__live/async/events", "Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "super::async_transport::events"),
    ("/__live/async/socket", "GET (WebSocket, same-origin)", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "LIVE_ASYNC_SOCKET_PATH,\n        std::sync::Arc::new(super::async_transport::AsyncSocketHandler)"),
    ("/__live/assets/{identity}/{file}", "Live methods", "`Router::try_live` / `try_live_with`",
     "framework/src/live/routes.rs", "super::assets::LIVE_ASSET_ROUTE,"),
    ("/__live/assets/{file}", "Live methods", "`Router::try_live` / `try_live_with`; answers a stale asset identity",
     "framework/src/live/routes.rs", "super::assets::LIVE_ASSET_MISS_ROUTE,"),
    ("/suprnova-ui/{component}/{file}", "Live methods", "`Router::try_live_ui_assets` / `try_live_ui_assets_from`",
     "framework/src/live/routes.rs", "super::ui_assets::LIVE_UI_ASSET_ROUTE,"),
    ("/webhooks/payments/{provider}", "POST", "`suprnova::payments::webhook_routes(db)`, merged by the app",
     "framework/src/payments/webhook_route.rs", '.post("/webhooks/payments/{provider}"'),
]

TABLES = [
    # (table, group, file, needle)
    *[(t, "framework migration `CreateRbacTables`", "framework/src/rbac/migrations/m_create_rbac_tables.rs", n) for t, n in (
        ("roles", "enum Roles"), ("permissions", "enum Permissions"), ("role_permissions", "enum RolePermissions"),
        ("model_roles", "enum ModelRoles"), ("model_permissions", "enum ModelPermissions"))],
    ("features", "framework migration `CreateFeaturesTable`", "framework/src/features/migrations/m_create_features_table.rs", "enum Features"),
    ("workflows", "framework migration `CreateWorkflowsTable`", "framework/src/workflow/migrations/m_create_workflows_table.rs", "enum Workflows"),
    ("workflow_steps", "framework migration `CreateWorkflowStepsTable`", "framework/src/workflow/migrations/m_create_workflow_steps_table.rs", "enum WorkflowSteps"),
    ("two_factor_credentials", "framework migration `auth_flows::two_factor::migration::Migration`", "framework/src/auth_flows/two_factor/migration.rs", "TwoFactorCredentials::Table"),
    ("auth_flow_tokens", "framework fn `auth_flows::create_auth_flow_tokens_table`", "framework/src/auth_flows/token_store.rs", "enum AuthFlowTokens"),
    *[(t, "framework migration `CreatePaymentsTables`", "framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs", f'"{t}"') for t in (
        "payments_customers", "payments_payment_methods", "payments_subscriptions", "payments_subscription_items",
        "payments_transactions", "payments_webhook_events")],
    *[(t, "framework migration `render_cache::migration::Migration` / `TierMigration`", "framework/src/render_cache/migration.rs", f'iden = "{t}"') for t in (
        "suprnova_render_generations", "suprnova_render_generation_log", "suprnova_render_epochs",
        "suprnova_render_entries", "suprnova_render_leases", "suprnova_live_instances", "suprnova_live_promotions")],
    *[(t, "Magnetar default schema", "crates/suprnova-magnetar/src/default_schema.rs", f'"{t}"') for t in (
        "app_users", "auth_sessions", "auth_methods", "auth_linked_accounts", "auth_tokens", "auth_ceremonies",
        "auth_lockouts", "auth_two_factor", "auth_remember_tokens", "auth_provider_tokens",
        "auth_lifecycle_deliveries", "auth_migration_runs", "auth_migration_identities", "magnetar_migration_state")],
    *[(t, "`suprnova new` scaffold migration", f"suprnova-cli/src/templates/files/backend/migrations/{f}", n) for t, f, n in (
        ("users", "create_users_table.rs.tpl", "enum Users"),
        ("sessions", "create_sessions_table.rs.tpl", "enum Sessions"),
        ("remember_tokens", "create_remember_tokens_table.rs.tpl", "enum RememberTokens"),
        ("auth_flow_tokens", "create_auth_flow_tokens_table.rs.tpl", 'Alias::new("auth_flow_tokens")'),
        ("workflows", "create_workflows_table.rs.tpl", "enum Workflows"),
        ("workflow_steps", "create_workflow_steps_table.rs.tpl", "enum WorkflowSteps"))],
    ("jobs", "operator-managed; default name of `QUEUE_DB_TABLE` for `QUEUE_DRIVER=database`", "framework/src/queue/mod.rs", 'unwrap_or_else(|_| "jobs".into())'),
    ("failed_jobs", "operator-managed; schema documented on `DatabaseFailedJobStore`", "framework/src/queue/failed.rs", "CREATE TABLE failed_jobs"),
    ("job_batches", "operator-managed; schema documented on `DatabaseBatchRepository`", "framework/src/queue/batch.rs", "CREATE TABLE job_batches"),
    ("job_batch_settlements", "operator-managed; schema documented on `DatabaseBatchRepository`", "framework/src/queue/batch.rs", "CREATE TABLE job_batch_settlements"),
]


def locate(rel, needle):
    text = (ROOT / rel).read_text()
    i = text.find(needle)
    if i == -1:
        raise SystemExit(f"needle not found in {rel}: {needle!r}")
    return text[:i].count("\n") + 1


checked = set()
if OUT.exists():
    for line in OUT.read_text().splitlines():
        m = re.match(r'\s*- \[x\] (?:[a-z ]+ )?`([^`]+)`', line)
        if m:
            checked.add(m.group(1))


def box(k):
    return "x" if k in checked else " "


L = ["# Suprnova feature map: HTTP endpoints and database tables", "",
     f"Source: repository at {REV}. Curated from the route-registration and migration code; "
     "each entry's line is looked up from a code excerpt at generation time.", "",
     "A checked box means the documentation for that item has been remediated against the source.", "",
     "## Counts", "", f"- Framework-owned endpoints: {len(ENDPOINTS)}",
     f"- Tables: {len(TABLES)} entries ({len({t[0] for t in TABLES})} distinct names)", "",
     "## HTTP endpoints the framework owns", "",
     "\"Live methods\" is the fixed set `LIVE_HTTP_METHODS` in `framework/src/live/routes.rs`.", ""]
for path, methods, how, f, needle in ENDPOINTS:
    L.append(f"- [{box(path)}] endpoint `{path}` · {f}:{locate(f, needle)}")
    L.append(f"  - {methods}; {how}")

L += ["", "## Database tables", ""]
group = None
for t, g, f, needle in TABLES:
    if g != group:
        group = g
        L += ["", f"### {g}", ""]
    tag = ("scaffold" if g.startswith("`suprnova new`") else "magnetar" if g.startswith("Magnetar")
           else "operator" if g.startswith("operator") else "framework")
    key = f"{t} ({tag})"
    L.append(f"- [{box(key)}] table `{key}` · {f}:{locate(f, needle)}")

OUT.write_text("\n".join(L) + "\n")
print(json.dumps({"endpoints": len(ENDPOINTS), "tables": len(TABLES), "checked_preserved": len(checked)}))
