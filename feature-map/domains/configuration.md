# Feature map: `manual/configuration.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 60 checked.

## Rust API: suprnova

### `suprnova::config::env`

- [ ] fn `suprnova::env` · framework/src/config/env.rs:297 (also `suprnova::config::env`, `suprnova::config::env::env`, `suprnova::env::env`)
- [ ] fn `suprnova::env_optional` · framework/src/config/env.rs:396 (also `suprnova::config::env::env_optional`, `suprnova::config::env_optional`, `suprnova::env::env_optional`)
- [ ] fn `suprnova::env_required` · framework/src/config/env.rs:332 (also `suprnova::config::env::env_required`, `suprnova::config::env_required`, `suprnova::env::env_required`)
- [ ] fn `suprnova::config::load_dotenv` · framework/src/config/env.rs:160 (also `suprnova::config::env::load_dotenv`, `suprnova::env::load_dotenv`)
- [ ] fn `suprnova::try_env_required` · framework/src/config/env.rs:369 (also `suprnova::config::env::try_env_required`, `suprnova::config::try_env_required`, `suprnova::env::try_env_required`)
- [ ] enum `suprnova::Environment` · framework/src/config/env.rs:32 (also `suprnova::config::Environment`, `suprnova::config::env::Environment`, `suprnova::env::Environment`)
  - Variants: `Local`, `Development`, `Staging`, `Production`, `Testing`, `Custom`
  - [ ] fn `suprnova::Environment::detect` · framework/src/config/env.rs:71
  - [ ] fn `suprnova::Environment::env_file_suffix` · framework/src/config/env.rs:87
  - [ ] fn `suprnova::Environment::is_production` · framework/src/config/env.rs:99
  - [ ] fn `suprnova::Environment::is_development` · framework/src/config/env.rs:104

### `suprnova::config::providers::app` (private module; items are public through re-exports)

- [ ] struct `suprnova::AppConfig` · framework/src/config/providers/app.rs:8 (also `suprnova::config::AppConfig`, `suprnova::config::providers::AppConfig`)
  - Public fields: `name`, `environment`, `debug`, `url`, `trusted_proxies`
  - [ ] fn `suprnova::AppConfig::from_env` · framework/src/config/providers/app.rs:49
  - [ ] fn `suprnova::AppConfig::try_from_env` · framework/src/config/providers/app.rs:80
  - [ ] fn `suprnova::AppConfig::builder` · framework/src/config/providers/app.rs:99
  - [ ] fn `suprnova::AppConfig::is_debug` · framework/src/config/providers/app.rs:104
  - [ ] fn `suprnova::AppConfig::is_production` · framework/src/config/providers/app.rs:109
  - [ ] fn `suprnova::AppConfig::is_development` · framework/src/config/providers/app.rs:114
- [ ] struct `suprnova::AppConfigBuilder` · framework/src/config/providers/app.rs:187 (also `suprnova::config::AppConfigBuilder`, `suprnova::config::providers::AppConfigBuilder`)
  - [ ] fn `suprnova::AppConfigBuilder::name` · framework/src/config/providers/app.rs:197
  - [ ] fn `suprnova::AppConfigBuilder::environment` · framework/src/config/providers/app.rs:203
  - [ ] fn `suprnova::AppConfigBuilder::debug` · framework/src/config/providers/app.rs:209
  - [ ] fn `suprnova::AppConfigBuilder::url` · framework/src/config/providers/app.rs:215
  - [ ] fn `suprnova::AppConfigBuilder::trusted_proxies` · framework/src/config/providers/app.rs:222
  - [ ] fn `suprnova::AppConfigBuilder::build` · framework/src/config/providers/app.rs:228

### `suprnova::config::providers::server` (private module; items are public through re-exports)

- [ ] struct `suprnova::ServerConfig` · framework/src/config/providers/server.rs:49 (also `suprnova::config::ServerConfig`, `suprnova::config::providers::ServerConfig`)
  - Public fields: `host`, `port`, `max_body_size`, `max_connections`, `header_read_timeout`, `health_readiness_token`
  - [ ] fn `suprnova::ServerConfig::from_env` · framework/src/config/providers/server.rs:129
  - [ ] fn `suprnova::ServerConfig::try_from_env` · framework/src/config/providers/server.rs:152
  - [ ] fn `suprnova::ServerConfig::builder` · framework/src/config/providers/server.rs:194
- [ ] struct `suprnova::ServerConfigBuilder` · framework/src/config/providers/server.rs:301 (also `suprnova::config::ServerConfigBuilder`, `suprnova::config::providers::ServerConfigBuilder`)
  - [ ] fn `suprnova::ServerConfigBuilder::host` · framework/src/config/providers/server.rs:312
  - [ ] fn `suprnova::ServerConfigBuilder::port` · framework/src/config/providers/server.rs:318
  - [ ] fn `suprnova::ServerConfigBuilder::max_body_size` · framework/src/config/providers/server.rs:324
  - [ ] fn `suprnova::ServerConfigBuilder::max_connections` · framework/src/config/providers/server.rs:334
  - [ ] fn `suprnova::ServerConfigBuilder::header_read_timeout` · framework/src/config/providers/server.rs:342
  - [ ] fn `suprnova::ServerConfigBuilder::health_readiness_token` · framework/src/config/providers/server.rs:353
  - [ ] fn `suprnova::ServerConfigBuilder::build` · framework/src/config/providers/server.rs:359
- [ ] const `suprnova::config::providers::DEFAULT_HEADER_READ_TIMEOUT_SECS` · framework/src/config/providers/server.rs:27
- [ ] const `suprnova::config::providers::DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION` · framework/src/config/providers/server.rs:39

### `suprnova::config::repository`

- [ ] fn `suprnova::config::repository::get` · framework/src/config/repository.rs:79
- [ ] fn `suprnova::config::repository::has` · framework/src/config/repository.rs:92
- [ ] fn `suprnova::config::repository::init_repository` · framework/src/config/repository.rs:52
- [ ] fn `suprnova::config::repository::register` · framework/src/config/repository.rs:65
- [ ] struct `suprnova::config::repository::ConfigRepository` · framework/src/config/repository.rs:14
  - [ ] fn `suprnova::config::repository::ConfigRepository::new` · framework/src/config/repository.rs:20
  - [ ] fn `suprnova::config::repository::ConfigRepository::register` · framework/src/config/repository.rs:27
  - [ ] fn `suprnova::config::repository::ConfigRepository::get` · framework/src/config/repository.rs:32
  - [ ] fn `suprnova::config::repository::ConfigRepository::has` · framework/src/config/repository.rs:40

### `suprnova::config::typed`

- [ ] fn `suprnova::config::typed::resolve` · framework/src/config/typed.rs:40
- [ ] fn `suprnova::config::typed::resolve_prefixed` · framework/src/config/typed.rs:52

### `suprnova::config`

- [ ] struct `suprnova::Config` · framework/src/config/mod.rs:42 (also `suprnova::config::Config`)
  - [ ] fn `suprnova::Config::init` · framework/src/config/mod.rs:75
  - [ ] fn `suprnova::Config::get` · framework/src/config/mod.rs:120
  - [ ] fn `suprnova::Config::register` · framework/src/config/mod.rs:145
  - [ ] fn `suprnova::Config::has` · framework/src/config/mod.rs:150
  - [ ] fn `suprnova::Config::environment` · framework/src/config/mod.rs:158
  - [ ] fn `suprnova::Config::is_production` · framework/src/config/mod.rs:165
  - [ ] fn `suprnova::Config::is_development` · framework/src/config/mod.rs:170
  - [ ] fn `suprnova::Config::is_debug` · framework/src/config/mod.rs:185
  - [ ] fn `suprnova::Config::resolve` · framework/src/config/mod.rs:211
  - [ ] fn `suprnova::Config::resolve_prefixed` · framework/src/config/mod.rs:219
