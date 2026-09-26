# Feature map: `manual/eloquent.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 262 checked.

## Command line

### Console binary (framework-registered commands)

- [ ] command `console model:prune` · framework/src/eloquent/console/prune.rs:25
  - Prune stale rows for every Prunable / MassPrunable model.

## Rust API: suprnova

### Re-exported from other crates

- [ ] trait `suprnova::ActiveModelBehavior` re-exports `sea_orm::entity::active_model::ActiveModelBehavior`
- [ ] trait `suprnova::ActiveModelTrait` re-exports `sea_orm::entity::active_model::ActiveModelTrait`
- [ ] enum `suprnova::ActiveValue` re-exports `sea_orm::entity::active_value::ActiveValue`
- [ ] trait `suprnova::ColumnTrait` re-exports `sea_orm::entity::column::ColumnTrait`
- [ ] trait `suprnova::ConnectionTrait` re-exports `sea_orm::database::connection::ConnectionTrait`
- [ ] struct `suprnova::DatabaseConnection` re-exports `sea_orm::database::db_connection::DatabaseConnection`
- [ ] struct `suprnova::DatabaseTransaction` re-exports `sea_orm::database::transaction::DatabaseTransaction`
- [ ] enum `suprnova::DbErr` re-exports `sea_orm::error::DbErr`
- [ ] proc derive `suprnova::DeriveActiveEnum` re-exports `sea_orm_macros::DeriveActiveEnum`
- [ ] trait `suprnova::EntityName` re-exports `sea_orm::entity::base_entity::EntityName`
- [ ] trait `suprnova::EntityTrait` re-exports `sea_orm::entity::base_entity::EntityTrait`
- [ ] trait `suprnova::IntoActiveModel` re-exports `sea_orm::entity::active_model::IntoActiveModel`
- [ ] trait `suprnova::ModelTrait` re-exports `sea_orm::entity::model::ModelTrait`
- [ ] variant `suprnova::NotSet` re-exports `sea_orm::entity::active_value::ActiveValue::NotSet`
- [ ] trait `suprnova::PrimaryKeyToColumn` re-exports `sea_orm::entity::primary_key::PrimaryKeyToColumn`
- [ ] trait `suprnova::PrimaryKeyTrait` re-exports `sea_orm::entity::primary_key::PrimaryKeyTrait`
- [ ] trait `suprnova::QueryFilter` re-exports `sea_orm::query::helper::QueryFilter`
- [ ] trait `suprnova::QueryOrder` re-exports `sea_orm::query::helper::QueryOrder`
- [ ] trait `suprnova::QuerySelect` re-exports `sea_orm::query::helper::QuerySelect`
- [ ] struct `suprnova::RelationDef` re-exports `sea_orm::entity::relation::RelationDef`
- [ ] trait `suprnova::RelationTrait` re-exports `sea_orm::entity::relation::RelationTrait`
- [ ] struct `suprnova::Schema` re-exports `sea_orm::schema::Schema`
- [ ] struct `suprnova::Select` re-exports `sea_orm::query::select::Select`
- [ ] variant `suprnova::Set` re-exports `sea_orm::entity::active_value::ActiveValue::Set`
- [ ] enum `suprnova::SqlErr` re-exports `sea_orm::error::SqlErr`
- [ ] trait `suprnova::TransactionTrait` re-exports `sea_orm::database::connection::TransactionTrait`
- [ ] trait `suprnova::TryGetable` re-exports `sea_orm::executor::query::TryGetable`
- [ ] module `suprnova::database::sea_orm` re-exports `sea_orm`
- [ ] module `suprnova::sea_orm` re-exports `sea_orm`

### `suprnova::eloquent::attrs`

- [ ] struct `suprnova::Attrs` · framework/src/eloquent/attrs.rs:28 (also `suprnova::eloquent::Attrs`, `suprnova::eloquent::attrs::Attrs`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Attrs::new` · framework/src/eloquent/attrs.rs:32
  - [ ] fn `suprnova::Attrs::insert` · framework/src/eloquent/attrs.rs:40
  - [ ] fn `suprnova::Attrs::get` · framework/src/eloquent/attrs.rs:46
  - [ ] fn `suprnova::Attrs::keys` · framework/src/eloquent/attrs.rs:51
  - [ ] fn `suprnova::Attrs::iter` · framework/src/eloquent/attrs.rs:56
  - [ ] fn `suprnova::Attrs::len` · framework/src/eloquent/attrs.rs:61
  - [ ] fn `suprnova::Attrs::is_empty` · framework/src/eloquent/attrs.rs:66
  - [ ] fn `suprnova::Attrs::contains_key` · framework/src/eloquent/attrs.rs:71
  - [ ] fn `suprnova::Attrs::merge` · framework/src/eloquent/attrs.rs:78

### `suprnova::eloquent::console::prune`

- [ ] struct `suprnova::eloquent::console::prune::PruneArgs` · framework/src/eloquent/console/prune.rs:28
  - Public fields: `model`, `pretend`
  - Implements: `suprnova::TypedCommand`

### `suprnova::eloquent::events`

- [ ] fn `suprnova::dispatch_after` · framework/src/eloquent/events.rs:175 (also `suprnova::eloquent::events::dispatch_after`)
- [ ] fn `suprnova::dispatch_cancellable` · framework/src/eloquent/events.rs:145 (also `suprnova::eloquent::events::dispatch_cancellable`)
- [ ] fn `suprnova::listen_cancellable` · framework/src/eloquent/events.rs:214 (also `suprnova::eloquent::events::listen_cancellable`)
- [ ] enum `suprnova::EventResult` · framework/src/eloquent/events.rs:45 (also `suprnova::eloquent::events::EventResult`)
  - Variants: `Ok`, `Cancel`
  - [ ] fn `suprnova::EventResult::ok` · framework/src/eloquent/events.rs:55
  - [ ] fn `suprnova::EventResult::cancel` · framework/src/eloquent/events.rs:60
  - [ ] fn `suprnova::EventResult::is_cancelled` · framework/src/eloquent/events.rs:65
- [ ] trait `suprnova::CancellableListener` · framework/src/eloquent/events.rs:76 (also `suprnova::eloquent::events::CancellableListener`)
  - [ ] fn `suprnova::CancellableListener::handle` · framework/src/eloquent/events.rs:79 (required)
- [ ] trait `suprnova::ModelEventHooks` · framework/src/eloquent/events.rs:286 (also `suprnova::eloquent::events::ModelEventHooks`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_creating` · framework/src/eloquent/events.rs:288 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_saving` · framework/src/eloquent/events.rs:292 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_created` · framework/src/eloquent/events.rs:297 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_saved` · framework/src/eloquent/events.rs:299 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_updating` · framework/src/eloquent/events.rs:301 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_updated` · framework/src/eloquent/events.rs:306 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_deleting` · framework/src/eloquent/events.rs:308 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_deleted` · framework/src/eloquent/events.rs:310 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_trashed` · framework/src/eloquent/events.rs:312 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_restoring` · framework/src/eloquent/events.rs:314 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_restored` · framework/src/eloquent/events.rs:316 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_force_deleting` · framework/src/eloquent/events.rs:318 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_force_deleted` · framework/src/eloquent/events.rs:320 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_replicating` · framework/src/eloquent/events.rs:322 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_retrieving` · framework/src/eloquent/events.rs:327 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_retrieved` · framework/src/eloquent/events.rs:329 (required)

### `suprnova::eloquent::fillable`

- [ ] fn `suprnova::prevent_silently_discarding_attributes` · framework/src/eloquent/fillable.rs:66 (also `suprnova::eloquent::fillable::prevent_silently_discarding_attributes`, `suprnova::eloquent::prevent_silently_discarding_attributes`)
- [ ] fn `suprnova::preventing_silently_discarding_attributes` · framework/src/eloquent/fillable.rs:72 (also `suprnova::eloquent::fillable::preventing_silently_discarding_attributes`, `suprnova::eloquent::preventing_silently_discarding_attributes`)
- [ ] fn `suprnova::unguarded` · framework/src/eloquent/fillable.rs:262 (also `suprnova::eloquent::fillable::unguarded`, `suprnova::eloquent::unguarded`)
- [ ] struct `suprnova::Fillable` · framework/src/eloquent/fillable.rs:79 (also `suprnova::eloquent::Fillable`, `suprnova::eloquent::fillable::Fillable`)
  - [ ] fn `suprnova::Fillable::allow_all` · framework/src/eloquent/fillable.rs:98
  - [ ] fn `suprnova::Fillable::guarded_default` · framework/src/eloquent/fillable.rs:109
  - [ ] fn `suprnova::Fillable::fillable` · framework/src/eloquent/fillable.rs:120
  - [ ] fn `suprnova::Fillable::guarded` · framework/src/eloquent/fillable.rs:127
  - [ ] fn `suprnova::Fillable::apply` · framework/src/eloquent/fillable.rs:141
  - [ ] fn `suprnova::Fillable::apply_checked` · framework/src/eloquent/fillable.rs:159

### `suprnova::eloquent::model`

- [ ] fn `suprnova::eloquent::model::json_value_to_sea_value` · framework/src/eloquent/model.rs:1469
- [ ] fn `suprnova::eloquent::model::sea_value_to_json_loose` · framework/src/eloquent/model.rs:1501
- [ ] trait `suprnova::FirstOrCreate` · framework/src/eloquent/model.rs:1567 (also `suprnova::eloquent::FirstOrCreate`, `suprnova::eloquent::model::FirstOrCreate`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::FirstOrCreate::first_or_create` · framework/src/eloquent/model.rs:1582 (provided)
  - [ ] fn `suprnova::FirstOrCreate::update_or_create` · framework/src/eloquent/model.rs:1593 (provided)
  - [ ] fn `suprnova::FirstOrCreate::first_or_new` · framework/src/eloquent/model.rs:1603 (provided)
  - [ ] fn `suprnova::FirstOrCreate::first_or` · framework/src/eloquent/model.rs:1612 (provided)
  - [ ] fn `suprnova::FirstOrCreate::find_or` · framework/src/eloquent/model.rs:1625 (provided)
  - [ ] fn `suprnova::FirstOrCreate::find_or_new` · framework/src/eloquent/model.rs:1641 (provided)
  - [ ] fn `suprnova::FirstOrCreate::create_or_first` · framework/src/eloquent/model.rs:1661 (provided)
  - [ ] fn `suprnova::FirstOrCreate::from_attrs_unsaved` · framework/src/eloquent/model.rs:1677 (required)
- [ ] trait `suprnova::Model` · framework/src/eloquent/model.rs:68 (also `suprnova::eloquent::Model`, `suprnova::eloquent::model::Model`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::Model::primary_key_name` · framework/src/eloquent/model.rs:91 (provided)
  - [ ] fn `suprnova::Model::qualified_key_name` · framework/src/eloquent/model.rs:103 (provided)
  - [ ] fn `suprnova::Model::fillable_filter` · framework/src/eloquent/model.rs:110 (required)
  - [ ] fn `suprnova::Model::try_from_storage` · framework/src/eloquent/model.rs:132 (provided)
  - [ ] fn `suprnova::Model::try_into_storage` · framework/src/eloquent/model.rs:147 (provided)
  - [ ] fn `suprnova::Model::field_value` · framework/src/eloquent/model.rs:167 (provided)
  - [ ] fn `suprnova::Model::to_array` · framework/src/eloquent/model.rs:188 (provided)
  - [ ] fn `suprnova::Model::to_json` · framework/src/eloquent/model.rs:200 (provided)
  - [ ] fn `suprnova::Model::find` · framework/src/eloquent/model.rs:227 (provided)
  - [ ] fn `suprnova::Model::find_or_fail` · framework/src/eloquent/model.rs:285 (provided)
  - [ ] fn `suprnova::Model::find_many` · framework/src/eloquent/model.rs:309 (provided)
  - [ ] fn `suprnova::Model::all` · framework/src/eloquent/model.rs:391 (provided)
  - [ ] fn `suprnova::Model::query` · framework/src/eloquent/model.rs:427 (provided)
  - [ ] fn `suprnova::Model::create` · framework/src/eloquent/model.rs:449 (provided)
  - [ ] fn `suprnova::Model::save` · framework/src/eloquent/model.rs:507 (provided)
  - [ ] fn `suprnova::Model::update` · framework/src/eloquent/model.rs:561 (provided)
  - [ ] fn `suprnova::Model::delete` · framework/src/eloquent/model.rs:611 (provided)
  - [ ] fn `suprnova::Model::force_delete` · framework/src/eloquent/model.rs:643 (provided)
  - [ ] fn `suprnova::Model::touch_owners` · framework/src/eloquent/model.rs:683 (provided)
  - [ ] fn `suprnova::Model::touch_owners_with_tx` · framework/src/eloquent/model.rs:709 (provided)
  - [ ] fn `suprnova::Model::save_with_tx` · framework/src/eloquent/model.rs:852 (provided)
  - [ ] fn `suprnova::Model::update_with_tx` · framework/src/eloquent/model.rs:887 (provided)
  - [ ] fn `suprnova::Model::delete_with_tx` · framework/src/eloquent/model.rs:923 (provided)
  - [ ] fn `suprnova::Model::create_with_tx` · framework/src/eloquent/model.rs:948 (provided)
  - [ ] fn `suprnova::Model::force_delete_with_tx` · framework/src/eloquent/model.rs:976 (provided)
  - [ ] fn `suprnova::Model::refresh` · framework/src/eloquent/model.rs:1000 (provided)
  - [ ] fn `suprnova::Model::refresh_for_update` · framework/src/eloquent/model.rs:1042 (provided)
  - [ ] fn `suprnova::Model::fresh` · framework/src/eloquent/model.rs:1059 (provided)
  - [ ] fn `suprnova::Model::replicate` · framework/src/eloquent/model.rs:1086 (provided)
  - [ ] fn `suprnova::Model::replicate_except` · framework/src/eloquent/model.rs:1104 (provided)
  - [ ] fn `suprnova::Model::replicate_into` · framework/src/eloquent/model.rs:1148 (provided)
  - [ ] fn `suprnova::Model::increment` · framework/src/eloquent/model.rs:1186 (provided)
  - [ ] fn `suprnova::Model::decrement` · framework/src/eloquent/model.rs:1223 (provided)
  - [ ] fn `suprnova::Model::destroy` · framework/src/eloquent/model.rs:1237 (provided)
  - [ ] fn `suprnova::Model::force_destroy` · framework/src/eloquent/model.rs:1256 (provided)
  - [ ] fn `suprnova::Model::is` · framework/src/eloquent/model.rs:1275 (provided)
  - [ ] fn `suprnova::Model::is_not` · framework/src/eloquent/model.rs:1280 (provided)
  - [ ] fn `suprnova::Model::to_array_except` · framework/src/eloquent/model.rs:1293 (provided)
  - [ ] fn `suprnova::Model::to_array_only` · framework/src/eloquent/model.rs:1307 (provided)
  - [ ] fn `suprnova::Model::save_quietly` · framework/src/eloquent/model.rs:1323 (provided)
  - [ ] fn `suprnova::Model::update_quietly` · framework/src/eloquent/model.rs:1329 (provided)
  - [ ] fn `suprnova::Model::delete_quietly` · framework/src/eloquent/model.rs:1334 (provided)
  - [ ] fn `suprnova::Model::force_delete_quietly` · framework/src/eloquent/model.rs:1339 (provided)
  - [ ] fn `suprnova::Model::update_or_fail` · framework/src/eloquent/model.rs:1361 (provided)
  - [ ] fn `suprnova::Model::delete_or_fail` · framework/src/eloquent/model.rs:1412 (provided)
  - [ ] fn `suprnova::Model::primary_key_value` · framework/src/eloquent/model.rs:1431 (required)
  - [ ] fn `suprnova::Model::primary_key_value_json` · framework/src/eloquent/model.rs:1438 (required)
  - [ ] fn `suprnova::Model::reset_primary_key` · framework/src/eloquent/model.rs:1442 (required)
  - [ ] fn `suprnova::Model::active_model_from_attrs` · framework/src/eloquent/model.rs:1446 (required)
  - [ ] fn `suprnova::Model::apply_attrs_to_active_model` · framework/src/eloquent/model.rs:1453 (required)
  - [ ] fn `suprnova::Model::into_active_model_for_update` · framework/src/eloquent/model.rs:1461 (required)
- [ ] trait `suprnova::ReplicateExt` · framework/src/eloquent/model.rs:1550 (also `suprnova::eloquent::ReplicateExt`, `suprnova::eloquent::model::ReplicateExt`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::ReplicateExt::replicate_with` · framework/src/eloquent/model.rs:1553 (required)

### `suprnova::eloquent::observers`

- [ ] fn `suprnova::bootstrap_observers` · framework/src/eloquent/observers.rs:311 (also `suprnova::eloquent::observers::bootstrap_observers`)
- [ ] struct `suprnova::ObserverEntry` · framework/src/eloquent/observers.rs:277 (also `suprnova::eloquent::observers::ObserverEntry`)
  - Public fields: `name`, `install`
- [ ] trait `suprnova::Observer` · framework/src/eloquent/observers.rs:130 (also `suprnova::eloquent::observers::Observer`)
  - [ ] fn `suprnova::Observer::retrieving` · framework/src/eloquent/observers.rs:147 (provided)
  - [ ] fn `suprnova::Observer::retrieved` · framework/src/eloquent/observers.rs:152 (provided)
  - [ ] fn `suprnova::Observer::saving` · framework/src/eloquent/observers.rs:161 (provided)
  - [ ] fn `suprnova::Observer::creating` · framework/src/eloquent/observers.rs:168 (provided)
  - [ ] fn `suprnova::Observer::updating` · framework/src/eloquent/observers.rs:175 (provided)
  - [ ] fn `suprnova::Observer::deleting` · framework/src/eloquent/observers.rs:182 (provided)
  - [ ] fn `suprnova::Observer::restoring` · framework/src/eloquent/observers.rs:188 (provided)
  - [ ] fn `suprnova::Observer::created` · framework/src/eloquent/observers.rs:195 (provided)
  - [ ] fn `suprnova::Observer::updated` · framework/src/eloquent/observers.rs:201 (provided)
  - [ ] fn `suprnova::Observer::saved` · framework/src/eloquent/observers.rs:206 (provided)
  - [ ] fn `suprnova::Observer::deleted` · framework/src/eloquent/observers.rs:213 (provided)
  - [ ] fn `suprnova::Observer::trashed` · framework/src/eloquent/observers.rs:220 (provided)
  - [ ] fn `suprnova::Observer::restored` · framework/src/eloquent/observers.rs:225 (provided)
  - [ ] fn `suprnova::Observer::replicating` · framework/src/eloquent/observers.rs:233 (provided)
  - [ ] fn `suprnova::Observer::force_deleting` · framework/src/eloquent/observers.rs:242 (provided)
  - [ ] fn `suprnova::Observer::force_deleted` · framework/src/eloquent/observers.rs:247 (provided)
- [ ] type `suprnova::ObserverInstallFuture` · framework/src/eloquent/observers.rs:258 (also `suprnova::eloquent::observers::ObserverInstallFuture`)

### `suprnova::eloquent::prunable`

- [ ] fn `suprnova::prune_all` · framework/src/eloquent/prunable.rs:154 (also `suprnova::eloquent::prunable::prune_all`, `suprnova::eloquent::prune_all`)
- [ ] fn `suprnova::prune_all_dry` · framework/src/eloquent/prunable.rs:164 (also `suprnova::eloquent::prunable::prune_all_dry`, `suprnova::eloquent::prune_all_dry`)
- [ ] fn `suprnova::prune_one` · framework/src/eloquent/prunable.rs:185 (also `suprnova::eloquent::prunable::prune_one`, `suprnova::eloquent::prune_one`)
- [ ] fn `suprnova::eloquent::pruners` · framework/src/eloquent/prunable.rs:147 (also `suprnova::eloquent::prunable::pruners`)
- [ ] struct `suprnova::PrunerEntry` · framework/src/eloquent/prunable.rs:132 (also `suprnova::eloquent::PrunerEntry`, `suprnova::eloquent::prunable::PrunerEntry`)
  - Public fields: `type_name`, `run`
- [ ] trait `suprnova::MassPrunable` · framework/src/eloquent/prunable.rs:102 (also `suprnova::eloquent::MassPrunable`, `suprnova::eloquent::prunable::MassPrunable`)
  - [ ] fn `suprnova::MassPrunable::prunable` · framework/src/eloquent/prunable.rs:119 (required)
- [ ] trait `suprnova::Prunable` · framework/src/eloquent/prunable.rs:69 (also `suprnova::eloquent::Prunable`, `suprnova::eloquent::prunable::Prunable`)
  - [ ] fn `suprnova::Prunable::prunable` · framework/src/eloquent/prunable.rs:83 (required)
  - [ ] fn `suprnova::Prunable::pruning` · framework/src/eloquent/prunable.rs:89 (provided)
- [ ] type `suprnova::eloquent::PrunerFn` · framework/src/eloquent/prunable.rs:126 (also `suprnova::eloquent::prunable::PrunerFn`)

### `suprnova::eloquent::registry`

- [ ] fn `suprnova::find_model_by_table` · framework/src/eloquent/registry.rs:46 (also `suprnova::eloquent::find_model_by_table`, `suprnova::eloquent::registry::find_model_by_table`)
- [ ] fn `suprnova::models` · framework/src/eloquent/registry.rs:29 (also `suprnova::eloquent::models`, `suprnova::eloquent::registry::models`)
- [ ] struct `suprnova::ModelEntry` · framework/src/eloquent/registry.rs:14 (also `suprnova::eloquent::ModelEntry`, `suprnova::eloquent::registry::ModelEntry`)
  - Public fields: `type_name`, `table`, `module_path`, `primary_key`

### `suprnova::eloquent::soft_deletes`

- [ ] trait `suprnova::SoftDeletes` · framework/src/eloquent/soft_deletes.rs:42 (also `suprnova::eloquent::SoftDeletes`, `suprnova::eloquent::soft_deletes::SoftDeletes`)
  - [ ] fn `suprnova::SoftDeletes::deleted_at_column` · framework/src/eloquent/soft_deletes.rs:57 (required)
  - [ ] fn `suprnova::SoftDeletes::is_trashed` · framework/src/eloquent/soft_deletes.rs:62 (required)

### `suprnova::eloquent::timestamps`

- [ ] fn `suprnova::eloquent::touches_disabled` · framework/src/eloquent/timestamps.rs:49 (also `suprnova::eloquent::timestamps::touches_disabled`)
- [ ] fn `suprnova::eloquent::touches_ignored_for` · framework/src/eloquent/timestamps.rs:130 (also `suprnova::eloquent::timestamps::touches_ignored_for`)
- [ ] fn `suprnova::eloquent::without_touching` · framework/src/eloquent/timestamps.rs:78 (also `suprnova::eloquent::timestamps::without_touching`)
- [ ] fn `suprnova::eloquent::without_touching_on` · framework/src/eloquent/timestamps.rs:107 (also `suprnova::eloquent::timestamps::without_touching_on`)
- [ ] trait `suprnova::Touchable` · framework/src/eloquent/timestamps.rs:162 (also `suprnova::eloquent::Touchable`, `suprnova::eloquent::timestamps::Touchable`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `rbac::entity::Permission`, `rbac::entity::Role`
  - [ ] fn `suprnova::Touchable::touch` · framework/src/eloquent/timestamps.rs:167 (required)

### `suprnova::eloquent::unique_id`

- [ ] enum `suprnova::eloquent::UniqueIdKind` · framework/src/eloquent/unique_id.rs:32 (also `suprnova::eloquent::unique_id::UniqueIdKind`)
  - Variants: `UuidV7`, `UuidV4`, `Ulid`
  - [ ] fn `suprnova::eloquent::UniqueIdKind::parse` · framework/src/eloquent/unique_id.rs:51
  - [ ] fn `suprnova::eloquent::UniqueIdKind::is_valid` · framework/src/eloquent/unique_id.rs:77
  - [ ] fn `suprnova::eloquent::UniqueIdKind::generate` · framework/src/eloquent/unique_id.rs:85
- [ ] trait `suprnova::eloquent::HasUniqueId` · framework/src/eloquent/unique_id.rs:98 (also `suprnova::eloquent::unique_id::HasUniqueId`)
  - [ ] const `suprnova::eloquent::HasUniqueId::UNIQUE_ID_KIND` · framework/src/eloquent/unique_id.rs:100
  - [ ] fn `suprnova::eloquent::HasUniqueId::new_unique_id` · framework/src/eloquent/unique_id.rs:106 (provided)

### `suprnova::eloquent`

- [ ] trait `suprnova::EloquentModel` · framework/src/eloquent/mod.rs:68 (also `suprnova::eloquent::EloquentModel`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] type `suprnova::EloquentModel::Entity` · framework/src/eloquent/mod.rs:70
  - [ ] type `suprnova::EloquentModel::Column` · framework/src/eloquent/mod.rs:72
  - [ ] type `suprnova::EloquentModel::Key` · framework/src/eloquent/mod.rs:88
  - [ ] const `suprnova::EloquentModel::TABLE` · framework/src/eloquent/mod.rs:90
  - [ ] const `suprnova::EloquentModel::PRIMARY_KEY` · framework/src/eloquent/mod.rs:97
  - [ ] const `suprnova::EloquentModel::SOFT_DELETES_COLUMN` · framework/src/eloquent/mod.rs:104
  - [ ] const `suprnova::EloquentModel::TOUCHES` · framework/src/eloquent/mod.rs:113
  - [ ] const `suprnova::EloquentModel::HAS_TIMESTAMPS` · framework/src/eloquent/mod.rs:125
  - [ ] const `suprnova::EloquentModel::UPDATED_AT_COLUMN` · framework/src/eloquent/mod.rs:130
  - [ ] fn `suprnova::EloquentModel::default_connection_name` · framework/src/eloquent/mod.rs:151 (provided)

### `suprnova`

- [ ] macro `suprnova::attrs` · framework/src/eloquent/attrs.rs:119

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::model` · suprnova-macros/src/lib.rs:847 (re-exported as `suprnova::model`)
  - Form: attribute `#[model]`
  - [ ] argument `table` · suprnova-macros/src/model/parse.rs:761
  - [ ] argument `primary_key` · suprnova-macros/src/model/parse.rs:762
  - [ ] argument `key_type` · suprnova-macros/src/model/parse.rs:763
  - [ ] argument `auto_increment` · suprnova-macros/src/model/parse.rs:772
  - [ ] argument `connection` · suprnova-macros/src/model/parse.rs:773
  - [ ] argument `fillable` · suprnova-macros/src/model/parse.rs:774
  - [ ] argument `guarded` · suprnova-macros/src/model/parse.rs:775
  - [ ] argument `casts` · suprnova-macros/src/model/parse.rs:776
  - [ ] argument `timestamps` · suprnova-macros/src/model/parse.rs:750
  - [ ] argument `created_at` · suprnova-macros/src/model/parse.rs:778
  - [ ] argument `updated_at` · suprnova-macros/src/model/parse.rs:779
  - [ ] argument `soft_deletes` · suprnova-macros/src/model/parse.rs:750
  - [ ] argument `soft_deletes_column` · suprnova-macros/src/model/parse.rs:781
  - [ ] argument `appends` · suprnova-macros/src/model/parse.rs:784
  - [ ] argument `hidden` · suprnova-macros/src/model/parse.rs:785
  - [ ] argument `visible` · suprnova-macros/src/model/parse.rs:786
  - [ ] argument `mutators` · suprnova-macros/src/model/parse.rs:787
  - [ ] argument `touches` · suprnova-macros/src/model/parse.rs:788
  - [ ] argument `relations` · suprnova-macros/src/model/parse.rs:789
  - [ ] argument `morph_type` · suprnova-macros/src/model/parse.rs:790
  - [ ] argument `observers` · suprnova-macros/src/model/parse.rs:811
  - [ ] argument `unique_id = "uuid" | "uuid_v7" | "uuid_v4" | "ulid"` · suprnova-macros/src/model/parse.rs:791
  - [ ] argument relation option `fk` · suprnova-macros/src/model/parse.rs:1135
  - [ ] argument relation option `lk` · suprnova-macros/src/model/parse.rs:1157
  - [ ] argument relation option `with_pivot` · suprnova-macros/src/model/parse.rs:1161
  - [ ] argument relation option `with_timestamps` · suprnova-macros/src/model/parse.rs:1128
  - [ ] argument relation option `with_default` · suprnova-macros/src/model/parse.rs:1171
  - [ ] argument relation option `scope` · suprnova-macros/src/model/parse.rs:1175
  - [ ] argument relation option `name` · suprnova-macros/src/model/parse.rs:1179
  - [ ] argument relation option `morph_name` · suprnova-macros/src/model/parse.rs:1179
  - [ ] argument relation option `targets` · suprnova-macros/src/model/parse.rs:1204
  - [ ] argument relation option `first_key` · suprnova-macros/src/model/parse.rs:1214
  - [ ] argument relation option `second_key` · suprnova-macros/src/model/parse.rs:1218
  - [ ] argument relation option `second_local_key` · suprnova-macros/src/model/parse.rs:1222
  - [ ] argument relation option `pivot_table` · suprnova-macros/src/model/parse.rs:1226
  - [ ] argument relation option `pivot_foreign_key` · suprnova-macros/src/model/parse.rs:1230
  - [ ] argument relation option `pivot_related_key` · suprnova-macros/src/model/parse.rs:1234
  - [ ] argument relation option `related_key` · suprnova-macros/src/model/parse.rs:1238
  - [ ] argument relation option `target_morph_type` · suprnova-macros/src/model/parse.rs:1242
- [ ] proc macro `suprnova_macros::observer` · suprnova-macros/src/lib.rs:1050 (re-exported as `suprnova::observer`)
  - Form: attribute `#[observer]`
  - [ ] argument method `retrieving` · suprnova-macros/src/observer.rs:331
  - [ ] argument method `retrieved` · suprnova-macros/src/observer.rs:333
  - [ ] argument method `created` · suprnova-macros/src/observer.rs:334
  - [ ] argument method `saved` · suprnova-macros/src/observer.rs:335
  - [ ] argument method `updated` · suprnova-macros/src/observer.rs:341
  - [ ] argument method `deleted` · suprnova-macros/src/observer.rs:343
  - [ ] argument method `trashed` · suprnova-macros/src/observer.rs:336
  - [ ] argument method `restored` · suprnova-macros/src/observer.rs:337
  - [ ] argument method `replicating` · suprnova-macros/src/observer.rs:348
  - [ ] argument method `force_deleting` · suprnova-macros/src/observer.rs:338
  - [ ] argument method `force_deleted` · suprnova-macros/src/observer.rs:339
  - [ ] argument cancellable method `creating` · suprnova-macros/src/observer.rs:406
  - [ ] argument cancellable method `saving` · suprnova-macros/src/observer.rs:410
  - [ ] argument cancellable method `updating` · suprnova-macros/src/observer.rs:414
  - [ ] argument cancellable method `deleting` · suprnova-macros/src/observer.rs:420
  - [ ] argument cancellable method `restoring` · suprnova-macros/src/observer.rs:422
- [ ] proc macro `suprnova_macros::prunable` · suprnova-macros/src/lib.rs:967 (re-exported as `suprnova::prunable`)
  - Form: attribute `#[prunable]`
