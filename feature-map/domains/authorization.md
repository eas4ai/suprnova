# Feature map: `manual/authorization.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 376 checked.

## Endpoints and tables

### framework migration `CreateRbacTables`

- [ ] table `roles (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:15
- [ ] table `permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:26
- [ ] table `role_permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:37
- [ ] table `model_roles (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:45
- [ ] table `model_permissions (framework)` · framework/src/rbac/migrations/m_create_rbac_tables.rs:54

## Rust API: suprnova

### `suprnova::authorization::gate` (private module; items are public through re-exports)

- [ ] struct `suprnova::Gate` · framework/src/authorization/gate.rs:30 (also `suprnova::authorization::Gate`)
  - [ ] fn `suprnova::Gate::define` · framework/src/authorization/gate.rs:36
  - [ ] fn `suprnova::Gate::define_with` · framework/src/authorization/gate.rs:62
  - [ ] fn `suprnova::Gate::allows` · framework/src/authorization/gate.rs:76
  - [ ] fn `suprnova::Gate::denies` · framework/src/authorization/gate.rs:81
  - [ ] fn `suprnova::Gate::authorize` · framework/src/authorization/gate.rs:92
  - [ ] fn `suprnova::Gate::define_async` · framework/src/authorization/gate.rs:116
  - [ ] fn `suprnova::Gate::define_async_with` · framework/src/authorization/gate.rs:128
  - [ ] fn `suprnova::Gate::allows_async` · framework/src/authorization/gate.rs:139
  - [ ] fn `suprnova::Gate::denies_async` · framework/src/authorization/gate.rs:148
  - [ ] fn `suprnova::Gate::authorize_async` · framework/src/authorization/gate.rs:157
  - [ ] fn `suprnova::Gate::inspect` · framework/src/authorization/gate.rs:180
  - [ ] fn `suprnova::Gate::inspect_async` · framework/src/authorization/gate.rs:190
  - [ ] fn `suprnova::Gate::raw` · framework/src/authorization/gate.rs:211
  - [ ] fn `suprnova::Gate::raw_async` · framework/src/authorization/gate.rs:219
  - [ ] fn `suprnova::Gate::before` · framework/src/authorization/gate.rs:249
  - [ ] fn `suprnova::Gate::after` · framework/src/authorization/gate.rs:270
  - [ ] fn `suprnova::Gate::default_denial_response` · framework/src/authorization/gate.rs:310
  - [ ] fn `suprnova::Gate::has` · framework/src/authorization/gate.rs:334
  - [ ] fn `suprnova::Gate::abilities` · framework/src/authorization/gate.rs:342
  - [ ] fn `suprnova::Gate::any` · framework/src/authorization/gate.rs:355
  - [ ] fn `suprnova::Gate::none` · framework/src/authorization/gate.rs:363
  - [ ] fn `suprnova::Gate::check` · framework/src/authorization/gate.rs:374
  - [ ] fn `suprnova::Gate::any_async` · framework/src/authorization/gate.rs:385
  - [ ] fn `suprnova::Gate::none_async` · framework/src/authorization/gate.rs:399
  - [ ] fn `suprnova::Gate::check_async` · framework/src/authorization/gate.rs:408

### `suprnova::authorization::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::GateResponse` · framework/src/authorization/response.rs:50 (also `suprnova::authorization::Response`)
  - [ ] fn `suprnova::GateResponse::allow` · framework/src/authorization/response.rs:64
  - [ ] fn `suprnova::GateResponse::deny` · framework/src/authorization/response.rs:76
  - [ ] fn `suprnova::GateResponse::deny_with` · framework/src/authorization/response.rs:87
  - [ ] fn `suprnova::GateResponse::deny_with_status` · framework/src/authorization/response.rs:98
  - [ ] fn `suprnova::GateResponse::deny_as_not_found` · framework/src/authorization/response.rs:110
  - [ ] fn `suprnova::GateResponse::with_message` · framework/src/authorization/response.rs:122
  - [ ] fn `suprnova::GateResponse::with_code` · framework/src/authorization/response.rs:132
  - [ ] fn `suprnova::GateResponse::with_status` · framework/src/authorization/response.rs:138
  - [ ] fn `suprnova::GateResponse::as_not_found` · framework/src/authorization/response.rs:144
  - [ ] fn `suprnova::GateResponse::allowed` · framework/src/authorization/response.rs:152
  - [ ] fn `suprnova::GateResponse::denied` · framework/src/authorization/response.rs:157
  - [ ] fn `suprnova::GateResponse::message` · framework/src/authorization/response.rs:162
  - [ ] fn `suprnova::GateResponse::code` · framework/src/authorization/response.rs:170
  - [ ] fn `suprnova::GateResponse::status` · framework/src/authorization/response.rs:175
  - [ ] fn `suprnova::GateResponse::authorize` · framework/src/authorization/response.rs:191

### `suprnova::authorization`

- [ ] fn `suprnova::authorization::init_policies` · framework/src/authorization/mod.rs:111
- [ ] trait `suprnova::Authorizable` · framework/src/authorization/mod.rs:26 (also `suprnova::authorization::Authorizable`)
  - [ ] fn `suprnova::Authorizable::can` · framework/src/authorization/mod.rs:29 (provided)
  - [ ] fn `suprnova::Authorizable::cannot` · framework/src/authorization/mod.rs:33 (provided)
  - [ ] fn `suprnova::Authorizable::authorize` · framework/src/authorization/mod.rs:43 (provided)
  - [ ] fn `suprnova::Authorizable::can_async` · framework/src/authorization/mod.rs:51 (provided)
  - [ ] fn `suprnova::Authorizable::cannot_async` · framework/src/authorization/mod.rs:63 (provided)
  - [ ] fn `suprnova::Authorizable::authorize_async` · framework/src/authorization/mod.rs:78 (provided)

### `suprnova::rbac::entity::model_permission::events`

- [ ] struct `suprnova::rbac::entity::model_permission::events::Created` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Creating` · framework/src/rbac/entity.rs:66
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Deleted` · framework/src/rbac/entity.rs:66
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Deleting` · framework/src/rbac/entity.rs:66
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::ForceDeleted` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::ForceDeleting` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Replicating` · framework/src/rbac/entity.rs:66
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Restored` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Restoring` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Retrieved` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Retrieving` · framework/src/rbac/entity.rs:66
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Saved` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Saving` · framework/src/rbac/entity.rs:66
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Trashed` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Updated` · framework/src/rbac/entity.rs:66
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Updating` · framework/src/rbac/entity.rs:66
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::model_permission`

- [ ] struct `suprnova::rbac::entity::model_permission::ColumnIter` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::ModelPermissionActiveModel` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::ActiveModel`)
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
- [ ] struct `suprnova::rbac::entity::ModelPermissionEntity` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Entity`)
- [ ] struct `suprnova::rbac::entity::ModelPermissionModel` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Model`)
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
  - [ ] fn `suprnova::rbac::entity::ModelPermissionModel::into_ex` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::model_permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::model_permission::RelationIter` · framework/src/rbac/entity.rs:66
- [ ] enum `suprnova::rbac::entity::ModelPermissionColumn` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Column`)
  - Variants: `Id`, `ModelType`, `ModelId`, `PermissionId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::as_str` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::from_name` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::iter` · framework/src/rbac/entity.rs:66
- [ ] enum `suprnova::rbac::entity::model_permission::PrimaryKey` · framework/src/rbac/entity.rs:66
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::model_permission::Relation` · framework/src/rbac/entity.rs:66

### `suprnova::rbac::entity::model_role::events`

- [ ] struct `suprnova::rbac::entity::model_role::events::Created` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Creating` · framework/src/rbac/entity.rs:53
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Deleted` · framework/src/rbac/entity.rs:53
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Deleting` · framework/src/rbac/entity.rs:53
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::ForceDeleted` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::ForceDeleting` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Replicating` · framework/src/rbac/entity.rs:53
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Restored` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Restoring` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Retrieved` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Retrieving` · framework/src/rbac/entity.rs:53
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Saved` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Saving` · framework/src/rbac/entity.rs:53
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Trashed` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Updated` · framework/src/rbac/entity.rs:53
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Updating` · framework/src/rbac/entity.rs:53
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::model_role`

- [ ] struct `suprnova::rbac::entity::model_role::ColumnIter` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::ModelRoleActiveModel` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::ActiveModel`)
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
- [ ] struct `suprnova::rbac::entity::ModelRoleEntity` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Entity`)
- [ ] struct `suprnova::rbac::entity::ModelRoleModel` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Model`)
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
  - [ ] fn `suprnova::rbac::entity::ModelRoleModel::into_ex` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::model_role::PrimaryKeyIter` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::model_role::RelationIter` · framework/src/rbac/entity.rs:53
- [ ] enum `suprnova::rbac::entity::ModelRoleColumn` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Column`)
  - Variants: `Id`, `ModelType`, `ModelId`, `RoleId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::as_str` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::from_name` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::iter` · framework/src/rbac/entity.rs:53
- [ ] enum `suprnova::rbac::entity::model_role::PrimaryKey` · framework/src/rbac/entity.rs:53
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::model_role::Relation` · framework/src/rbac/entity.rs:53

### `suprnova::rbac::entity::permission::events`

- [ ] struct `suprnova::rbac::entity::permission::events::Created` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Creating` · framework/src/rbac/entity.rs:25
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Deleted` · framework/src/rbac/entity.rs:25
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Deleting` · framework/src/rbac/entity.rs:25
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::ForceDeleted` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::ForceDeleting` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Replicating` · framework/src/rbac/entity.rs:25
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Restored` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Restoring` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Retrieved` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Retrieving` · framework/src/rbac/entity.rs:25
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Saved` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Saving` · framework/src/rbac/entity.rs:25
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Trashed` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Updated` · framework/src/rbac/entity.rs:25
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Updating` · framework/src/rbac/entity.rs:25
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::permission`

- [ ] struct `suprnova::rbac::entity::permission::ColumnIter` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::PermissionActiveModel` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::ActiveModel`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
- [ ] struct `suprnova::rbac::entity::PermissionEntity` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Entity`)
- [ ] struct `suprnova::rbac::entity::PermissionModel` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Model`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - [ ] fn `suprnova::rbac::entity::PermissionModel::into_ex` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::permission::RelationIter` · framework/src/rbac/entity.rs:25
- [ ] enum `suprnova::rbac::entity::PermissionColumn` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Column`)
  - Variants: `Id`, `Name`, `DisplayName`, `GuardName`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::as_str` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::from_name` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::iter` · framework/src/rbac/entity.rs:25
- [ ] enum `suprnova::rbac::entity::permission::PrimaryKey` · framework/src/rbac/entity.rs:25
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::permission::Relation` · framework/src/rbac/entity.rs:25
- [ ] type `suprnova::rbac::entity::permission::__Suprnova_Cast_Storage_created_at` · framework/src/rbac/entity.rs:25
- [ ] type `suprnova::rbac::entity::permission::__Suprnova_Cast_Storage_updated_at` · framework/src/rbac/entity.rs:25

### `suprnova::rbac::entity::role::events`

- [ ] struct `suprnova::rbac::entity::role::events::Created` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Creating` · framework/src/rbac/entity.rs:7
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Deleted` · framework/src/rbac/entity.rs:7
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Deleting` · framework/src/rbac/entity.rs:7
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::ForceDeleted` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::ForceDeleting` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Replicating` · framework/src/rbac/entity.rs:7
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Restored` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Restoring` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Retrieved` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Retrieving` · framework/src/rbac/entity.rs:7
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Saved` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Saving` · framework/src/rbac/entity.rs:7
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Trashed` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Updated` · framework/src/rbac/entity.rs:7
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Updating` · framework/src/rbac/entity.rs:7
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::role_permission::events`

- [ ] struct `suprnova::rbac::entity::role_permission::events::Created` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Creating` · framework/src/rbac/entity.rs:42
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Deleted` · framework/src/rbac/entity.rs:42
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Deleting` · framework/src/rbac/entity.rs:42
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::ForceDeleted` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::ForceDeleting` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Replicating` · framework/src/rbac/entity.rs:42
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Restored` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Restoring` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Retrieved` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Retrieving` · framework/src/rbac/entity.rs:42
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Saved` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Saving` · framework/src/rbac/entity.rs:42
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Trashed` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Updated` · framework/src/rbac/entity.rs:42
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Updating` · framework/src/rbac/entity.rs:42
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::role_permission`

- [ ] struct `suprnova::rbac::entity::role_permission::ColumnIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::role_permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::role_permission::RelationIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::RolePermissionActiveModel` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::ActiveModel`)
  - Public fields: `id`, `role_id`, `permission_id`
- [ ] struct `suprnova::rbac::entity::RolePermissionEntity` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Entity`)
- [ ] struct `suprnova::rbac::entity::RolePermissionModel` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Model`)
  - Public fields: `id`, `role_id`, `permission_id`
  - [ ] fn `suprnova::rbac::entity::RolePermissionModel::into_ex` · framework/src/rbac/entity.rs:42
- [ ] enum `suprnova::rbac::entity::role_permission::PrimaryKey` · framework/src/rbac/entity.rs:42
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::role_permission::Relation` · framework/src/rbac/entity.rs:42
- [ ] enum `suprnova::rbac::entity::RolePermissionColumn` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Column`)
  - Variants: `Id`, `RoleId`, `PermissionId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::as_str` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::from_name` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::iter` · framework/src/rbac/entity.rs:42

### `suprnova::rbac::entity::role`

- [ ] struct `suprnova::rbac::entity::role::ColumnIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::role::PrimaryKeyIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::role::RelationIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::RoleActiveModel` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::ActiveModel`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
- [ ] struct `suprnova::rbac::entity::RoleEntity` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Entity`)
- [ ] struct `suprnova::rbac::entity::RoleModel` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Model`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - [ ] fn `suprnova::rbac::entity::RoleModel::into_ex` · framework/src/rbac/entity.rs:7
- [ ] enum `suprnova::rbac::entity::role::PrimaryKey` · framework/src/rbac/entity.rs:7
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::role::Relation` · framework/src/rbac/entity.rs:7
- [ ] enum `suprnova::rbac::entity::RoleColumn` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Column`)
  - Variants: `Id`, `Name`, `DisplayName`, `GuardName`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::RoleColumn::as_str` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::RoleColumn::from_name` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::RoleColumn::iter` · framework/src/rbac/entity.rs:7
- [ ] type `suprnova::rbac::entity::role::__Suprnova_Cast_Storage_created_at` · framework/src/rbac/entity.rs:7
- [ ] type `suprnova::rbac::entity::role::__Suprnova_Cast_Storage_updated_at` · framework/src/rbac/entity.rs:7

### `suprnova::rbac::entity`

- [ ] struct `suprnova::rbac::entity::ModelPermission` · framework/src/rbac/entity.rs:67
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::ModelPermission::fill` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::without_global_scope` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::without_global_scopes` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::on` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::on_write_connection` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::count` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::sum` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::avg` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::min` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::max` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pluck` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pluck_keyed` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::filter` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::db_where` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::where_in` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::where_like` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::latest` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::oldest` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pivot` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_count` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_sum` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_avg` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_min` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_max` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::observe` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::ModelRole` · framework/src/rbac/entity.rs:54
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::ModelRole::fill` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::without_global_scope` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::without_global_scopes` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::on` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::on_write_connection` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::count` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::sum` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::avg` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::min` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::max` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pluck` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pluck_keyed` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::filter` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::db_where` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::where_in` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::where_like` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::latest` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::oldest` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pivot` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_count` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_sum` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_avg` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_min` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_max` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::observe` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::Permission` · framework/src/rbac/entity.rs:26
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::rbac::entity::Permission::fill` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::without_global_scope` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::without_global_scopes` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::on` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::on_write_connection` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::count` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::sum` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::avg` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::min` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::max` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pluck` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pluck_keyed` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::filter` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::db_where` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::where_in` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::where_like` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::latest` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::oldest` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pivot` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_count` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_sum` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_avg` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_min` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_max` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::observe` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::Role` · framework/src/rbac/entity.rs:8
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::rbac::entity::Role::fill` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::without_global_scope` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::without_global_scopes` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::on` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::on_write_connection` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::count` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::sum` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::avg` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::min` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::max` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pluck` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pluck_keyed` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::filter` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::db_where` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::where_in` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::where_like` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::latest` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::oldest` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pivot` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_count` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_sum` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_avg` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_min` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_max` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::observe` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::RolePermission` · framework/src/rbac/entity.rs:43
  - Public fields: `id`, `role_id`, `permission_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::RolePermission::fill` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::without_global_scope` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::without_global_scopes` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::on` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::on_write_connection` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::count` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::sum` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::avg` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::min` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::max` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pluck` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pluck_keyed` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::filter` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::db_where` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::where_in` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::where_like` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::latest` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::oldest` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pivot` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_count` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_sum` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_avg` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_min` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_max` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::observe` · framework/src/rbac/entity.rs:42

### `suprnova::rbac::has_roles` (private module; items are public through re-exports)

- [ ] fn `suprnova::rbac::assign_role_to_model` · framework/src/rbac/has_roles.rs:417
- [ ] fn `suprnova::rbac::assign_role_to_model_on_guard` · framework/src/rbac/has_roles.rs:429
- [ ] fn `suprnova::rbac::create_permission` · framework/src/rbac/has_roles.rs:304
- [ ] fn `suprnova::rbac::create_permission_on_guard` · framework/src/rbac/has_roles.rs:312
- [ ] fn `suprnova::rbac::create_role` · framework/src/rbac/has_roles.rs:278
- [ ] fn `suprnova::rbac::create_role_on_guard` · framework/src/rbac/has_roles.rs:286
- [ ] fn `suprnova::rbac::give_permission_to_model` · framework/src/rbac/has_roles.rs:510
- [ ] fn `suprnova::rbac::give_permission_to_model_on_guard` · framework/src/rbac/has_roles.rs:521
- [ ] fn `suprnova::rbac::give_permission_to_role` · framework/src/rbac/has_roles.rs:332
- [ ] fn `suprnova::rbac::give_permission_to_role_on_guard` · framework/src/rbac/has_roles.rs:342
- [ ] fn `suprnova::rbac::has_permission_for_model` · framework/src/rbac/has_roles.rs:624
- [ ] fn `suprnova::rbac::has_permission_for_model_on_guard` · framework/src/rbac/has_roles.rs:636
- [ ] fn `suprnova::rbac::has_role_for_model` · framework/src/rbac/has_roles.rs:593
- [ ] fn `suprnova::rbac::has_role_for_model_on_guard` · framework/src/rbac/has_roles.rs:602
- [ ] fn `suprnova::rbac::remove_permission_from_model` · framework/src/rbac/has_roles.rs:560
- [ ] fn `suprnova::rbac::remove_permission_from_model_on_guard` · framework/src/rbac/has_roles.rs:578
- [ ] fn `suprnova::rbac::remove_permission_from_role` · framework/src/rbac/has_roles.rs:380
- [ ] fn `suprnova::rbac::remove_permission_from_role_on_guard` · framework/src/rbac/has_roles.rs:398
- [ ] fn `suprnova::rbac::remove_role_from_model` · framework/src/rbac/has_roles.rs:466
- [ ] fn `suprnova::rbac::remove_role_from_model_on_guard` · framework/src/rbac/has_roles.rs:492
- [ ] trait `suprnova::HasRoles` · framework/src/rbac/has_roles.rs:690 (also `suprnova::rbac::HasRoles`)
  - [ ] fn `suprnova::HasRoles::rbac_model_type` · framework/src/rbac/has_roles.rs:693 (provided)
  - [ ] fn `suprnova::HasRoles::rbac_model_id` · framework/src/rbac/has_roles.rs:699 (provided)
  - [ ] fn `suprnova::HasRoles::assign_role` · framework/src/rbac/has_roles.rs:704 (provided)
  - [ ] fn `suprnova::HasRoles::give_permission_to` · framework/src/rbac/has_roles.rs:709 (provided)
  - [ ] fn `suprnova::HasRoles::remove_role` · framework/src/rbac/has_roles.rs:728 (provided)
  - [ ] fn `suprnova::HasRoles::remove_permission_to` · framework/src/rbac/has_roles.rs:744 (provided)
  - [ ] fn `suprnova::HasRoles::has_role` · framework/src/rbac/has_roles.rs:754 (provided)
  - [ ] fn `suprnova::HasRoles::has_permission_to` · framework/src/rbac/has_roles.rs:761 (provided)

### `suprnova::rbac::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::PermissionMiddleware` · framework/src/rbac/middleware.rs:84 (also `suprnova::rbac::PermissionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::PermissionMiddleware::new` · framework/src/rbac/middleware.rs:92
  - [ ] fn `suprnova::PermissionMiddleware::redirect_to` · framework/src/rbac/middleware.rs:104
- [ ] struct `suprnova::RoleMiddleware` · framework/src/rbac/middleware.rs:29 (also `suprnova::rbac::RoleMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RoleMiddleware::new` · framework/src/rbac/middleware.rs:37
  - [ ] fn `suprnova::RoleMiddleware::redirect_to` · framework/src/rbac/middleware.rs:49

### `suprnova::rbac::migrations::m_create_rbac_tables`

- [ ] struct `suprnova::rbac::migrations::CreateRbacTables` · framework/src/rbac/migrations/m_create_rbac_tables.rs:6 (also `suprnova::rbac::migrations::m_create_rbac_tables::Migration`)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::policy` · suprnova-macros/src/lib.rs:682 (re-exported as `suprnova::policy`)
  - Form: attribute `#[policy]`
