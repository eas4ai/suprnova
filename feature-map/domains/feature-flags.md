# Feature map: `manual/feature-flags.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 116 checked.

## Endpoints and tables

### framework migration `CreateFeaturesTable`

- [ ] table `features (framework)` · framework/src/features/migrations/m_create_features_table.rs:44

## Rust API: suprnova

### Re-exported from other crates

- [ ] trait `suprnova::Evaluator` re-exports `featureflag::evaluator::Evaluator`
- [ ] struct `suprnova::EvaluatorRef` re-exports `featureflag::evaluator::EvaluatorRef`
- [ ] struct `suprnova::Feature` re-exports `featureflag::feature::Feature`
- [ ] macro `suprnova::feature` re-exports `featureflag::feature`
- [ ] module `suprnova::feature` re-exports `featureflag::feature`
- [ ] struct `suprnova::features::Context` re-exports `featureflag::context::Context`
- [ ] trait `suprnova::features::Evaluator` re-exports `featureflag::evaluator::Evaluator`
- [ ] struct `suprnova::features::EvaluatorRef` re-exports `featureflag::evaluator::EvaluatorRef`
- [ ] struct `suprnova::features::Feature` re-exports `featureflag::feature::Feature`
- [ ] function `suprnova::features::set_global_default` re-exports `featureflag::evaluator::global::set_global_default`
- [ ] function `suprnova::features::try_set_global_default` re-exports `featureflag::evaluator::global::try_set_global_default`
- [ ] macro `suprnova::is_enabled` re-exports `featureflag::is_enabled`

### `suprnova::features::admin`

- [ ] fn `suprnova::features::admin::delete` · framework/src/features/admin.rs:183
- [ ] fn `suprnova::features::admin::get` · framework/src/features/admin.rs:90
- [ ] fn `suprnova::features::admin::list` · framework/src/features/admin.rs:77
- [ ] fn `suprnova::features::admin::upsert` · framework/src/features/admin.rs:113
- [ ] struct `suprnova::features::admin::FeatureRow` · framework/src/features/admin.rs:29
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`

### `suprnova::features::bootstrap`

- [ ] fn `suprnova::features::bootstrap_database_cached` · framework/src/features/bootstrap.rs:165 (also `suprnova::features::bootstrap::bootstrap_database_cached`)
- [ ] fn `suprnova::features::install_evaluator` · framework/src/features/bootstrap.rs:95 (also `suprnova::features::bootstrap::install_evaluator`)
- [ ] fn `suprnova::features::is_installed` · framework/src/features/bootstrap.rs:77 (also `suprnova::features::bootstrap::is_installed`)
- [ ] fn `suprnova::features::mark_installed` · framework/src/features/bootstrap.rs:70 (also `suprnova::features::bootstrap::mark_installed`)
- [ ] struct `suprnova::features::BootstrappedFeatures` · framework/src/features/bootstrap.rs:120 (also `suprnova::features::bootstrap::BootstrappedFeatures`)
  - Public fields: `database`, `cached`

### `suprnova::features::entity::feature::events`

- [ ] struct `suprnova::features::entity::feature::events::Created` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Creating` · framework/src/features/entity.rs:28
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Deleted` · framework/src/features/entity.rs:28
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Deleting` · framework/src/features/entity.rs:28
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::ForceDeleted` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::ForceDeleting` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Replicating` · framework/src/features/entity.rs:28
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Restored` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Restoring` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Retrieved` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Retrieving` · framework/src/features/entity.rs:28
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Saved` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Saving` · framework/src/features/entity.rs:28
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Trashed` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Updated` · framework/src/features/entity.rs:28
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Updating` · framework/src/features/entity.rs:28
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::features::entity::feature`

- [ ] struct `suprnova::features::entity::ActiveModel` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::ActiveModel`)
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
- [ ] struct `suprnova::features::entity::feature::ColumnIter` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::Entity` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Entity`)
- [ ] struct `suprnova::features::entity::Model` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Model`)
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
  - [ ] fn `suprnova::features::entity::Model::into_ex` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::feature::PrimaryKeyIter` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::feature::RelationIter` · framework/src/features/entity.rs:28
- [ ] enum `suprnova::features::entity::Column` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Column`)
  - Variants: `Id`, `Name`, `ScopeKey`, `Enabled`, `Description`, `UpdatedBy`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::features::entity::Column::as_str` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Column::from_name` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Column::iter` · framework/src/features/entity.rs:28
- [ ] enum `suprnova::features::entity::feature::PrimaryKey` · framework/src/features/entity.rs:28
  - Variants: `Id`
- [ ] enum `suprnova::features::entity::feature::Relation` · framework/src/features/entity.rs:28
- [ ] type `suprnova::features::entity::feature::__Suprnova_Cast_Storage_created_at` · framework/src/features/entity.rs:28
- [ ] type `suprnova::features::entity::feature::__Suprnova_Cast_Storage_updated_at` · framework/src/features/entity.rs:28

### `suprnova::features::entity`

- [ ] struct `suprnova::features::entity::Feature` · framework/src/features/entity.rs:29
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::features::entity::Feature::fill` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::without_global_scope` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::without_global_scopes` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::on` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::on_write_connection` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::count` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::sum` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::avg` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::min` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::max` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pluck` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pluck_keyed` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::filter` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::db_where` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::where_in` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::where_like` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::latest` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::oldest` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pivot` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_count` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_sum` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_avg` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_min` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_max` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::observe` · framework/src/features/entity.rs:28

### `suprnova::features::evaluators::cached`

- [ ] struct `suprnova::features::CachedEvaluator` · framework/src/features/evaluators/cached.rs:75 (also `suprnova::features::evaluators::cached::CachedEvaluator`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::CachedEvaluator::new` · framework/src/features/evaluators/cached.rs:110
  - [ ] fn `suprnova::features::CachedEvaluator::inner` · framework/src/features/evaluators/cached.rs:121
  - [ ] fn `suprnova::features::CachedEvaluator::invalidate` · framework/src/features/evaluators/cached.rs:129
  - [ ] fn `suprnova::features::CachedEvaluator::invalidate_all` · framework/src/features/evaluators/cached.rs:136
  - [ ] fn `suprnova::features::CachedEvaluator::len` · framework/src/features/evaluators/cached.rs:142
  - [ ] fn `suprnova::features::CachedEvaluator::is_empty` · framework/src/features/evaluators/cached.rs:147

### `suprnova::features::evaluators::database`

- [ ] struct `suprnova::features::DatabaseEvaluator` · framework/src/features/evaluators/database.rs:85 (also `suprnova::features::evaluators::database::DatabaseEvaluator`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::DatabaseEvaluator::new` · framework/src/features/evaluators/database.rs:211
  - [ ] fn `suprnova::features::DatabaseEvaluator::new_in_memory` · framework/src/features/evaluators/database.rs:233
  - [ ] fn `suprnova::features::DatabaseEvaluator::reload` · framework/src/features/evaluators/database.rs:265
  - [ ] fn `suprnova::features::DatabaseEvaluator::set_flag` · framework/src/features/evaluators/database.rs:338

### `suprnova::features::events`

- [ ] struct `suprnova::features::FeatureDeleted` · framework/src/features/events.rs:43 (also `suprnova::features::events::FeatureDeleted`)
  - Public fields: `name`, `scope_key`, `actor_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::FeatureUpdated` · framework/src/features/events.rs:19 (also `suprnova::features::events::FeatureUpdated`)
  - Public fields: `name`, `scope_key`, `enabled`, `actor_id`
  - Implements: `suprnova::Event`

### `suprnova::features::fields`

- [ ] struct `suprnova::features::TeamField` · framework/src/features/fields.rs:109 (also `suprnova::features::fields::TeamField`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::features::TeamField::new` · framework/src/features/fields.rs:113
  - [ ] fn `suprnova::features::TeamField::as_str` · framework/src/features/fields.rs:118
- [ ] struct `suprnova::features::UserIdField` · framework/src/features/fields.rs:71 (also `suprnova::features::fields::UserIdField`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::features::UserIdField::new` · framework/src/features/fields.rs:76
  - [ ] fn `suprnova::features::UserIdField::from_i64` · framework/src/features/fields.rs:82
  - [ ] fn `suprnova::features::UserIdField::as_str` · framework/src/features/fields.rs:87
  - [ ] fn `suprnova::features::UserIdField::as_i64` · framework/src/features/fields.rs:94

### `suprnova::features::middleware`

- [ ] struct `suprnova::features::FeatureMiddleware` · framework/src/features/middleware.rs:87 (also `suprnova::features::middleware::FeatureMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::features::FeatureMiddleware::new` · framework/src/features/middleware.rs:95
  - [ ] fn `suprnova::features::FeatureMiddleware::with_user_id_extractor` · framework/src/features/middleware.rs:105
  - [ ] fn `suprnova::features::FeatureMiddleware::with_team_extractor` · framework/src/features/middleware.rs:116
  - [ ] fn `suprnova::features::FeatureMiddleware::with_team_from_header` · framework/src/features/middleware.rs:130

### `suprnova::features::migrations::m_create_features_table`

- [ ] struct `suprnova::features::migrations::CreateFeaturesTable` · framework/src/features/migrations/m_create_features_table.rs:30 (also `suprnova::features::migrations::m_create_features_table::Migration`)

### `suprnova::features::sync`

- [ ] fn `suprnova::features::sync::notify` · framework/src/features/sync.rs:186
- [ ] fn `suprnova::features::sync::notify_reloaded` · framework/src/features/sync.rs:196
- [ ] struct `suprnova::features::CompositeFeatureSync` · framework/src/features/sync.rs:133 (also `suprnova::features::sync::CompositeFeatureSync`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::CompositeFeatureSync::new` · framework/src/features/sync.rs:146
- [ ] trait `suprnova::features::FeatureSync` · framework/src/features/sync.rs:108 (also `suprnova::features::sync::FeatureSync`)
  - Implemented here by: `features::CachedEvaluator`, `features::CompositeFeatureSync`, `features::DatabaseEvaluator`
  - [ ] fn `suprnova::features::FeatureSync::on_flag_changed` · framework/src/features/sync.rs:114 (required)
  - [ ] fn `suprnova::features::FeatureSync::on_snapshot_reloaded` · framework/src/features/sync.rs:124 (provided)
