# Feature map: `manual/deployment.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 25 checked.

## Command line

### App runner (`suprnova::Application`; the project binary)

- [ ] command `app serve` · framework/src/app/mod.rs:118
  - Run the web server (default command)
- [ ] command `app web:run` · framework/src/app/mod.rs:124
  - Run the web server (alias for serve)
- [ ] command `app down` · framework/src/app/mod.rs:207
  - Put the application into maintenance mode
- [ ] command `app up` · framework/src/app/mod.rs:234
  - Bring the application out of maintenance mode

## Endpoints and tables

### HTTP endpoints the framework owns

- [ ] endpoint `/_suprnova/health` · framework/src/server.rs:1847
  - any; always, answered by the server before routing; `?db=true` adds a database probe
- [ ] endpoint `/_suprnova/health/live` · framework/src/server.rs:1848
  - any; always; liveness, touches nothing
- [ ] endpoint `/_suprnova/health/ready` · framework/src/server.rs:1849
  - any; always; readiness, probes dependencies; `SERVER_HEALTH_READINESS_TOKEN` gates it

## Rust API: suprnova

### `suprnova::app::maintenance`

- [ ] fn `suprnova::maintenance_mode` · framework/src/app/maintenance.rs:312 (also `suprnova::app::maintenance::maintenance_mode`)
- [ ] struct `suprnova::CacheMaintenanceMode` · framework/src/app/maintenance.rs:263 (also `suprnova::app::maintenance::CacheMaintenanceMode`)
  - Implements: `suprnova::MaintenanceMode`
  - [ ] fn `suprnova::CacheMaintenanceMode::new` · framework/src/app/maintenance.rs:269
  - [ ] fn `suprnova::CacheMaintenanceMode::with_key` · framework/src/app/maintenance.rs:276
- [ ] struct `suprnova::FileMaintenanceMode` · framework/src/app/maintenance.rs:177 (also `suprnova::app::maintenance::FileMaintenanceMode`)
  - Implements: `suprnova::MaintenanceMode`
  - [ ] fn `suprnova::FileMaintenanceMode::new` · framework/src/app/maintenance.rs:183
  - [ ] fn `suprnova::FileMaintenanceMode::with_path` · framework/src/app/maintenance.rs:190
- [ ] struct `suprnova::MaintenanceMiddleware` · framework/src/app/maintenance.rs:395 (also `suprnova::app::maintenance::MaintenanceMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::MaintenanceMiddleware::new` · framework/src/app/maintenance.rs:402
  - [ ] fn `suprnova::MaintenanceMiddleware::with_driver` · framework/src/app/maintenance.rs:410
  - [ ] fn `suprnova::MaintenanceMiddleware::except` · framework/src/app/maintenance.rs:419
- [ ] struct `suprnova::MaintenancePayload` · framework/src/app/maintenance.rs:72 (also `suprnova::app::maintenance::MaintenancePayload`)
  - Public fields: `except`, `redirect`, `retry`, `refresh`, `secret`, `status`, `template`
  - [ ] fn `suprnova::MaintenancePayload::new` · framework/src/app/maintenance.rs:132
- [ ] trait `suprnova::MaintenanceMode` · framework/src/app/maintenance.rs:164 (also `suprnova::app::maintenance::MaintenanceMode`)
  - Implemented here by: `CacheMaintenanceMode`, `FileMaintenanceMode`
  - [ ] fn `suprnova::MaintenanceMode::activate` · framework/src/app/maintenance.rs:166 (required)
  - [ ] fn `suprnova::MaintenanceMode::deactivate` · framework/src/app/maintenance.rs:168 (required)
  - [ ] fn `suprnova::MaintenanceMode::active` · framework/src/app/maintenance.rs:170 (required)
  - [ ] fn `suprnova::MaintenanceMode::data` · framework/src/app/maintenance.rs:172 (required)
