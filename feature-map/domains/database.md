# Feature map: `manual/database.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 149 checked.

## Rust API: suprnova

### `suprnova::database::config`

- [ ] struct `suprnova::DatabaseConfig` · framework/src/database/config.rs:70 (also `suprnova::database::DatabaseConfig`, `suprnova::database::config::DatabaseConfig`)
  - Public fields: `url`, `max_connections`, `min_connections`, `connect_timeout`, `logging`, `idle_timeout`, `max_lifetime`, `acquire_timeout`, `test_before_acquire`, `ping_after_idle`, `url_source`
  - [ ] const `suprnova::DatabaseConfig::DEFAULT_SQLITE_URL` · framework/src/database/config.rs:129
  - [ ] fn `suprnova::DatabaseConfig::from_env` · framework/src/database/config.rs:138
  - [ ] fn `suprnova::DatabaseConfig::builder` · framework/src/database/config.rs:159
  - [ ] fn `suprnova::DatabaseConfig::database_type` · framework/src/database/config.rs:164
  - [ ] fn `suprnova::DatabaseConfig::is_configured` · framework/src/database/config.rs:183
  - [ ] fn `suprnova::DatabaseConfig::validate_for_environment` · framework/src/database/config.rs:201
  - [ ] fn `suprnova::DatabaseConfig::validate_pool` · framework/src/database/config.rs:236
- [ ] struct `suprnova::database::DatabaseConfigBuilder` · framework/src/database/config.rs:271 (also `suprnova::database::config::DatabaseConfigBuilder`)
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::url` · framework/src/database/config.rs:286
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::max_connections` · framework/src/database/config.rs:292
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::min_connections` · framework/src/database/config.rs:298
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::connect_timeout` · framework/src/database/config.rs:304
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::logging` · framework/src/database/config.rs:310
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::idle_timeout` · framework/src/database/config.rs:317
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::max_lifetime` · framework/src/database/config.rs:324
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::acquire_timeout` · framework/src/database/config.rs:331
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::test_before_acquire` · framework/src/database/config.rs:337
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::ping_after_idle` · framework/src/database/config.rs:344
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::build` · framework/src/database/config.rs:358
- [ ] enum `suprnova::DatabaseType` · framework/src/database/config.rs:8 (also `suprnova::database::DatabaseType`, `suprnova::database::config::DatabaseType`)
  - Variants: `Postgres`, `Mysql`, `Sqlite`, `Unknown`
- [ ] enum `suprnova::UrlSource` · framework/src/database/config.rs:28 (also `suprnova::database::UrlSource`, `suprnova::database::config::UrlSource`)
  - Variants: `Env`, `Default`, `Explicit`

### `suprnova::database::connection_registry`

- [ ] struct `suprnova::ConnectionRegistry` · framework/src/database/connection_registry.rs:74 (also `suprnova::database::ConnectionRegistry`, `suprnova::database::connection_registry::ConnectionRegistry`)
  - [ ] fn `suprnova::ConnectionRegistry::register` · framework/src/database/connection_registry.rs:116
  - [ ] fn `suprnova::ConnectionRegistry::register_existing` · framework/src/database/connection_registry.rs:134
  - [ ] fn `suprnova::ConnectionRegistry::get` · framework/src/database/connection_registry.rs:146
  - [ ] fn `suprnova::ConnectionRegistry::has` · framework/src/database/connection_registry.rs:168
- [ ] const `suprnova::PRIMARY_CONNECTION_NAME` · framework/src/database/connection_registry.rs:81 (also `suprnova::database::PRIMARY_CONNECTION_NAME`, `suprnova::database::connection_registry::PRIMARY_CONNECTION_NAME`)
- [ ] const `suprnova::READ_REPLICA_CONNECTION_NAME` · framework/src/database/connection_registry.rs:86 (also `suprnova::database::READ_REPLICA_CONNECTION_NAME`, `suprnova::database::connection_registry::READ_REPLICA_CONNECTION_NAME`)

### `suprnova::database::connection`

- [ ] struct `suprnova::DbConnection` · framework/src/database/connection.rs:24 (also `suprnova::database::DbConnection`, `suprnova::database::connection::DbConnection`)
  - [ ] fn `suprnova::DbConnection::connect` · framework/src/database/connection.rs:34
  - [ ] fn `suprnova::DbConnection::from_raw` · framework/src/database/connection.rs:157
  - [ ] fn `suprnova::DbConnection::inner` · framework/src/database/connection.rs:176
  - [ ] fn `suprnova::DbConnection::conn` · framework/src/database/connection.rs:190

### `suprnova::database::db_facade`

- [ ] struct `suprnova::DbTableBuilder` · framework/src/database/db_facade.rs:110 (also `suprnova::database::DbTableBuilder`, `suprnova::database::db_facade::DbTableBuilder`)
  - [ ] fn `suprnova::DbTableBuilder::new` · framework/src/database/db_facade.rs:129
  - [ ] fn `suprnova::DbTableBuilder::on` · framework/src/database/db_facade.rs:145
  - [ ] fn `suprnova::DbTableBuilder::select` · framework/src/database/db_facade.rs:158
  - [ ] fn `suprnova::DbTableBuilder::filter` · framework/src/database/db_facade.rs:168
  - [ ] fn `suprnova::DbTableBuilder::filter_op` · framework/src/database/db_facade.rs:181
  - [ ] fn `suprnova::DbTableBuilder::where_binary` · framework/src/database/db_facade.rs:204
  - [ ] fn `suprnova::DbTableBuilder::where_not_binary` · framework/src/database/db_facade.rs:216
  - [ ] fn `suprnova::DbTableBuilder::order_by_desc` · framework/src/database/db_facade.rs:228
  - [ ] fn `suprnova::DbTableBuilder::order_by_asc` · framework/src/database/db_facade.rs:234
  - [ ] fn `suprnova::DbTableBuilder::limit` · framework/src/database/db_facade.rs:240
  - [ ] fn `suprnova::DbTableBuilder::offset` · framework/src/database/db_facade.rs:246
  - [ ] fn `suprnova::DbTableBuilder::get` · framework/src/database/db_facade.rs:282
  - [ ] fn `suprnova::DbTableBuilder::first` · framework/src/database/db_facade.rs:310
  - [ ] fn `suprnova::DbTableBuilder::count` · framework/src/database/db_facade.rs:325
  - [ ] fn `suprnova::DbTableBuilder::insert` · framework/src/database/db_facade.rs:384
  - [ ] fn `suprnova::DbTableBuilder::update` · framework/src/database/db_facade.rs:506
  - [ ] fn `suprnova::DbTableBuilder::update_all` · framework/src/database/db_facade.rs:551
  - [ ] fn `suprnova::DbTableBuilder::delete` · framework/src/database/db_facade.rs:566
  - [ ] fn `suprnova::DbTableBuilder::delete_all` · framework/src/database/db_facade.rs:600

### `suprnova::database::dynamic_row`

- [ ] struct `suprnova::DynamicRow` · framework/src/database/dynamic_row.rs:59 (also `suprnova::database::DynamicRow`, `suprnova::database::dynamic_row::DynamicRow`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::DynamicRow::from_map` · framework/src/database/dynamic_row.rs:65
  - [ ] fn `suprnova::DynamicRow::into_map` · framework/src/database/dynamic_row.rs:71
  - [ ] fn `suprnova::DynamicRow::get_int` · framework/src/database/dynamic_row.rs:77
  - [ ] fn `suprnova::DynamicRow::get_string` · framework/src/database/dynamic_row.rs:88
  - [ ] fn `suprnova::DynamicRow::get_bool` · framework/src/database/dynamic_row.rs:100
  - [ ] fn `suprnova::DynamicRow::get_float` · framework/src/database/dynamic_row.rs:112
  - [ ] fn `suprnova::DynamicRow::get_value` · framework/src/database/dynamic_row.rs:124
  - [ ] fn `suprnova::DynamicRow::get_as` · framework/src/database/dynamic_row.rs:138
  - [ ] fn `suprnova::DynamicRow::get_optional_string` · framework/src/database/dynamic_row.rs:148
  - [ ] fn `suprnova::DynamicRow::get_optional_int` · framework/src/database/dynamic_row.rs:164

### `suprnova::database::events`

- [ ] struct `suprnova::ConnectionEstablished` · framework/src/database/events.rs:54 (also `suprnova::database::ConnectionEstablished`, `suprnova::database::events::ConnectionEstablished`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::DatabaseBusy` · framework/src/database/events.rs:208 (also `suprnova::database::DatabaseBusy`, `suprnova::database::events::DatabaseBusy`)
  - Public fields: `connection_name`, `connections`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::QueryExecuted` · framework/src/database/events.rs:73 (also `suprnova::database::QueryExecuted`, `suprnova::database::events::QueryExecuted`)
  - Public fields: `sql`, `bindings`, `time`, `connection_name`, `read_write_type`, `result`
  - Implements: `suprnova::Event`
  - [ ] fn `suprnova::QueryExecuted::to_raw_sql` · framework/src/database/events.rs:118
- [ ] struct `suprnova::TransactionBeginning` · framework/src/database/events.rs:162 (also `suprnova::database::TransactionBeginning`, `suprnova::database::events::TransactionBeginning`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TransactionCommitted` · framework/src/database/events.rs:175 (also `suprnova::database::TransactionCommitted`, `suprnova::database::events::TransactionCommitted`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TransactionRolledBack` · framework/src/database/events.rs:193 (also `suprnova::database::TransactionRolledBack`, `suprnova::database::events::TransactionRolledBack`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] enum `suprnova::ReadWriteType` · framework/src/database/events.rs:100 (also `suprnova::database::ReadWriteType`, `suprnova::database::events::ReadWriteType`)
  - Variants: `Read`, `Write`
- [ ] type `suprnova::QueryListener` · framework/src/database/events.rs:270 (also `suprnova::database::QueryListener`, `suprnova::database::events::QueryListener`)

### `suprnova::database::identifier`

- [ ] fn `suprnova::database::validate_identifier` · framework/src/database/identifier.rs:61 (also `suprnova::database::identifier::validate_identifier`)
- [ ] fn `suprnova::database::identifier::validate_savepoint_name` · framework/src/database/identifier.rs:126
- [ ] fn `suprnova::database::validate_sql_operator` · framework/src/database/identifier.rs:200 (also `suprnova::database::identifier::validate_sql_operator`)

### `suprnova::database::model`

- [ ] trait `suprnova::EntityExt` · framework/src/database/model.rs:111 (also `suprnova::database::EntityExt`, `suprnova::database::model::EntityExt`)
  - [ ] fn `suprnova::EntityExt::all` · framework/src/database/model.rs:140 (provided)
  - [ ] fn `suprnova::EntityExt::find_by_pk` · framework/src/database/model.rs:175 (provided)
  - [ ] fn `suprnova::EntityExt::find_or_fail` · framework/src/database/model.rs:214 (provided)
  - [ ] fn `suprnova::EntityExt::count_all` · framework/src/database/model.rs:252 (provided)
  - [ ] fn `suprnova::EntityExt::exists_any` · framework/src/database/model.rs:289 (provided)
  - [ ] fn `suprnova::EntityExt::first` · framework/src/database/model.rs:318 (provided)
- [ ] trait `suprnova::EntityExtMut` · framework/src/database/model.rs:373 (also `suprnova::database::EntityExtMut`, `suprnova::database::model::EntityExtMut`)
  - [ ] fn `suprnova::EntityExtMut::insert_one` · framework/src/database/model.rs:411 (provided)
  - [ ] fn `suprnova::EntityExtMut::update_one` · framework/src/database/model.rs:456 (provided)
  - [ ] fn `suprnova::EntityExtMut::delete_by_pk` · framework/src/database/model.rs:497 (provided)
  - [ ] fn `suprnova::EntityExtMut::save_one` · framework/src/database/model.rs:546 (provided)

### `suprnova::database::query_builder`

- [ ] struct `suprnova::database::QueryBuilder` · framework/src/database/query_builder.rs:84 (also `suprnova::database::query_builder::QueryBuilder`)
  - [ ] fn `suprnova::database::QueryBuilder::new` · framework/src/database/query_builder.rs:97
  - [ ] fn `suprnova::database::QueryBuilder::filter` · framework/src/database/query_builder.rs:126
  - [ ] fn `suprnova::database::QueryBuilder::order_by_asc` · framework/src/database/query_builder.rs:156
  - [ ] fn `suprnova::database::QueryBuilder::order_by_desc` · framework/src/database/query_builder.rs:186
  - [ ] fn `suprnova::database::QueryBuilder::order_by` · framework/src/database/query_builder.rs:217
  - [ ] fn `suprnova::database::QueryBuilder::limit` · framework/src/database/query_builder.rs:243
  - [ ] fn `suprnova::database::QueryBuilder::offset` · framework/src/database/query_builder.rs:268
  - [ ] fn `suprnova::database::QueryBuilder::all` · framework/src/database/query_builder.rs:288
  - [ ] fn `suprnova::database::QueryBuilder::first` · framework/src/database/query_builder.rs:322
  - [ ] fn `suprnova::database::QueryBuilder::first_or_fail` · framework/src/database/query_builder.rs:356
  - [ ] fn `suprnova::database::QueryBuilder::count` · framework/src/database/query_builder.rs:385
  - [ ] fn `suprnova::database::QueryBuilder::exists` · framework/src/database/query_builder.rs:416
  - [ ] fn `suprnova::database::QueryBuilder::into_select` · framework/src/database/query_builder.rs:453

### `suprnova::database::transaction`

- [ ] struct `suprnova::Transaction` · framework/src/database/transaction.rs:301 (also `suprnova::database::Transaction`, `suprnova::database::transaction::Transaction`)
  - [ ] fn `suprnova::Transaction::backend` · framework/src/database/transaction.rs:1014
  - [ ] fn `suprnova::Transaction::query_all` · framework/src/database/transaction.rs:1022
  - [ ] fn `suprnova::Transaction::handle` · framework/src/database/transaction.rs:1033
  - [ ] fn `suprnova::Transaction::savepoint` · framework/src/database/transaction.rs:1061
  - [ ] fn `suprnova::Transaction::rollback_to` · framework/src/database/transaction.rs:1107
  - [ ] fn `suprnova::Transaction::commit` · framework/src/database/transaction.rs:1148
  - [ ] fn `suprnova::Transaction::rollback` · framework/src/database/transaction.rs:1172
- [ ] struct `suprnova::TxHandle` · framework/src/database/transaction.rs:330 (also `suprnova::database::TxHandle`, `suprnova::database::transaction::TxHandle`)

### `suprnova::database`

- [ ] struct `suprnova::DB` · framework/src/database/mod.rs:181 (also `suprnova::database::DB`)
  - [ ] fn `suprnova::DB::table` · framework/src/database/db_facade.rs:738
  - [ ] fn `suprnova::DB::select` · framework/src/database/db_facade.rs:763
  - [ ] fn `suprnova::DB::select_one` · framework/src/database/db_facade.rs:787
  - [ ] fn `suprnova::DB::scalar` · framework/src/database/db_facade.rs:848
  - [ ] fn `suprnova::DB::insert` · framework/src/database/db_facade.rs:904
  - [ ] fn `suprnova::DB::update` · framework/src/database/db_facade.rs:914
  - [ ] fn `suprnova::DB::delete` · framework/src/database/db_facade.rs:923
  - [ ] fn `suprnova::DB::statement` · framework/src/database/db_facade.rs:946
  - [ ] fn `suprnova::DB::unprepared` · framework/src/database/db_facade.rs:974
  - [ ] fn `suprnova::DB::affecting_statement` · framework/src/database/db_facade.rs:1031
  - [ ] fn `suprnova::DB::table_on` · framework/src/database/db_facade.rs:1085
  - [ ] fn `suprnova::DB::select_on` · framework/src/database/db_facade.rs:1095
  - [ ] fn `suprnova::DB::statement_on` · framework/src/database/db_facade.rs:1125
  - [ ] fn `suprnova::DB::affecting_statement_on` · framework/src/database/db_facade.rs:1154
  - [ ] fn `suprnova::DB::listen` · framework/src/database/db_facade.rs:1205
  - [ ] fn `suprnova::DB::flush_listeners` · framework/src/database/db_facade.rs:1219
  - [ ] fn `suprnova::DB::enable_query_log` · framework/src/database/db_facade.rs:1233
  - [ ] fn `suprnova::DB::disable_query_log` · framework/src/database/db_facade.rs:1244
  - [ ] fn `suprnova::DB::logging` · framework/src/database/db_facade.rs:1254
  - [ ] fn `suprnova::DB::get_query_log` · framework/src/database/db_facade.rs:1265
  - [ ] fn `suprnova::DB::flush_query_log` · framework/src/database/db_facade.rs:1275
  - [ ] fn `suprnova::DB::database_name` · framework/src/database/db_facade.rs:1293
  - [ ] fn `suprnova::DB::driver_name` · framework/src/database/db_facade.rs:1305
  - [ ] fn `suprnova::DB::driver_title` · framework/src/database/db_facade.rs:1322
  - [ ] fn `suprnova::DB::server_version` · framework/src/database/db_facade.rs:1343
  - [ ] fn `suprnova::DB::transaction` · framework/src/database/transaction.rs:1233
  - [ ] fn `suprnova::DB::begin_transaction` · framework/src/database/transaction.rs:1437
  - [ ] fn `suprnova::DB::transaction_with_attempts` · framework/src/database/transaction.rs:1487
  - [ ] fn `suprnova::DB::init` · framework/src/database/mod.rs:204
  - [ ] fn `suprnova::DB::init_with` · framework/src/database/mod.rs:237
  - [ ] fn `suprnova::DB::connection` · framework/src/database/mod.rs:281
  - [ ] fn `suprnova::DB::is_connected` · framework/src/database/mod.rs:298
  - [ ] fn `suprnova::DB::get` · framework/src/database/mod.rs:325
  - [ ] fn `suprnova::DB::register_named` · framework/src/database/mod.rs:346
  - [ ] fn `suprnova::DB::named` · framework/src/database/mod.rs:353
- [ ] type `suprnova::Database` · framework/src/database/mod.rs:135 (also `suprnova::database::Database`)
